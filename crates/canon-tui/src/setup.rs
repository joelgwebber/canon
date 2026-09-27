//! The Outputs and Settings tabs: where playback goes, the service connections, and the
//! preferences a listener changes (streaming order, autoplay, MusicBrainz lookups).
//!
//! Their rows are derived from what the daemon last said (sinks, services, settings) rather than
//! stored, so they can't drift from it; the cursor is an index into them. Settings are changed
//! the protocol's way: read, modify, write the whole document back.

use std::time::{Duration, Instant};

use canon_api::protocol::ServiceView;
use canon_core::{
    ConnectionInfo, Health, LoginFlow, OutputMode, Service, Settings, SinkEndpoint, SinkInfo,
    SinkKind,
};

/// One row of the Outputs or Settings tab.
#[derive(Debug, Clone, PartialEq)]
pub enum SetupRow {
    Heading(String),
    /// A speaker (or the local output), selected by its preferred protocol.
    Output(SinkInfo),
    /// One protocol of a speaker that speaks several, to pin it.
    Endpoint {
        name: String,
        endpoint: SinkEndpoint,
    },
    Connection(ConnectionInfo),
    /// A streaming service, at its place in the user's order.
    Order(Service),
    Toggle(Toggle, bool),
}

impl SetupRow {
    pub(crate) fn selectable(&self) -> bool {
        !matches!(self, SetupRow::Heading(_))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toggle {
    Autoplay,
    Identify,
}

impl Toggle {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Toggle::Autoplay => "Autoplay: keep playing similar tracks when the queue runs out",
            Toggle::Identify => "Identify tracks with MusicBrainz (sends ISRCs and barcodes)",
        }
    }

    pub(crate) fn get(self, settings: &Settings) -> bool {
        match self {
            Toggle::Autoplay => settings.queue.autoplay,
            Toggle::Identify => settings.library.identify,
        }
    }

    pub(crate) fn set(self, settings: &mut Settings, on: bool) {
        match self {
            Toggle::Autoplay => settings.queue.autoplay = on,
            Toggle::Identify => settings.library.identify = on,
        }
    }
}

/// The Outputs tab's rows: each output, then its protocols when it has more than one.
#[must_use]
pub fn output_rows(sinks: &[SinkInfo]) -> Vec<SetupRow> {
    let mut rows = Vec::new();
    for sink in sinks {
        rows.push(SetupRow::Output(sink.clone()));
        if sink.protocols.len() > 1 {
            rows.extend(sink.protocols.iter().map(|endpoint| SetupRow::Endpoint {
                name: sink.name.clone(),
                endpoint: endpoint.clone(),
            }));
        }
    }
    rows
}

/// The Settings tab's rows: connections by service, the streaming order, then toggles.
#[must_use]
pub fn settings_rows(services: &[ServiceView], settings: Option<&Settings>) -> Vec<SetupRow> {
    let mut rows = vec![SetupRow::Heading("Services".into())];
    for view in services {
        for method in &view.methods {
            let connection = view
                .connections
                .iter()
                .find(|c| c.id == method.id)
                .cloned()
                .unwrap_or_else(|| ConnectionInfo {
                    id: method.id.clone(),
                    service: method.service,
                    label: method.label.clone(),
                    grants: method.grants,
                    verified: None,
                    health: Health::NeedsLogin,
                    account: None,
                });
            rows.push(SetupRow::Connection(connection));
        }
    }
    if let Some(settings) = settings {
        rows.push(SetupRow::Heading(
            "Streaming order (plays count where they're played)".into(),
        ));
        rows.extend(settings.streaming.order.iter().map(|s| SetupRow::Order(*s)));
        rows.push(SetupRow::Heading("Preferences".into()));
        for toggle in [Toggle::Autoplay, Toggle::Identify] {
            rows.push(SetupRow::Toggle(toggle, toggle.get(settings)));
        }
    }
    rows
}

/// What a connection's state reads as.
#[must_use]
pub fn health(connection: &ConnectionInfo) -> String {
    let who = connection
        .account
        .as_ref()
        .map(|a| a.username.clone().unwrap_or_else(|| a.user_id.clone()));
    match &connection.health {
        Health::Ok => match who {
            Some(who) => format!("signed in as {who}"),
            None => "signed in".into(),
        },
        Health::NeedsLogin => "not signed in".into(),
        Health::Degraded { why } => format!("degraded: {why}"),
        Health::Failing { why } => format!("failing: {why}"),
    }
}

/// What a connection can do, briefly.
#[must_use]
pub fn grants(connection: &ConnectionInfo) -> String {
    let can = connection.verified.as_ref().unwrap_or(&connection.grants);
    let mut parts = Vec::new();
    if can.catalog {
        parts.push("browse".to_owned());
    }
    if can.library_read {
        parts.push("library".to_owned());
    }
    match can.stream {
        Some(quality) => parts.push(format!(
            "streams {}",
            serde_json::to_value(quality)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default()
        )),
        None => parts.push("no audio".to_owned()),
    }
    parts.join(" · ")
}

/// A protocol's name.
#[must_use]
pub fn protocol(kind: SinkKind) -> &'static str {
    match kind {
        SinkKind::Local => "local",
        SinkKind::Chromecast => "cast",
        SinkKind::Dlna => "dlna",
    }
}

/// An output's mode as settings have it.
#[must_use]
pub fn mode(settings: Option<&Settings>, output: &SinkInfo) -> Option<OutputMode> {
    if output.kind == SinkKind::Local {
        return None;
    }
    Some(
        settings
            .map(|s| s.output(&output.id).mode)
            .unwrap_or_default(),
    )
}

/// A sign-in in progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Login {
    pub method: String,
    pub flow: LoginFlow,
    /// The URL being pasted back, for a browser login.
    pub input: String,
    /// When to ask again whether a device code has been approved.
    pub next_poll: Option<Instant>,
    pub interval: Duration,
}
