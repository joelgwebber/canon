//! The client against scripted server messages: no daemon, no terminal.

use std::time::{Duration, Instant};

use canon_api::{ClientEnvelope, ClientMessage, PROTOCOL_VERSION, ReplyData, ServerMessage};
use canon_core::{
    Codec, EntityId, PlaybackState, PlayerSnapshot, PlayingFrom, QueueView, SinkId, SinkInfo,
    SinkKind, SourceRef, TrackMeta, TrackRef,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::App;
use crate::render::render;

fn track(title: &str, artist: &str, secs: u64) -> TrackRef {
    TrackRef {
        id: EntityId::new(),
        meta: TrackMeta {
            title: title.into(),
            artists: vec![artist.into()],
            album: Some("Sorceress".into()),
            duration_ms: Some(secs * 1000),
            artwork_url: None,
        },
        sources: vec![SourceRef::Tidal { id: "1".into() }],
    }
}

fn queue() -> Vec<TrackRef> {
    vec![
        track("Sorceress", "Opeth", 329),
        track("The Wilde Flowers", "Opeth", 430),
        track("Will o the Wisp", "Opeth", 307),
        track("Chrysalis", "Opeth", 432),
        track("Era", "Opeth", 342),
    ]
}

fn playing(index: usize, position_ms: u64, revision: u64) -> PlayerSnapshot {
    let tracks = queue();
    PlayerSnapshot {
        seq: 10,
        state: PlaybackState::Playing,
        track: Some(tracks[index].clone()),
        position_ms,
        duration_ms: tracks[index].meta.duration_ms,
        rate: 1.0,
        volume: 0.4,
        muted: false,
        sink: Some(SinkId("uuid:kef".into())),
        error: None,
        queue: QueueView {
            len: tracks.len(),
            index,
            revision,
            ..QueueView::default()
        },
        playing_from: Some(PlayingFrom {
            source: SourceRef::Tidal { id: "1".into() },
            codec: Codec::Flac,
            sample_rate: 44_100,
            bit_depth: Some(16),
        }),
    }
}

/// Answer every outstanding request as the daemon would.
fn answer(app: &mut App, tracks: &[TrackRef], revision: u64) -> Vec<ClientEnvelope> {
    let requests = app.take_requests();
    for request in &requests {
        let data = match request.message {
            ClientMessage::Queue => ReplyData::Queue {
                revision,
                index: 0,
                tracks: tracks.to_vec(),
            },
            ClientMessage::ListSinks => ReplyData::Sinks {
                sinks: vec![SinkInfo {
                    id: SinkId("kef".into()),
                    name: "Tunes".into(),
                    kind: SinkKind::Chromecast,
                    protocols: vec![canon_core::SinkEndpoint {
                        id: SinkId("uuid:kef".into()),
                        kind: SinkKind::Dlna,
                    }],
                }],
            },
            _ => ReplyData::Ack,
        };
        app.apply(ServerMessage::ok(request.id, data));
    }
    requests
}

/// A client connected to a daemon playing the third of five tracks, 1:23 in.
fn connected(at: Instant) -> App {
    let mut app = App::new(at);
    app.resize(14);
    app.apply(ServerMessage::Hello {
        protocol: PROTOCOL_VERSION,
    });
    app.apply(ServerMessage::Snapshot {
        snapshot: playing(2, 83_000, 1),
    });
    answer(&mut app, &queue(), 1);
    app
}

fn draw(app: &App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| render(app, frame)).unwrap();
    toque::SnapshotEncoder::new()
        .encode(terminal.backend().buffer())
        .join("\n")
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn press(app: &mut App, code: KeyCode) -> Vec<ClientMessage> {
    app.handle_key(key(code));
    app.take_requests().into_iter().map(|r| r.message).collect()
}

#[test]
fn the_queue_and_what_is_playing() {
    let app = connected(Instant::now());
    insta::assert_snapshot!(draw(&app, 80, 14));
}

#[test]
fn the_key_reference() {
    let mut app = connected(Instant::now());
    app.handle_key(key(KeyCode::Char('?')));
    insta::assert_snapshot!(draw(&app, 80, 16));
    app.handle_key(key(KeyCode::Char('j')));
    assert!(!app.help, "any key closes it");
    assert_eq!(app.cursor, 2, "without acting");
}

#[test]
fn a_new_client_starts_on_the_playing_entry() {
    let app = connected(Instant::now());
    assert_eq!(app.cursor, 2);
    assert_eq!(
        app.output_name(),
        "Tunes",
        "named through its DLNA endpoint"
    );
}

#[test]
fn position_moves_between_snapshots_and_holds_when_paused() {
    let start = Instant::now();
    let mut app = connected(start);
    app.tick(start + Duration::from_millis(2_500));
    assert_eq!(app.position(), Duration::from_millis(85_500));

    let mut paused = playing(2, 90_000, 1);
    paused.state = PlaybackState::Paused;
    paused.rate = 0.0;
    app.apply(ServerMessage::Snapshot { snapshot: paused });
    app.tick(start + Duration::from_secs(60));
    assert_eq!(app.position(), Duration::from_secs(90));

    app.apply(ServerMessage::Snapshot {
        snapshot: playing(2, 300_000, 1),
    });
    app.tick(start + Duration::from_secs(600));
    assert_eq!(app.position(), Duration::from_secs(307), "held at the end");
}

#[test]
fn transport_keys_send_commands() {
    let start = Instant::now();
    let mut app = connected(start);
    assert!(matches!(
        press(&mut app, KeyCode::Char(' '))[..],
        [ClientMessage::Pause]
    ));
    app.tick(start + Duration::from_secs(2));
    assert!(matches!(
        press(&mut app, KeyCode::Right)[..],
        [ClientMessage::Seek {
            position_ms: 95_000
        }]
    ));
    assert!(matches!(
        press(&mut app, KeyCode::Left)[..],
        [ClientMessage::Seek {
            position_ms: 75_000
        }]
    ));
    match press(&mut app, KeyCode::Char('+'))[..] {
        [ClientMessage::SetVolume { volume }] => assert!((volume - 0.45).abs() < 1e-6),
        ref other => panic!("{other:?}"),
    }
    assert!(matches!(
        press(&mut app, KeyCode::Char('n'))[..],
        [ClientMessage::Next]
    ));
    assert!(matches!(
        press(&mut app, KeyCode::Char('r'))[..],
        [ClientMessage::Repeat {
            mode: canon_core::Repeat::All
        }]
    ));
}

#[test]
fn queue_keys_edit_the_entry_under_the_cursor() {
    let mut app = connected(Instant::now());
    assert!(press(&mut app, KeyCode::Char('j')).is_empty());
    assert_eq!(app.cursor, 3);
    assert!(matches!(
        press(&mut app, KeyCode::Enter)[..],
        [ClientMessage::Jump { index: 3 }]
    ));
    assert!(matches!(
        press(&mut app, KeyCode::Char('K'))[..],
        [ClientMessage::Move { from: 3, to: 2 }]
    ));
    assert_eq!(app.cursor, 2, "the cursor moves with the entry");
    assert!(matches!(
        press(&mut app, KeyCode::Char('d'))[..],
        [ClientMessage::Remove { index: 2 }]
    ));
    assert!(press(&mut app, KeyCode::Char('G')).is_empty());
    assert_eq!(app.cursor, 4);
    assert!(
        press(&mut app, KeyCode::Char('J')).is_empty(),
        "nowhere to move the last entry"
    );
}

#[test]
fn the_queue_is_fetched_again_when_it_changes() {
    let mut app = connected(Instant::now());
    app.apply(ServerMessage::Snapshot {
        snapshot: playing(2, 84_000, 1),
    });
    assert!(
        app.take_requests().is_empty(),
        "same revision, nothing to fetch"
    );

    let shorter = queue()[..4].to_vec();
    app.apply(ServerMessage::Snapshot {
        snapshot: playing(2, 85_000, 2),
    });
    app.apply(ServerMessage::Snapshot {
        snapshot: playing(2, 86_000, 2),
    });
    let asked = answer(&mut app, &shorter, 2);
    assert_eq!(asked.len(), 1, "one fetch while one is outstanding");
    assert_eq!(app.queue.len(), 4);
}

#[test]
fn the_cursor_follows_playback_unless_moved_away() {
    let mut app = connected(Instant::now());
    app.apply(ServerMessage::Snapshot {
        snapshot: playing(3, 0, 1),
    });
    assert_eq!(app.cursor, 3);
    app.handle_key(key(KeyCode::Char('g')));
    app.apply(ServerMessage::Snapshot {
        snapshot: playing(4, 0, 1),
    });
    assert_eq!(app.cursor, 0, "left where the user put it");
}

#[test]
fn errors_and_a_lost_connection_are_shown() {
    let mut app = connected(Instant::now());
    app.handle_key(key(KeyCode::Char('n')));
    let request = app.take_requests().remove(0);
    app.apply(ServerMessage::err(request.id, "no next track"));
    assert!(draw(&app, 80, 14).contains("no next track"));

    app.lost("connection reset".into());
    let frame = draw(&app, 80, 14);
    assert!(frame.contains("disconnected: connection reset"), "{frame}");
    assert!(!app.awaiting_replies());
}

#[test]
fn long_and_wide_titles_are_cut_to_their_column() {
    let mut app = App::new(Instant::now());
    app.resize(8);
    app.apply(ServerMessage::Hello {
        protocol: PROTOCOL_VERSION,
    });
    let mut wide = track(
        "東京事変 — 群青日和 (Live at Budokan, Remastered Edition)",
        "東京事変",
        200,
    );
    wide.meta.album = None;
    let mut snapshot = playing(0, 0, 1);
    snapshot.duration_ms = wide.meta.duration_ms;
    snapshot.track = Some(wide.clone());
    app.apply(ServerMessage::Snapshot { snapshot });
    answer(&mut app, &[wide], 1);
    insta::assert_snapshot!(draw(&app, 60, 8));
}
