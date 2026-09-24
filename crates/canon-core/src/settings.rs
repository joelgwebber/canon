//! User settings: one schema for the file, the API, and (later) generated client types (yak
//! canon-f04a).
//!
//! [`Settings`] is *the* schema. It is what `settings.json` holds, what the `set_settings` op
//! takes, and what the `settings` op returns, so there is no second shape to drift from the first.
//! Unknown fields are refused rather than ignored: a client still sending a renamed or retired key
//! gets an error, not tideway's silent success that changed nothing.
//!
//! Each field's effect on a running daemon is applied where it is read, and says when it applies.

use std::collections::BTreeMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{Result, SinkId};

/// Everything a user can set that has to survive a restart.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    /// Per-output preferences, keyed by the output's id (`SinkInfo::id`, one per physical
    /// speaker). An output with no entry uses the defaults.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub outputs: BTreeMap<String, OutputSettings>,
    /// What the queue does on its own.
    #[serde(skip_serializing_if = "QueueSettings::is_default")]
    pub queue: QueueSettings,
    /// The Spotify connection (`spotify.web`).
    #[serde(skip_serializing_if = "SpotifySettings::is_default")]
    pub spotify: SpotifySettings,
}

/// The Spotify connection. Spotify's Web API only serves a developer app the user registers
/// themselves, so canon can't ship a client id: the user supplies theirs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SpotifySettings {
    /// The client id of the user's Spotify developer app, which must have
    /// `http://127.0.0.1:8898/spotify/callback` registered as a redirect URI. Read at each
    /// sign-in and each `services` listing, so setting it needs no restart.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
}

impl SpotifySettings {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// What the queue does on its own.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct QueueSettings {
    /// When the last track in the queue starts, add tracks like it (the service's radio for it),
    /// so playback carries on instead of stopping. Applies from the next track.
    pub autoplay: bool,
}

impl QueueSettings {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

impl Settings {
    /// The effective preferences for `output`.
    #[must_use]
    pub fn output(&self, output: &SinkId) -> OutputSettings {
        self.outputs.get(&output.0).cloned().unwrap_or_default()
    }
}

/// Preferences for one output.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OutputSettings {
    /// How tracks are delivered to a network renderer. Applies from the next track or seek.
    pub mode: OutputMode,
}

/// How tracks reach a network renderer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputMode {
    /// One continuous stream across the queue: gapless joins (and, later, crossfade) on every
    /// renderer. The renderer's own display shows the first track of the stream.
    #[default]
    Flow,
    /// One stream per track: the renderer's display is right for every track, at the cost of a
    /// gap between tracks.
    Standard,
}

/// Where settings live: read them, and replace them whole.
#[async_trait]
pub trait SettingsStore: Send + Sync {
    /// The current settings.
    fn get(&self) -> Settings;

    /// Replace the settings and persist them. On error nothing has changed.
    async fn set(&self, settings: Settings) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_document_is_the_defaults() {
        let settings: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(settings, Settings::default());
        assert_eq!(
            settings.output(&SinkId("anything".into())).mode,
            OutputMode::Flow
        );
    }

    /// A stale or misspelled key is an error, never a silent no-op.
    #[test]
    fn unknown_fields_are_refused_at_every_level() {
        assert!(serde_json::from_str::<Settings>(r#"{"crossfade": 3}"#).is_err());
        assert!(
            serde_json::from_str::<Settings>(r#"{"outputs": {"x": {"moed": "flow"}}}"#).is_err()
        );
        assert!(
            serde_json::from_str::<Settings>(r#"{"outputs": {"x": {"mode": "fast"}}}"#).is_err()
        );
    }

    #[test]
    fn autoplay_round_trips_and_is_off_by_default() {
        assert!(!Settings::default().queue.autoplay);
        let json = r#"{"queue":{"autoplay":true}}"#;
        let settings: Settings = serde_json::from_str(json).unwrap();
        assert!(settings.queue.autoplay);
        assert_eq!(serde_json::to_string(&settings).unwrap(), json);
        assert!(serde_json::from_str::<Settings>(r#"{"queue":{"autoplya":true}}"#).is_err());
    }

    #[test]
    fn the_spotify_client_id_round_trips_and_is_unset_by_default() {
        assert_eq!(Settings::default().spotify.client_id, None);
        assert_eq!(serde_json::to_string(&Settings::default()).unwrap(), "{}");
        let json = r#"{"spotify":{"client_id":"abc123"}}"#;
        let settings: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(settings.spotify.client_id.as_deref(), Some("abc123"));
        assert_eq!(serde_json::to_string(&settings).unwrap(), json);
        assert!(serde_json::from_str::<Settings>(r#"{"spotify":{"clientid":"x"}}"#).is_err());
    }

    #[test]
    fn a_per_output_mode_round_trips() {
        let json = r#"{"outputs":{"Kitchen":{"mode":"standard"}}}"#;
        let settings: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(
            settings.output(&SinkId("Kitchen".into())).mode,
            OutputMode::Standard
        );
        assert_eq!(serde_json::to_string(&settings).unwrap(), json);
    }
}
