//! Stream resolution: `playbackinfo` → a concrete list of segment URLs (yak canon-4c55).
//!
//! Tidal doesn't hand out a single stream URL; it returns a *manifest* that has to be
//! decoded into the actual media URLs. Two manifest shapes matter:
//!
//! * **BTS** (`application/vnd.tidal.bts`) — a base64 JSON blob with a `urls` list and
//!   the codec/mime. The device-code (TV) client serves Lossless and below this way,
//!   usually as a single full-file URL.
//! * **DASH** (`application/dash+xml`) — a base64 MPD. We do our **own** narrow
//!   SegmentTemplate extraction (init + numbered media segments) rather than feed
//!   Tidal's non-standard MPD to a generic DASH demuxer, which rejects it across tiers
//!   (the reason spelled out on canon-4c55). Hi-res DASH via PKCE is canon-d389.
//!
//! Encrypted manifests are refused outright: canon has no key handling and never will
//! for personal use, so an encrypted asset is an honest [`Error::Unsupported`], not a
//! silent failure.

use base64::Engine;
use canon_core::{Codec, Error, Quality, ReplayGain, Result, StreamInfo};
use serde::Deserialize;

use crate::auth::API_BASE;
use crate::http::TidalHttp;

/// Tidal's playback endpoints are gated on a plausible client version header (and the
/// browser-fingerprint TLS the impersonation client provides); without it they answer
/// `401 subStatus 4005 "Asset is not ready for playback"` even for streamable tracks.
const CLIENT_VERSION: &str = "2025.7.16";

/// The playback gate also checks the User-Agent: it wants the real Tidal Android app's
/// UA, not a browser's. tideway presents exactly this string over its impersonated
/// transport; a Chrome UA over the same TLS still 4005s.
const TIDAL_USER_AGENT: &str = "TIDAL_ANDROID/2.88.0 okhttp/4.12.0";

/// The `playbackinfo` response (the fields canon needs). Tidal ships the EBU-R128
/// loudness tags right here, so ReplayGain is known before a single byte is fetched.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlaybackInfo {
    #[serde(default)]
    audio_quality: Option<String>,
    manifest_mime_type: String,
    /// base64-encoded manifest (BTS JSON or DASH MPD).
    manifest: String,
    #[serde(default)]
    sample_rate: Option<u32>,
    #[serde(default)]
    bit_depth: Option<u8>,
    #[serde(default)]
    track_replay_gain: Option<f32>,
    #[serde(default)]
    track_peak_amplitude: Option<f32>,
    #[serde(default)]
    album_replay_gain: Option<f32>,
    #[serde(default)]
    album_peak_amplitude: Option<f32>,
}

/// The BTS manifest body (base64-decoded JSON).
#[derive(Debug, Clone, Deserialize)]
struct BtsManifest {
    #[serde(rename = "mimeType", default)]
    mime_type: Option<String>,
    #[serde(default)]
    codecs: Option<String>,
    #[serde(rename = "encryptionType", default)]
    encryption_type: Option<String>,
    #[serde(default)]
    urls: Vec<String>,
}

/// A resolved, playable Tidal stream: the ordered URL list plus enough physical
/// description to size the output and drive ReplayGain.
#[derive(Debug, Clone)]
pub struct ResolvedTidalStream {
    /// Segment URLs in play order (index 0 is the init segment for DASH; a BTS list is
    /// concatenated as-is). Fetch and concatenate to reconstruct the encoded stream.
    pub urls: Vec<String>,
    /// The codec, mapped onto canon's vocabulary.
    pub codec: Codec,
    /// A file-extension hint for the demuxer probe (`flac`, `m4a`).
    pub extension_hint: &'static str,
    /// Physical stream description (sample rate/bit depth/replaygain where Tidal
    /// reports them; the decoder fills any gaps).
    pub info: StreamInfo,
    /// The quality Tidal actually served (may be clamped below the request).
    pub served_quality: Option<String>,
}

