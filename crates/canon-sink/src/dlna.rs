//! DLNA/UPnP renderer sink: AVTransport control and polled state feedback (yak canon-685a).
//!
//! The second protocol behind [`Sink`], and deliberately the adversarial one: where Cast is one
//! uniform spec with pushed status, DLNA is a device zoo with patchy eventing and coarse position.
//! Everything protocol-neutral — the stream server, the FLAC tap, liveness by consumption, the
//! renderer clock, the edge filter, attribution of reports to loads — is reused unchanged; this
//! module only speaks SOAP and *classifies* what the device says.
//!
//! ## State is polled
//!
//! GENA eventing (`LastChange`) is the spec's push channel, but it is exactly where the zoo is
//! least reliable: renderers that never send, subscriptions that silently lapse, and a callback
//! server that has to be reachable on the right interface. A poll of `GetTransportInfo` and
//! `GetPositionInfo` every [`POLL_INTERVAL`] works on everything and is what position needs
//! anyway (`LastChange` does not carry `RelTime`). Eventing can be layered on later to cut state
//! latency; it can never replace the poll.
//!
//! ## What the device's answers mean
//!
//! * `TrackURI` is the one takeover signal DLNA gives: a renderer playing a URI that isn't ours has
//!   been handed something else by another controller — the analogue of a foreign Cast media
//!   session.
//! * `STOPPED` after our media has played is the track ending (our stream's body ended; see
//!   [`crate::stream_server`]). `STOPPED` *before* it has played is a renderer between
//!   `SetAVTransportURI` and `Play`, and means nothing.
//! * `RelTime` is relative to the media we handed it, and whole seconds on most devices. Both are
//!   already the renderer clock's business (its origin, and `SLEW_BEHIND`); nothing is corrected
//!   here.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use canon_core::{
    Error, LoadId, RendererEvent, RendererReport, RendererState, Result, Sink, SinkId, SinkKind,
    TrackMeta,
};
use rupnp::http::Uri;
use tokio::sync::mpsc;

use crate::discovery::AV_TRANSPORT;
use crate::renderer::{EdgeFilter, RendererEvents};

/// RenderingControl, matched by type prefix for the same reason as AVTransport.
const RENDERING_CONTROL: &str = "urn:schemas-upnp-org:service:RenderingControl:";

/// How often transport state and position are polled.
const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Consecutive failed polls before the renderer is declared gone. A single timeout is a busy
/// device; seconds of them is a device that has left.
const POLL_FAILURES: u32 = 6;

/// The protocolInfo our stream is announced with. `audio/flac` over HTTP is what renderers list
/// in `GetProtocolInfo` (the LS50 Wireless II does: `http-get:*:audio/flac:*`).
const PROTOCOL_INFO: &str = "http-get:*:audio/flac:*";

#[derive(Debug)]
enum DlnaCommand {
    Load {
        url: String,
        meta: TrackMeta,
        load: LoadId,
    },
    Play,
    Pause,
    Stop,
    SetVolume(f32),
    SetMuted(bool),
}

/// A DLNA MediaRenderer, driven from its own task.
///
/// Dropping it closes the command channel; the task then stops our media and exits, closing the
/// event stream.
pub struct DlnaSink {
    id: SinkId,
    commands: mpsc::UnboundedSender<DlnaCommand>,
    loads: AtomicU64,
}

impl DlnaSink {
    /// Read the renderer's description at `location` and start driving it.
    ///
    /// # Errors
    /// [`Error::Sink`] if the description can't be read, or the device has no AVTransport.
    pub async fn connect(id: SinkId, location: &str) -> Result<(Self, RendererEvents)> {
        let url: Uri = location
            .parse()
            .map_err(|e| Error::Sink(format!("dlna location {location}: {e}")))?;
        let device = rupnp::Device::from_url(url)
            .await
            .map_err(|e| Error::Sink(format!("dlna description {location}: {e}")))?;
        let find = |prefix: &str| {
            device
                .services_iter()
                .find(|service| service.service_type().to_string().starts_with(prefix))
                .cloned()
        };
        let transport = find(AV_TRANSPORT)
            .ok_or_else(|| Error::Sink(format!("{} has no AVTransport", device.friendly_name())))?;
        let rendering = find(RENDERING_CONTROL);
        let renderer = Renderer {
            url: device.url().clone(),
            transport,
            rendering,
        };

        let (commands, command_rx) = mpsc::unbounded_channel();
        let (events, event_rx) = mpsc::unbounded_channel();
        tokio::spawn(run(renderer, command_rx, events));
        Ok((
            Self {
                id,
                commands,
                loads: AtomicU64::new(0),
            },
            event_rx,
        ))
    }

