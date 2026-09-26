//! Judging whether a service's track is a library track's recording when no identifier says so
//! (yak canon-94cb): the fallback after ISRCs, MusicBrainz's included, have found nothing.
//!
//! The judgement is deliberately strict, since a wrong match plays the wrong audio under the
//! right name, and is kept (as a binding with provenance `fuzzy`), so it is made once. Titles must
//! agree once remaster tags and punctuation are set aside ("Siberian Khatru - 2003 Remaster" is
//! "Siberian Khatru (2025 Remaster)"), but any other qualifier must agree too: a live take, a
//! remix, an edit or an acoustic version is a different recording, and its title says so. The
//! lead artist must be credited on the candidate, and the lengths must be within a few seconds,
//! which also tells apart the album cut and the radio edit that share a title.

use canon_core::SourceTrack;

use crate::model::Track;

/// How far apart two lengths can be for the same recording: services round, and mastering
/// trims silence.
pub const LENGTH_TOLERANCE_MS: u64 = 3_000;

/// How sure the judgement is that `candidate` is `track`'s recording, from `MATCH_FLOOR` to just
/// under 1, or `None` when it isn't. `lead` is the name of the track's first credited artist.
#[must_use]
pub fn judge(track: &Track, lead: &str, candidate: &SourceTrack) -> Option<f32> {
    let (want, have) = (track.duration_ms?, candidate.duration_ms?);
    let apart = want.abs_diff(have);
    if apart > LENGTH_TOLERANCE_MS {
        return None;
    }
    let lead = normalize(lead);
    if lead.is_empty()
        || !candidate
            .artists
            .iter()
            .any(|artist| normalize(&artist.name) == lead)
    {
        return None;
    }
    let (title, other) = (base_title(&track.title), base_title(&candidate.title));
    if title.is_empty() || title != other {
        return None;
    }
    // Lengths within a second, and titles alike even before remaster tags come off, are as sure
    // as a judgement gets.
    #[allow(clippy::cast_precision_loss)]
    let closeness = 1.0 - apart as f32 / LENGTH_TOLERANCE_MS as f32;
    let verbatim = normalize(&track.title) == normalize(&candidate.title);
    Some(MATCH_FLOOR + 0.1 * closeness + if verbatim { 0.05 } else { 0.0 })
}

/// The least confidence a fuzzy match carries: below any identifier-backed binding (1.0).
pub const MATCH_FLOOR: f32 = 0.8;

/// A title with its remaster tags removed, normalized: "Siberian Khatru - 2003 Remaster",
/// "Siberian Khatru (Remastered 2011)" and "siberian khatru" are all "siberian khatru".
fn base_title(title: &str) -> String {
    let mut title = title.to_owned();
    loop {
        let trimmed = strip_remaster(&title);
        if trimmed == title {
            break;
        }
        title = trimmed;
    }
    normalize(&title)
}

/// `title` without one trailing remaster tag: a bracketed "(… Remaster …)" or a dashed
/// "- … Remastered …".
fn strip_remaster(title: &str) -> String {
    let title = title.trim_end();
    let is_remaster = |tag: &str| tag.to_lowercase().contains("remaster");
    if let Some(open) = title.rfind(['(', '[']) {
        let tag = &title[open..];
        if tag.ends_with([')', ']']) && is_remaster(tag) {
            return title[..open].trim_end().to_owned();
        }
    }
    if let Some(dash) = title.rfind(" - ")
        && is_remaster(&title[dash..])
    {
        return title[..dash].trim_end().to_owned();
    }
    title.to_owned()
}

/// Lower case, letters and digits only, single spaces; "&" reads as "and".
fn normalize(text: &str) -> String {
    let text = text.to_lowercase().replace('&', " and ");
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use canon_core::{SourceArtist, SourceRef};

    use super::*;

    fn track(title: &str, duration_ms: u64) -> Track {
        Track {
            title: title.into(),
            credit: "Opeth".into(),
            artists: Vec::new(),
            duration_ms: Some(duration_ms),
            isrcs: Vec::new(),
            mbid: None,
        }
    }

    fn on_tidal(title: &str, artist: &str, duration_ms: u64) -> SourceTrack {
        SourceTrack {
            source: SourceRef::Tidal { id: "1".into() },
            title: title.into(),
            artists: vec![SourceArtist {
                source: None,
                name: artist.into(),
            }],
            album: None,
            disc: None,
            position: None,
            duration_ms: Some(duration_ms),
            isrc: None,
        }
    }

    /// Opeth's "Era" as Spotify and Tidal list it: the same recording under different ISRCs.
    #[test]
    fn the_same_title_artist_and_length_is_the_same_recording() {
        let era = track("Era", 341_613);
        let sure = judge(&era, "Opeth", &on_tidal("Era", "Opeth", 342_000)).unwrap();
        assert!(sure > 0.9, "{sure}");
        assert!(sure < 1.0);
    }

    #[test]
    fn another_take_of_the_song_is_not() {
        let era = track("Era", 341_613);
        let lead = "Opeth";
        assert_eq!(
            judge(&era, lead, &on_tidal("Era (Live)", "Opeth", 342_000)),
            None
        );
        assert_eq!(judge(&era, lead, &on_tidal("Era", "Opeth", 451_000)), None);
        assert_eq!(judge(&era, lead, &on_tidal("Era", "Enigma", 342_000)), None);
        assert_eq!(
            judge(&era, lead, &on_tidal("Era - Radio Edit", "Opeth", 342_000)),
            None
        );
    }

    #[test]
    fn remaster_tags_are_set_aside() {
        let khatru = track("Siberian Khatru - 2003 Remaster", 534_720);
        let sure = judge(
            &khatru,
            "YES",
            &on_tidal("Siberian Khatru (2025 Remaster)", "Yes", 537_000),
        )
        .unwrap();
        assert!((MATCH_FLOOR..0.9).contains(&sure), "{sure}");
        assert_eq!(
            base_title("Money [Remastered 2011] - 2023 Remaster"),
            "money"
        );
    }

    #[test]
    fn without_lengths_there_is_no_judgement() {
        let mut era = track("Era", 0);
        era.duration_ms = None;
        assert_eq!(
            judge(&era, "Opeth", &on_tidal("Era", "Opeth", 342_000)),
            None
        );
    }

    #[test]
    fn punctuation_and_ampersands_do_not_matter() {
        assert_eq!(normalize("Simon & Garfunkel"), "simon and garfunkel");
        assert_eq!(normalize("Will o' the Wisp"), normalize("Will O The Wisp"));
    }
}
