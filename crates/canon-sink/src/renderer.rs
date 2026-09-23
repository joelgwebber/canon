//! What every network renderer protocol shares: opening a session on a discovered device, and
//! the edge filter over its reports.
//!
//! A protocol module (Cast today, DLNA next) owns only its wire: it connects, turns commands into
//! protocol calls, and *classifies* what the device says into [`RendererEvent`]s. Which reports are
//! allowed to repeat, and how a device becomes a session, is decided once here.

use canon_core::{Error, RendererEvent, RendererReport, Result, Sink, SinkKind};
use tokio::sync::mpsc;

use crate::DiscoveredDevice;
use crate::cast::CastSink;

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
            let (sink, events) =
                CastSink::connect(device.id.clone(), device.name.clone(), device.addr).await?;
            Ok((Box::new(sink), events))
        }
        SinkKind::Dlna => Err(Error::Unsupported(
            "DLNA renderers are not supported yet".into(),
        )),
        SinkKind::Local => Err(Error::Unsupported(
            "the local output is not a network renderer".into(),
        )),
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
