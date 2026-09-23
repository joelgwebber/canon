//! What every network renderer protocol shares: opening a session on a discovered device, and
//! the edge filter over its reports.
//!
//! A protocol module (Cast today, DLNA next) owns only its wire: it connects, turns commands into
//! protocol calls, and *classifies* what the device says into [`RendererEvent`]s. Which reports are
//! allowed to repeat, and how a device becomes a session, is decided once here.

use std::collections::BTreeMap;
use std::net::IpAddr;

use canon_core::{
    Error, RendererEvent, RendererReport, Result, Sink, SinkEndpoint, SinkInfo, SinkKind,
};
use tokio::sync::mpsc;

use crate::DiscoveredDevice;
use crate::cast::CastSink;
use crate::dlna::DlnaSink;

/// A renderer's reports, in protocol-neutral terms, each attributed to the load it describes. The
/// channel closing means the session is over: either we dropped the sink, or its connection died.
pub type RendererEvents = mpsc::UnboundedReceiver<RendererReport>;

/// Open a control session on a discovered renderer, whatever protocol it speaks.
///
/// This is the one place that knows which protocols exist. Everything above it holds a
/// `Box<dyn Sink>` and reads [`RendererEvents`].
///
/// # Errors
/// [`Error::Sink`] if the device can't be reached; [`Error::Unsupported`] for a protocol canon
/// does not speak yet.
pub async fn connect(device: &DiscoveredDevice) -> Result<(Box<dyn Sink>, RendererEvents)> {
    match device.kind {
        SinkKind::Chromecast => {
            let (sink, events) = CastSink::connect(device.id.clone(), device.addr).await?;
            Ok((Box::new(sink), events))
        }
        SinkKind::Dlna => {
            let location = device.location.as_deref().ok_or_else(|| {
                Error::Sink(format!(
                    "{} was discovered without a description",
                    device.name
                ))
            })?;
            let (sink, events) = DlnaSink::connect(device.id.clone(), location).await?;
            Ok((Box::new(sink), events))
        }
        SinkKind::Local => Err(Error::Unsupported(
            "the local output is not a network renderer".into(),
        )),
    }
}

/// The selectable outputs behind a discovery snapshot: one per physical device, however many
/// protocols reach it, each naming its preferred protocol first.
///
/// A device is identified by its address. Nothing better is on offer: a speaker's protocols do not
/// share an identity (the LS50 Wireless II's Cast id and UPnP UDN are unrelated), but its Cast and
/// DLNA endpoints are the same host, seen at the same moment.
#[must_use]
pub fn outputs(devices: &[DiscoveredDevice]) -> Vec<SinkInfo> {
    let mut by_host: BTreeMap<IpAddr, Vec<&DiscoveredDevice>> = BTreeMap::new();
    for device in devices {
        by_host.entry(device.addr.ip()).or_default().push(device);
    }
    let mut outputs: Vec<SinkInfo> = by_host
        .into_values()
        .map(|mut endpoints| {
            endpoints.sort_by_key(|device| preference(device.kind));
            let preferred = endpoints[0];
            SinkInfo {
                id: preferred.id.clone(),
                name: preferred.name.clone(),
                kind: preferred.kind,
                protocols: endpoints
                    .iter()
                    .map(|device| SinkEndpoint {
                        id: device.id.clone(),
                        kind: device.kind,
                    })
                    .collect(),
            }
        })
        .collect();
    outputs.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.0.cmp(&b.id.0)));
    outputs
}

/// Which protocol to drive a device over when it speaks several, lowest first. Cast first: its
/// sessions are explicit, a takeover is an unambiguous foreign media session, and it is the path
/// with the most time on real hardware. DLNA is the fallback — and the A/B.
fn preference(kind: SinkKind) -> u8 {
    match kind {
        SinkKind::Chromecast => 0,
        SinkKind::Dlna => 1,
        SinkKind::Local => 2,
    }
}

/// Lets conditions and positions through every time, and each edge through once.
///
/// Level vs edge is the whole of it. `State` describes a condition the renderer is in, and it
/// always goes through: the player is the only thing that knows its own state, so it is the only
/// thing that can decide whether a report is a transition. Suppressing conditions here once meant
/// a re-LOAD (which puts the player back to `Loading`) left the protocol believing it had already
/// reported `Playing`, wedging playback in `Loading` forever — and it would equally hide a pause
/// the device never actually performed.
///
/// `Ended`/`Superseded`/`Failed` are edges: each drives a one-shot action (queue auto-advance,
/// fail-back), so a continuous poll must not fire them over and over. An edge is suppressed only
/// while nothing else has been reported since, so a second track ending after the first one played
/// still gets through. [`reset`](Self::reset) it on every new load: an edge belongs to one stream.
#[derive(Debug, Default)]
pub struct EdgeFilter {
    last: Option<RendererEvent>,
}