/// Map the Tidal `audioquality` request enum to its query-param spelling.
fn quality_param(quality: Quality) -> &'static str {
    match quality {
        Quality::Low => "LOW",
        Quality::High => "HIGH",
        Quality::Lossless => "LOSSLESS",
        Quality::HiRes => "HI_RES_LOSSLESS",
    }
}

fn map_codec(codecs: Option<&str>, mime: Option<&str>) -> (Codec, &'static str) {
    let token = codecs.or(mime).unwrap_or("").to_ascii_lowercase();
    if token.contains("flac") {
        (Codec::Flac, "flac")
    } else if token.contains("mp4a") || token.contains("aac") {
        (Codec::Aac, "m4a")
    } else if token.contains("alac") {
        (Codec::Alac, "m4a")
    } else {
        // Unknown: default the probe hint to mp4, since non-FLAC Tidal audio is fMP4.
        (Codec::Other, "m4a")
    }
}

fn replaygain(info: &PlaybackInfo) -> Option<ReplayGain> {
    let track_gain_db = info.track_replay_gain?;
    Some(ReplayGain {
        track_gain_db,
        track_peak: info.track_peak_amplitude.unwrap_or(1.0),
        album_gain_db: info.album_replay_gain,
        album_peak: info.album_peak_amplitude,
    })
}

/// Resolve a track id to its playable stream via `playbackinfo`.
///
/// `bearer` is a valid access token; `country` is the account's country code (Tidal
/// requires it on the request). All HTTP goes through the impersonation seam, which is
/// mandatory here — `playbackinfo` is fingerprint-gated where softer endpoints are not.
pub async fn resolve(
    http: &dyn TidalHttp,
    bearer: &str,
    track_id: &str,
    quality: Quality,
    country: &str,
    session_id: &str,
) -> Result<ResolvedTidalStream> {
    // The sessionId query param (from /v1/sessions) is mandatory on playback endpoints;
    // omitting it is the other half of the 4005 gate, alongside the fingerprint + client
    // version. Match tidalapi's request exactly.
    let url = format!(
        "{API_BASE}/v1/tracks/{track_id}/playbackinfopostpaywall\
         ?audioquality={q}&playbackmode=STREAM&assetpresentation=FULL\
         &countryCode={country}&sessionId={session_id}",
        q = quality_param(quality),
    );
    let authorization = format!("Bearer {bearer}");
    let resp = http
        .get(
            &url,
            &[
                ("Authorization", &authorization),
                ("x-tidal-client-version", CLIENT_VERSION),
                ("User-Agent", TIDAL_USER_AGENT),
            ],
        )
        .await?;

    if resp.status == 401 || resp.status == 403 {
        return Err(Error::Auth(format!(
            "playbackinfo rejected: HTTP {} {}",
            resp.status,
            resp.text().unwrap_or_default()
        )));
    }
    if resp.status == 404 {
        return Err(Error::NotFound(format!("track {track_id} has no stream")));
    }
    if !resp.is_success() {
        return Err(Error::Source(format!(
            "playbackinfo failed: HTTP {} {}",
            resp.status,
            resp.text().unwrap_or_default()
        )));
    }

    let info: PlaybackInfo = resp.json()?;
    parse_manifest(&info)
}

/// Decode + dispatch the manifest by its MIME type.
fn parse_manifest(info: &PlaybackInfo) -> Result<ResolvedTidalStream> {
    let raw = base64::engine::general_purpose::STANDARD
        .decode(info.manifest.as_bytes())
        .map_err(|e| Error::Source(format!("manifest base64 decode: {e}")))?;

    let stream_info = |codec: Codec| StreamInfo {
        codec,
        sample_rate: info.sample_rate.unwrap_or(0),
        bit_depth: info.bit_depth,
        channels: 2,
        replaygain: replaygain(info),
    };

    match info.manifest_mime_type.as_str() {
        "application/vnd.tidal.bts" => {
            let bts: BtsManifest = serde_json::from_slice(&raw)
                .map_err(|e| Error::Source(format!("bts manifest decode: {e}")))?;
            reject_if_encrypted(bts.encryption_type.as_deref())?;
            if bts.urls.is_empty() {
                return Err(Error::Source("bts manifest had no urls".into()));
            }
            let (codec, ext) = map_codec(bts.codecs.as_deref(), bts.mime_type.as_deref());
            Ok(ResolvedTidalStream {
                urls: bts.urls,
                codec,
                extension_hint: ext,
                info: stream_info(codec),
                served_quality: info.audio_quality.clone(),
            })
        }
        "application/dash+xml" => {
            let mpd = std::str::from_utf8(&raw)
                .map_err(|e| Error::Source(format!("dash manifest utf8: {e}")))?;
            let dash = parse_dash(mpd)?;
            Ok(ResolvedTidalStream {
                urls: dash.urls,
                codec: dash.codec,
                extension_hint: dash.extension_hint,
                info: stream_info(dash.codec),
                served_quality: info.audio_quality.clone(),
            })
        }
        other => Err(Error::Unsupported(format!("manifest mime {other}"))),
    }
}

