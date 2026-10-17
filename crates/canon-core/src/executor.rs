//! What passes between a player's actor and its executor (canon-806d).
//!
//! The actor decides and emits [`Effect`]s; an executor carries them out and reports what really
//! happened as [`EngineEvent`]s tagged with the generation they belong to. In one process those
//! cross a channel. Across the network (a player carrying out its own playback against the server
//! that keeps its queue) they cross a WebSocket as these messages, and nothing else is shared:
//! the actor judges every report by its generation exactly as it does in process, so the network
//! adds latency, not semantics.

use serde::{Deserialize, Serialize};

use crate::{Effect, EngineEvent, SinkId, SinkInfo};

/// From the server to an executor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExecutorOrder {
    /// Carry out one of the actor's decisions, in the order sent. A start or a prepare comes with
    /// `audio`: the path, with its token, to fetch the track from on this server (`GET`, adding
    /// `&position_ms=` for a start from elsewhere than the top).
    Effect {
        effect: Effect,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        audio: Option<String>,
    },
    /// Connect to output `sink`, so that playback can move there; answer with
    /// [`ExecutorReport::Selected`] carrying the same `request`. Only once it has does the actor
    /// restart playback there, as when the server drove its own outputs.
    Select { request: u64, sink: SinkId },
}

/// From an executor to the server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExecutorReport {
    /// What happened, for the playback of `generation`. `None` only for what is about the output
    /// rather than one playback ([`EngineEvent::SinkFailed`], [`EngineEvent::RendererVolume`]); the
    /// server drops any other report without one.
    Engine {
        #[serde(default)]
        generation: Option<u64>,
        event: EngineEvent,
    },
    /// The outputs this executor can play to, local first: what its player's clients may select.
    Outputs { sinks: Vec<SinkInfo> },
    /// The answer to [`ExecutorOrder::Select`]: `error` says why the output couldn't be reached.
    Selected {
        request: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
}

/// [`std::time::Duration`] on the wire as whole milliseconds, which is what every other time in
/// the protocol is.
pub(crate) mod millis {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serializer};

    #[allow(clippy::trivially_copy_pass_by_ref)] // serde's `with` passes a reference
    pub fn serialize<S: Serializer>(duration: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        Ok(Duration::from_millis(u64::deserialize(d)?))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::{EntityId, PositionDrive, RendererState, TrackMeta, TrackRef};

    /// Every order and report survives the trip, and times read as milliseconds.
    #[test]
    fn messages_read_back_as_written() {
        let track = TrackRef {
            id: EntityId::new(),
            meta: TrackMeta {
                title: "Army of Me".into(),
                ..TrackMeta::default()
            },
            sources: Vec::new(),
        };
        let orders = [
            ExecutorOrder::Effect {
                effect: Effect::Start {
                    generation: 3,
                    track,
                    position: Duration::from_millis(42_500),
                },
                audio: Some("/audio/x?token=t".into()),
            },
            ExecutorOrder::Effect {
                effect: Effect::SetVolume(0.5),
                audio: None,
            },
            ExecutorOrder::Effect {
                effect: Effect::Pause,
                audio: None,
            },
            ExecutorOrder::Select {
                request: 1,
                sink: SinkId("uuid:kef".into()),
            },
        ];
        for order in orders {
            let json = serde_json::to_value(&order).unwrap();
            assert_eq!(
                serde_json::from_value::<ExecutorOrder>(json).unwrap(),
                order
            );
        }
        let start = serde_json::to_value(ExecutorOrder::Effect {
            effect: Effect::Halt { generation: 4 },
            audio: None,
        })
        .unwrap();
        assert_eq!(start["effect"]["effect"], "halt");

        let reports = [
            ExecutorReport::Engine {
                generation: Some(3),
                event: EngineEvent::Loaded {
                    sample_rate: 44_100,
                    duration_ms: Some(234_000),
                    start_ms: 42_000,
                    drive: PositionDrive::Frames,
                    joins: true,
                },
            },
            ExecutorReport::Engine {
                generation: Some(3),
                event: EngineEvent::RendererPosition(Duration::from_millis(1_250)),
            },
            ExecutorReport::Engine {
                generation: Some(3),
                event: EngineEvent::RendererState(RendererState::Buffering),
            },
            ExecutorReport::Engine {
                generation: None,
                event: EngineEvent::SinkFailed {
                    sink: SinkId("uuid:kef".into()),
                    why: "gone".into(),
                },
            },
            ExecutorReport::Outputs {
                sinks: vec![SinkInfo::local()],
            },
            ExecutorReport::Selected {
                request: 1,
                error: Some("refused".into()),
            },
        ];
        for report in reports {
            let json = serde_json::to_value(&report).unwrap();
            assert_eq!(
                serde_json::from_value::<ExecutorReport>(json).unwrap(),
                report
            );
        }
        let position =
            serde_json::to_value(EngineEvent::RendererPosition(Duration::from_millis(1_250)))
                .unwrap();
        assert_eq!(
            position,
            serde_json::json!({"event": "renderer_position", "data": 1250})
        );
    }
}