impl EdgeFilter {
    /// Whether `event` should be forwarded.
    pub fn admit(&mut self, event: &RendererEvent) -> bool {
        // Positions are a continuous correction and never repeat meaningfully; storing one would
        // also mask the next terminal event.
        if matches!(event, RendererEvent::Position(_)) {
            return true;
        }
        if event.is_edge() && self.last.as_ref() == Some(event) {
            return false;
        }
        self.last = Some(event.clone());
        true
    }

    /// Forget what was reported about the previous stream. A new load starts a new stream, whose
    /// edges have not happened yet — even if the last one also ended.
    pub fn reset(&mut self) {
        self.last = None;
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use canon_core::RendererState;

    use super::*;

    fn device(id: &str, name: &str, kind: SinkKind, addr: &str) -> DiscoveredDevice {
        DiscoveredDevice {
            id: canon_core::SinkId(id.to_string()),
            name: name.to_string(),
            kind,
            addr: addr.parse().unwrap(),
            location: None,
        }
    }

    /// One speaker, two protocols: listed once, Cast preferred, both reachable.
    #[test]
    fn a_multi_protocol_speaker_is_one_output() {
        let outputs = outputs(&[
            device(
                "dlna:uuid:c353",
                "Tunes",
                SinkKind::Dlna,
                "192.168.0.205:16500",
            ),
            device(
                "LS50._googlecast",
                "Tunes",
                SinkKind::Chromecast,
                "192.168.0.205:8009",
            ),
            device(
                "Kitchen._googlecast",
                "Kitchen",
                SinkKind::Chromecast,
                "192.168.0.7:8009",
            ),
        ]);
        assert_eq!(outputs.len(), 2);
        let tunes = outputs.iter().find(|o| o.name == "Tunes").unwrap();
        assert_eq!(tunes.kind, SinkKind::Chromecast);
        assert_eq!(tunes.id.0, "LS50._googlecast");
        let kinds: Vec<SinkKind> = tunes.protocols.iter().map(|p| p.kind).collect();
        assert_eq!(kinds, vec![SinkKind::Chromecast, SinkKind::Dlna]);
    }

    #[test]
    fn a_single_protocol_device_lists_itself() {
        let outputs = outputs(&[device(
            "dlna:uuid:1",
            "Den",
            SinkKind::Dlna,
            "192.168.0.9:80",
        )]);
        assert_eq!(outputs[0].id.0, "dlna:uuid:1");
        assert_eq!(outputs[0].protocols.len(), 1);
    }

    #[test]
    fn a_repeated_condition_keeps_flowing() {
        let mut filter = EdgeFilter::default();
        let playing = RendererEvent::State(RendererState::Playing);
        for _ in 0..5 {
            assert!(filter.admit(&playing));
        }
    }

    #[test]
    fn a_repeated_edge_fires_once() {
        let mut filter = EdgeFilter::default();
        assert!(filter.admit(&RendererEvent::Ended));
        for _ in 0..5 {
            assert!(
                !filter.admit(&RendererEvent::Ended),
                "a finished stream must not advance the queue again"
            );
        }
    }

    #[test]
    fn an_edge_fires_again_after_something_else_happened() {
        let mut filter = EdgeFilter::default();
        assert!(filter.admit(&RendererEvent::Ended));
        assert!(filter.admit(&RendererEvent::State(RendererState::Playing)));
        assert!(filter.admit(&RendererEvent::Ended), "the next track's end");
    }

    #[test]
    fn a_new_load_rearms_every_edge() {
        let mut filter = EdgeFilter::default();
        assert!(filter.admit(&RendererEvent::Ended));
        filter.reset();
        assert!(
            filter.admit(&RendererEvent::Ended),
            "a short next track ending with no report in between must still advance"
        );
    }

    #[test]
    fn positions_neither_repeat_filter_nor_mask_an_edge() {
        let mut filter = EdgeFilter::default();
        assert!(filter.admit(&RendererEvent::Ended));
        assert!(filter.admit(&RendererEvent::Position(Duration::from_secs(1))));
        assert!(filter.admit(&RendererEvent::Position(Duration::from_secs(1))));
        assert!(
            !filter.admit(&RendererEvent::Ended),
            "a position report must not re-arm a stale edge"
        );
    }
}