    fn send(&self, command: DlnaCommand) -> Result<()> {
        self.commands
            .send(command)
            .map_err(|_| Error::Sink("dlna renderer session is gone".to_string()))
    }
}

impl Sink for DlnaSink {
    fn id(&self) -> SinkId {
        self.id.clone()
    }
    fn kind(&self) -> SinkKind {
        SinkKind::Dlna
    }
    fn load(&self, url: &str, meta: &TrackMeta) -> Result<LoadId> {
        let load = LoadId(self.loads.fetch_add(1, Ordering::Relaxed) + 1);
        self.send(DlnaCommand::Load {
            url: url.to_string(),
            meta: meta.clone(),
            load,
        })?;
        Ok(load)
    }
    fn play(&self) -> Result<()> {
        self.send(DlnaCommand::Play)
    }
    fn pause(&self) -> Result<()> {
        self.send(DlnaCommand::Pause)
    }
    fn stop(&self) -> Result<()> {
        self.send(DlnaCommand::Stop)
    }
    fn set_volume(&self, volume: f32) -> Result<()> {
        self.send(DlnaCommand::SetVolume(volume.clamp(0.0, 1.0)))
    }
    fn set_muted(&self, muted: bool) -> Result<()> {
        self.send(DlnaCommand::SetMuted(muted))
    }
}

/// The services we drive on one device.
struct Renderer {
    url: Uri,
    transport: rupnp::Service,
    rendering: Option<rupnp::Service>,
}

impl Renderer {
    async fn transport(&self, action: &str, args: &str) -> Result<Vec<(String, String)>> {
        let payload = format!("<InstanceID>0</InstanceID>{args}");
        self.transport
            .action(&self.url, action, &payload)
            .await
            .map(|reply| reply.into_iter().collect())
            .map_err(|e| Error::Sink(format!("dlna {action}: {e}")))
    }

    async fn rendering(&self, action: &str, args: &str) -> Result<()> {
        let Some(rendering) = &self.rendering else {
            return Err(Error::Unsupported(
                "renderer has no RenderingControl".into(),
            ));
        };
        let payload = format!("<InstanceID>0</InstanceID><Channel>Master</Channel>{args}");
        rendering
            .action(&self.url, action, &payload)
            .await
            .map(|_| ())
            .map_err(|e| Error::Sink(format!("dlna {action}: {e}")))
    }
}

/// What this session has loaded on the renderer.
struct Loaded {
    load: LoadId,
    url: String,
    /// Whether the renderer has been seen playing this media — before that, `STOPPED` is a
    /// renderer that hasn't started yet, not one that has finished.
    played: bool,
}