fn reject_if_encrypted(encryption_type: Option<&str>) -> Result<()> {
    match encryption_type {
        None | Some("NONE") => Ok(()),
        Some(other) => Err(Error::Unsupported(format!(
            "encrypted manifest (encryptionType={other})"
        ))),
    }
}

struct DashStream {
    urls: Vec<String>,
    codec: Codec,
    extension_hint: &'static str,
}

/// Narrow SegmentTemplate extraction for Tidal's MPD: pull the codec, the
/// initialization URL, and the numbered media URLs implied by the SegmentTimeline.
///
/// Deliberately small and Tidal-shaped rather than a general DASH parser. Rejects a
/// manifest carrying ContentProtection (encrypted) before emitting any URL.
fn parse_dash(mpd: &str) -> Result<DashStream> {
    use quick_xml::events::Event;
    use quick_xml::reader::Reader;

    if mpd.contains("ContentProtection") {
        return Err(Error::Unsupported("encrypted DASH manifest".into()));
    }

    let mut reader = Reader::from_str(mpd);
    reader.config_mut().trim_text(true);

    let mut codecs: Option<String> = None;
    let mut mime: Option<String> = None;
    let mut initialization: Option<String> = None;
    let mut media: Option<String> = None;
    let mut start_number: u64 = 1;
    let mut segment_count: u64 = 0; // total media segments from the timeline

    let attr = |e: &quick_xml::events::BytesStart, key: &str| -> Option<String> {
        e.attributes().flatten().find_map(|a| {
            if a.key.as_ref() == key.as_bytes() {
                Some(String::from_utf8_lossy(&a.value).into_owned())
            } else {
                None
            }
        })
    };

    let mut buf = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| Error::Source(format!("dash mpd parse: {e}")))?
        {
            Event::Eof => break,
            Event::Start(e) | Event::Empty(e) => match e.name().as_ref() {
                b"Representation" => {
                    codecs = attr(&e, "codecs").or(codecs.take());
                }
                b"AdaptationSet" => {
                    mime = attr(&e, "mimeType").or(mime.take());
                }
                b"SegmentTemplate" => {
                    initialization = attr(&e, "initialization");
                    media = attr(&e, "media");
                    if let Some(n) = attr(&e, "startNumber").and_then(|s| s.parse().ok()) {
                        start_number = n;
                    }
                }
                b"S" => {
                    // A timeline entry covers 1 + @r additional segments.
                    let repeat: u64 = attr(&e, "r")
                        .and_then(|s| s.parse::<i64>().ok())
                        .map(|r| r.max(0) as u64)
                        .unwrap_or(0);
                    segment_count += 1 + repeat;
                }
                _ => {}
            },
            _ => {}
        }
        buf.clear();
    }

    let media = media.ok_or_else(|| Error::Source("dash manifest had no media template".into()))?;
    let (codec, ext) = map_codec(codecs.as_deref(), mime.as_deref());

    let mut urls = Vec::with_capacity(segment_count as usize + 1);
    if let Some(init) = initialization {
        urls.push(init);
    }
    // Expand $Number$ over the timeline. Tidal templates use a bare $Number$.
    for n in 0..segment_count {
        let number = start_number + n;
        urls.push(media.replace("$Number$", &number.to_string()));
    }
    if urls.is_empty() {
        return Err(Error::Source("dash manifest yielded no urls".into()));
    }

    Ok(DashStream {
        urls,
        codec,
        extension_hint: ext,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bts_playbackinfo(manifest_json: &str) -> PlaybackInfo {
        let b64 = base64::engine::general_purpose::STANDARD.encode(manifest_json.as_bytes());
        PlaybackInfo {
            audio_quality: Some("LOSSLESS".into()),
            manifest_mime_type: "application/vnd.tidal.bts".into(),
            manifest: b64,
            sample_rate: Some(44_100),
            bit_depth: Some(16),
            track_replay_gain: Some(-7.5),
            track_peak_amplitude: Some(0.98),
            album_replay_gain: Some(-8.0),
            album_peak_amplitude: Some(0.99),
        }
    }

    #[test]
    fn parses_bts_single_url_flac() {
        let info = bts_playbackinfo(
            r#"{"mimeType":"audio/flac","codecs":"flac","encryptionType":"NONE","urls":["https://cdn.tidal/x.flac"]}"#,
        );
        let resolved = parse_manifest(&info).unwrap();
        assert_eq!(resolved.urls, ["https://cdn.tidal/x.flac"]);
        assert_eq!(resolved.codec, Codec::Flac);
        assert_eq!(resolved.extension_hint, "flac");
        assert_eq!(resolved.info.sample_rate, 44_100);
        let rg = resolved.info.replaygain.unwrap();
        assert!((rg.track_gain_db - -7.5).abs() < 1e-6);
    }

    #[test]
    fn rejects_encrypted_bts() {
        let info = bts_playbackinfo(
            r#"{"mimeType":"audio/mp4","codecs":"mp4a.40.2","encryptionType":"OLD","urls":["https://x"]}"#,
        );
        assert!(matches!(parse_manifest(&info), Err(Error::Unsupported(_))));
    }

    #[test]
    fn parses_dash_segment_template() {
        let mpd = r#"<?xml version="1.0"?>
        <MPD><Period><AdaptationSet mimeType="audio/mp4">
          <Representation codecs="flac">
            <SegmentTemplate initialization="https://cdn/init.mp4" media="https://cdn/seg_$Number$.mp4" startNumber="1">
              <SegmentTimeline><S d="100" r="2"/><S d="50"/></SegmentTimeline>
            </SegmentTemplate>
          </Representation>
        </AdaptationSet></Period></MPD>"#;
        let info = PlaybackInfo {
            audio_quality: Some("HI_RES_LOSSLESS".into()),
            manifest_mime_type: "application/dash+xml".into(),
            manifest: base64::engine::general_purpose::STANDARD.encode(mpd.as_bytes()),
            sample_rate: Some(96_000),
            bit_depth: Some(24),
            track_replay_gain: None,
            track_peak_amplitude: None,
            album_replay_gain: None,
            album_peak_amplitude: None,
        };
        let resolved = parse_manifest(&info).unwrap();
        // init + (3 + 1) media segments from the timeline.
        assert_eq!(resolved.urls.len(), 5);
        assert_eq!(resolved.urls[0], "https://cdn/init.mp4");
        assert_eq!(resolved.urls[1], "https://cdn/seg_1.mp4");
        assert_eq!(resolved.urls[4], "https://cdn/seg_4.mp4");
        assert_eq!(resolved.codec, Codec::Flac);
    }

    #[test]
    fn rejects_encrypted_dash() {
        let mpd = r#"<MPD><Period><AdaptationSet><ContentProtection schemeIdUri="x"/></AdaptationSet></Period></MPD>"#;
        let info = PlaybackInfo {
            audio_quality: None,
            manifest_mime_type: "application/dash+xml".into(),
            manifest: base64::engine::general_purpose::STANDARD.encode(mpd.as_bytes()),
            sample_rate: None,
            bit_depth: None,
            track_replay_gain: None,
            track_peak_amplitude: None,
            album_replay_gain: None,
            album_peak_amplitude: None,
        };
        assert!(matches!(parse_manifest(&info), Err(Error::Unsupported(_))));
    }
}