async fn run(
    renderer: Renderer,
    mut commands: mpsc::UnboundedReceiver<DlnaCommand>,
    events: mpsc::UnboundedSender<RendererReport>,
) {
    let mut poll = tokio::time::interval(POLL_INTERVAL);
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut loaded: Option<Loaded> = None;
    let mut edges = EdgeFilter::default();
    let mut failures = 0u32;
    let report = |load: LoadId, event: RendererEvent| {
        let _ = events.send(RendererReport { load, event });
    };

    loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else {
                    // The sink was dropped: leave the renderer silent, not playing out our stream.
                    if loaded.is_some() {
                        let _ = renderer.transport("Stop", "").await;
                    }
                    return;
                };
                match command {
                    DlnaCommand::Load { url, meta, load } => {
                        match load_and_play(&renderer, &url, &meta).await {
                            Ok(()) => {
                                loaded = Some(Loaded { load, url, played: false });
                                edges.reset();
                            }
                            // A renderer that refuses our stream is not a usable output.
                            Err(e) => report(load, RendererEvent::Failed(e.to_string())),
                        }
                    }
                    other => {
                        // A refused transport or volume command is not the session failing: a
                        // renderer mid-transition commonly rejects a Pause (UPnP error 701), and
                        // what it is actually doing arrives on the next poll either way.
                        if let Err(e) = command_once(&renderer, other).await {
                            tracing::warn!("dlna command refused: {e}");
                        }
                    }
                }
            }
            _ = poll.tick() => {
                let Some(current) = loaded.as_mut() else { continue };
                match poll_once(&renderer).await {
                    Ok(status) => {
                        tracing::trace!(?status, "dlna status poll");
                        failures = 0;
                        if status.state == "PLAYING" {
                            current.played = true;
                        }
                        if let Some(event) = classify(&status, &current.url, current.played)
                            && edges.admit(&event)
                        {
                            report(current.load, event);
                        }
                        if let Some(position) = reported_position(&status) {
                            report(current.load, RendererEvent::Position(position));
                        }
                    }
                    Err(e) => {
                        failures += 1;
                        if failures >= POLL_FAILURES {
                            report(current.load, RendererEvent::Failed(format!("dlna poll: {e}")));
                            return;
                        }
                    }
                }
            }
        }
    }
}

async fn load_and_play(renderer: &Renderer, url: &str, meta: &TrackMeta) -> Result<()> {
    let args = format!(
        "<CurrentURI>{}</CurrentURI><CurrentURIMetaData>{}</CurrentURIMetaData>",
        xml_escape(url),
        xml_escape(&didl(url, meta)),
    );
    renderer.transport("SetAVTransportURI", &args).await?;
    renderer.transport("Play", "<Speed>1</Speed>").await?;
    Ok(())
}

async fn command_once(renderer: &Renderer, command: DlnaCommand) -> Result<()> {
    match command {
        DlnaCommand::Play => renderer
            .transport("Play", "<Speed>1</Speed>")
            .await
            .map(|_| ()),
        DlnaCommand::Pause => renderer.transport("Pause", "").await.map(|_| ()),
        DlnaCommand::Stop => renderer.transport("Stop", "").await.map(|_| ()),
        DlnaCommand::SetVolume(volume) => {
            // RenderingControl's Master volume is 0–100 on every renderer seen so far.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let level = (volume * 100.0).round() as u32;
            renderer
                .rendering(
                    "SetVolume",
                    &format!("<DesiredVolume>{level}</DesiredVolume>"),
                )
                .await
        }
        DlnaCommand::SetMuted(muted) => {
            let flag = u8::from(muted);
            renderer
                .rendering("SetMute", &format!("<DesiredMute>{flag}</DesiredMute>"))
                .await
        }
        DlnaCommand::Load { .. } => unreachable!("loads are handled by the caller"),
    }
}

/// One poll's worth of what the renderer says.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Status {
    /// `CurrentTransportState`.
    state: String,
    /// `TrackURI`, if the renderer reported one.
    track_uri: Option<String>,
    /// `RelTime`, as reported.
    rel_time: Option<String>,
}

async fn poll_once(renderer: &Renderer) -> Result<Status> {
    let transport = renderer.transport("GetTransportInfo", "").await?;
    let position = renderer.transport("GetPositionInfo", "").await?;
    let field = |fields: &[(String, String)], name: &str| {
        fields
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
            .filter(|value| !value.is_empty())
    };
    Ok(Status {
        state: field(&transport, "CurrentTransportState").unwrap_or_default(),
        track_uri: field(&position, "TrackURI"),
        rel_time: field(&position, "RelTime"),
    })
}

/// Classify one poll, given the URL of the media we loaded and whether it has played yet.
fn classify(status: &Status, ours: &str, played: bool) -> Option<RendererEvent> {
    // Another controller handed the renderer something else. (An empty or missing TrackURI says
    // nothing either way: many renderers clear it when they stop.)
    if let Some(uri) = &status.track_uri
        && uri != ours
    {
        return Some(RendererEvent::Superseded(format!(
            "the renderer is playing {uri}, not our stream"
        )));
    }
    match status.state.as_str() {
        "PLAYING" => Some(RendererEvent::State(RendererState::Playing)),
        "PAUSED_PLAYBACK" | "PAUSED_RECORDING" => Some(RendererEvent::State(RendererState::Paused)),
        "TRANSITIONING" => Some(RendererEvent::State(RendererState::Buffering)),
        // It played our media and has stopped: the stream ended, and the track with it. (A stop
        // we asked for is also this, and the controller has already forgotten that load.)
        "STOPPED" | "NO_MEDIA_PRESENT" if played => Some(RendererEvent::Ended),
        _ => None,
    }
}

/// The position a poll reports, if it is one we should believe: only while playing, and only when
/// the renderer actually implements `RelTime`.
fn reported_position(status: &Status) -> Option<Duration> {
    if status.state != "PLAYING" {
        return None;
    }
    parse_duration(status.rel_time.as_deref()?)
}

/// UPnP's `H+:MM:SS[.F+]` (or `.F0/F1` fraction) duration; `None` for `NOT_IMPLEMENTED` and the like.
fn parse_duration(text: &str) -> Option<Duration> {
    let mut parts = text.trim().splitn(3, ':');
    let hours: u64 = parts.next()?.parse().ok()?;
    let minutes: u64 = parts.next()?.parse().ok()?;
    let seconds = parts.next()?;
    let (whole, fraction) = seconds.split_once('.').unwrap_or((seconds, ""));
    let whole: u64 = whole.parse().ok()?;
    let millis = match fraction.split_once('/') {
        Some((num, den)) => {
            let (num, den): (u64, u64) = (num.parse().ok()?, den.parse().ok()?);
            (num * 1000).checked_div(den).unwrap_or(0)
        }
        None if fraction.is_empty() => 0,
        None => {
            let digits: String = fraction.chars().take(3).collect();
            let scale = 10u64.pow(3 - u32::try_from(digits.len()).ok()?);
            digits.parse::<u64>().ok()? * scale
        }
    };
    Some(Duration::from_millis(
        ((hours * 60 + minutes) * 60 + whole) * 1000 + millis,
    ))
}

/// DIDL-Lite metadata describing our stream, so the renderer shows what is playing and knows the
/// format before it fetches.
fn didl(url: &str, meta: &TrackMeta) -> String {
    let mut item = format!("<dc:title>{}</dc:title>", xml_escape(&meta.title));
    for artist in &meta.artists {
        item.push_str(&format!(
            "<upnp:artist>{}</upnp:artist>",
            xml_escape(artist)
        ));
    }
    if let Some(album) = &meta.album {
        item.push_str(&format!("<upnp:album>{}</upnp:album>", xml_escape(album)));
    }
    if let Some(art) = &meta.artwork_url {
        item.push_str(&format!(
            "<upnp:albumArtURI>{}</upnp:albumArtURI>",
            xml_escape(art)
        ));
    }
    format!(
        "<DIDL-Lite xmlns=\"urn:schemas-upnp-org:metadata-1-0/DIDL-Lite/\" \
         xmlns:dc=\"http://purl.org/dc/elements/1.1/\" \
         xmlns:upnp=\"urn:schemas-upnp-org:metadata-1-0/upnp/\">\
         <item id=\"0\" parentID=\"-1\" restricted=\"1\">{item}\
         <upnp:class>object.item.audioItem.musicTrack</upnp:class>\
         <res protocolInfo=\"{PROTOCOL_INFO}\">{}</res></item></DIDL-Lite>",
        xml_escape(url)
    )
}

fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const OURS: &str = "http://192.168.0.67:5000/stream/3.flac";

    fn status(state: &str, uri: Option<&str>, rel: Option<&str>) -> Status {
        Status {
            state: state.to_string(),
            track_uri: uri.map(str::to_string),
            rel_time: rel.map(str::to_string),
        }
    }

    #[test]
    fn transport_states_map_to_renderer_states() {
        let cases = [
            ("PLAYING", RendererState::Playing),
            ("PAUSED_PLAYBACK", RendererState::Paused),
            ("TRANSITIONING", RendererState::Buffering),
        ];
        for (state, expected) in cases {
            assert_eq!(
                classify(&status(state, Some(OURS), None), OURS, true),
                Some(RendererEvent::State(expected)),
                "{state}"
            );
        }
    }

    /// Stopped after playing is the end of the track: our stream's body ended.
    #[test]
    fn stopped_after_playing_is_the_end() {
        assert_eq!(
            classify(&status("STOPPED", Some(OURS), None), OURS, true),
            Some(RendererEvent::Ended)
        );
        assert_eq!(
            classify(&status("NO_MEDIA_PRESENT", None, None), OURS, true),
            Some(RendererEvent::Ended)
        );
    }

    /// Between SetAVTransportURI and Play a renderer sits STOPPED; that is not a finished track.
    #[test]
    fn stopped_before_playing_means_nothing() {
        assert_eq!(
            classify(&status("STOPPED", Some(OURS), None), OURS, false),
            None
        );
    }

    /// DLNA's takeover signal: the renderer is playing a URI that isn't ours.
    #[test]
    fn a_foreign_track_uri_is_a_takeover() {
        let event = classify(
            &status("PLAYING", Some("http://phone/song.mp3"), None),
            OURS,
            true,
        );
        assert!(
            matches!(event, Some(RendererEvent::Superseded(_))),
            "{event:?}"
        );
    }

    #[test]
    fn a_missing_track_uri_is_not_a_takeover() {
        assert_eq!(
            classify(&status("PLAYING", None, None), OURS, true),
            Some(RendererEvent::State(RendererState::Playing))
        );
    }

    #[test]
    fn position_is_believed_only_while_playing() {
        assert_eq!(
            reported_position(&status("PLAYING", Some(OURS), Some("0:01:05"))),
            Some(Duration::from_secs(65))
        );
        assert_eq!(
            reported_position(&status("PAUSED_PLAYBACK", Some(OURS), Some("0:01:05"))),
            None
        );
        assert_eq!(
            reported_position(&status("PLAYING", Some(OURS), Some("NOT_IMPLEMENTED"))),
            None
        );
    }

    #[test]
    fn upnp_durations_parse_in_every_shape_seen() {
        assert_eq!(parse_duration("0:00:05"), Some(Duration::from_secs(5)));
        assert_eq!(parse_duration("00:03:54"), Some(Duration::from_secs(234)));
        assert_eq!(
            parse_duration("1:02:03.5"),
            Some(Duration::from_millis(3_723_500))
        );
        assert_eq!(
            parse_duration("0:00:01.250"),
            Some(Duration::from_millis(1_250))
        );
        assert_eq!(
            parse_duration("0:00:02.1/4"),
            Some(Duration::from_millis(2_250))
        );
        assert_eq!(parse_duration("NOT_IMPLEMENTED"), None);
        assert_eq!(parse_duration(""), None);
    }

    #[test]
    fn metadata_is_escaped_and_announces_flac() {
        let meta = TrackMeta {
            title: "Rock & <Roll>".to_string(),
            artists: vec!["A \"B\"".to_string()],
            ..TrackMeta::default()
        };
        let didl = didl("http://h/stream/1.flac?a=1&b=2", &meta);
        assert!(didl.contains("<dc:title>Rock &amp; &lt;Roll&gt;</dc:title>"));
        assert!(didl.contains("<upnp:artist>A &quot;B&quot;</upnp:artist>"));
        assert!(didl.contains("protocolInfo=\"http-get:*:audio/flac:*\""));
        assert!(didl.contains(">http://h/stream/1.flac?a=1&amp;b=2</res>"));
    }
}
