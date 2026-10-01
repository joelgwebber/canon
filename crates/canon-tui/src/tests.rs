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

// --- browsing ---

use canon_library::{
    AlbumDetail, AlbumView, ArtistView, EntityKind, ItemRef, LibraryPage, ListedTrack, Named,
    SearchView, TrackView,
};

use crate::browse::Row;

fn track_view(title: &str, artist: &str, saved: bool) -> TrackView {
    TrackView {
        id: EntityId::new(),
        title: title.into(),
        artists: vec![Named {
            id: EntityId::new(),
            name: artist.into(),
        }],
        album: Some(Named {
            id: EntityId::new(),
            name: "Sorceress".into(),
        }),
        duration_ms: Some(341_000),
        artwork_url: None,
        saved,
        sources: vec![SourceRef::Tidal { id: "1".into() }],
        plays_from: Some(canon_core::Service::Tidal),
    }
}

fn album_view(title: &str) -> AlbumView {
    AlbumView {
        id: EntityId::new(),
        title: title.into(),
        credit: "Opeth".into(),
        artists: Vec::new(),
        release_date: Some("2016-09-30".into()),
        artwork_url: None,
        saved: false,
        sources: Vec::new(),
    }
}

/// Answer the outstanding requests with `reply`, which sees each request.
fn reply_with(app: &mut App, reply: impl Fn(&ClientMessage) -> ReplyData) -> Vec<ClientMessage> {
    let requests = app.take_requests();
    for request in &requests {
        app.apply(ServerMessage::ok(request.id, reply(&request.message)));
    }
    requests.into_iter().map(|r| r.message).collect()
}

fn library(total: usize, from: usize, count: usize) -> ReplyData {
    ReplyData::Library(LibraryPage {
        total,
        tracks: (from..from + count)
            .map(|i| track_view(&format!("Track {i}"), "Opeth", true))
            .collect(),
        ..LibraryPage::default()
    })
}

#[test]
fn the_library_loads_a_page_at_a_time_as_the_cursor_nears_its_end() {
    let mut app = connected(Instant::now());
    app.handle_key(key(KeyCode::Char('2')));
    let asked = reply_with(&mut app, |_| library(450, 0, 200));
    assert!(matches!(
        asked[..],
        [ClientMessage::Library {
            kind: EntityKind::Track,
            offset: 0,
            limit: Some(200),
            ..
        }]
    ));
    insta::assert_snapshot!(draw(&app, 90, 14));

    app.handle_key(key(KeyCode::PageDown));
    assert!(app.take_requests().is_empty(), "far from the end yet");
    app.handle_key(key(KeyCode::Char('G')));
    let next = app.take_requests();
    assert!(matches!(
        next[..],
        [ClientEnvelope {
            message: ClientMessage::Library { offset: 200, .. },
            ..
        }]
    ));
    app.handle_key(key(KeyCode::Char('k')));
    assert!(app.take_requests().is_empty(), "not again while it loads");
}

#[test]
fn search_results_come_in_sections_and_a_track_plays_its_section_from_there() {
    let mut app = connected(Instant::now());
    app.handle_key(key(KeyCode::Char('/')));
    for c in "opeth".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    assert!(draw(&app, 90, 14).contains("search: opeth"));
    app.handle_key(key(KeyCode::Enter));
    let asked = reply_with(&mut app, |_| {
        ReplyData::Search(SearchView {
            tracks: vec![
                track_view("Era", "Opeth", false),
                track_view("Will o the Wisp", "Opeth", true),
            ],
            albums: vec![album_view("Sorceress")],
            artists: vec![ArtistView {
                id: EntityId::new(),
                name: "Opeth".into(),
                saved: true,
                sources: Vec::new(),
            }],
            playlists: Vec::new(),
        })
    });
    assert!(matches!(&asked[..], [ClientMessage::Search { query, .. }] if query == "opeth"));
    insta::assert_snapshot!(draw(&app, 90, 14));

    app.handle_key(key(KeyCode::Char('j')));
    app.handle_key(key(KeyCode::Enter));
    let played = reply_with(&mut app, |_| ReplyData::Ack);
    match &played[..] {
        [
            ClientMessage::QueueAdd {
                items,
                at: canon_api::protocol::QueueAt::Now,
                start: 1,
            },
        ] => assert_eq!(items.len(), 2, "the tracks, not the albums or artists"),
        other => panic!("{other:?}"),
    }
    assert!(draw(&app, 90, 14).contains("playing \"Will o the Wisp\""));
}

#[test]
fn escape_leaves_the_search_box_without_searching() {
    let mut app = connected(Instant::now());
    app.handle_key(key(KeyCode::Char('/')));
    app.handle_key(key(KeyCode::Char('x')));
    app.handle_key(key(KeyCode::Esc));
    assert!(app.input.is_none());
    assert!(app.take_requests().is_empty());
    app.handle_key(key(KeyCode::Char('q')));
    assert!(app.should_quit(), "keys are the app's again");
}

#[test]
fn an_album_opens_with_its_discs_and_closes_again() {
    let mut app = connected(Instant::now());
    app.handle_key(key(KeyCode::Char('2')));
    app.handle_key(key(KeyCode::Char(']')));
    let albums = reply_with(&mut app, |message| match message {
        ClientMessage::Library {
            kind: EntityKind::Album,
            ..
        } => ReplyData::Library(LibraryPage {
            total: 1,
            albums: vec![album_view("Sorceress")],
            ..LibraryPage::default()
        }),
        _ => library(1, 0, 1),
    });
    assert_eq!(albums.len(), 2, "tracks first, then albums");

    app.handle_key(key(KeyCode::Enter));
    reply_with(&mut app, |_| {
        let listed = |disc, position, title| ListedTrack {
            disc,
            position,
            track: track_view(title, "Opeth", false),
        };
        ReplyData::Album(AlbumDetail {
            album: album_view("Sorceress"),
            tracks: vec![
                listed(1, 1, "Persephone"),
                listed(1, 2, "Sorceress"),
                listed(2, 1, "The Ward"),
            ],
        })
    });
    let page = app.page().unwrap();
    assert_eq!(page.title, "Sorceress — Opeth");
    assert!(matches!(&page.rows[0], Row::Heading(h) if h == "Disc 1"));
    assert!(matches!(&page.rows[3], Row::Heading(h) if h == "Disc 2"));
    assert_eq!(page.cursor, 1, "on the first track, not the heading");

    app.handle_key(key(KeyCode::Char('h')));
    assert_eq!(app.page().unwrap().title, "Albums");
}

#[test]
fn queue_keys_queue_what_is_selected_and_say_so() {
    let mut app = connected(Instant::now());
    app.handle_key(key(KeyCode::Char('2')));
    reply_with(&mut app, |_| library(1, 0, 1));
    for (code, at, said) in [
        (
            'a',
            canon_api::protocol::QueueAt::End,
            "added \"Track 0\" to the queue",
        ),
        (
            'A',
            canon_api::protocol::QueueAt::Next,
            "\"Track 0\" plays next",
        ),
        (
            'P',
            canon_api::protocol::QueueAt::Now,
            "playing \"Track 0\"",
        ),
    ] {
        app.handle_key(key(KeyCode::Char(code)));
        let sent = reply_with(&mut app, |_| ReplyData::Ack);
        match &sent[..] {
            [
                ClientMessage::QueueAdd {
                    items, at: sent_at, ..
                },
            ] => {
                assert_eq!(*sent_at, at);
                assert!(matches!(items[..], [ItemRef::Entity { .. }]));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(app.notice.as_deref(), Some(said));
    }
}

#[test]
fn saving_marks_the_item_wherever_it_is_shown() {
    let mut app = connected(Instant::now());
    app.handle_key(key(KeyCode::Char('/')));
    app.handle_key(key(KeyCode::Char('x')));
    app.handle_key(key(KeyCode::Enter));
    let era = track_view("Era", "Opeth", false);
    let shown = era.clone();
    reply_with(&mut app, move |_| {
        ReplyData::Search(SearchView {
            tracks: vec![shown.clone()],
            ..SearchView::default()
        })
    });
    app.handle_key(key(KeyCode::Char('*')));
    let sent = reply_with(&mut app, |_| ReplyData::Ack);
    assert!(matches!(sent[..], [ClientMessage::Save { .. }]));
    assert_eq!(app.page().unwrap().selected().unwrap().saved(), Some(true));
    assert_eq!(app.notice.as_deref(), Some("saved \"Era\""));

    app.handle_key(key(KeyCode::Char('*')));
    let sent = reply_with(&mut app, |_| ReplyData::Ack);
    assert!(matches!(sent[..], [ClientMessage::Unsave { .. }]));
    assert_eq!(app.page().unwrap().selected().unwrap().saved(), Some(false));
}

// --- outputs and settings ---

use canon_api::protocol::ServiceView;
use canon_core::{
    Capabilities, ConnectionInfo, DeviceCode, FlowKind, Health, LoginFlow, LoginStatus, Method,
    OutputMode, Service, Settings,
};

fn tunes() -> SinkInfo {
    SinkInfo {
        id: SinkId("kef".into()),
        name: "Tunes".into(),
        kind: SinkKind::Chromecast,
        protocols: vec![
            canon_core::SinkEndpoint {
                id: SinkId("kef-cast".into()),
                kind: SinkKind::Chromecast,
            },
            canon_core::SinkEndpoint {
                id: SinkId("uuid:kef".into()),
                kind: SinkKind::Dlna,
            },
        ],
    }
}

fn services() -> Vec<ServiceView> {
    let method = |id: &str, flow| Method {
        id: id.into(),
        service: Service::Tidal,
        label: format!("Tidal ({id})"),
        flow,
        grants: Capabilities::default(),
        note: String::new(),
    };
    vec![ServiceView {
        service: Service::Tidal,
        methods: vec![
            method("tidal.pkce", FlowKind::Browser),
            method("tidal.device", FlowKind::DeviceCode),
        ],
        connections: vec![ConnectionInfo {
            id: "tidal.pkce".into(),
            service: Service::Tidal,
            label: "Tidal (browser login)".into(),
            grants: Capabilities {
                catalog: true,
                library_read: true,
                library_write: true,
                recommendations: true,
                stream: Some(canon_core::Quality::HiRes),
            },
            verified: None,
            health: Health::Ok,
            account: Some(canon_core::Account {
                service: Service::Tidal,
                user_id: "189763387".into(),
                username: None,
                attributes: std::collections::BTreeMap::new(),
            }),
        }],
    }]
}

/// Answer requests as a daemon with Tunes, two Tidal logins and default settings would.
fn setup_reply(settings: &Settings) -> impl Fn(&ClientMessage) -> ReplyData + '_ {
    move |message| match message {
        ClientMessage::ListSinks => ReplyData::Sinks {
            sinks: vec![SinkInfo::local(), tunes()],
        },
        ClientMessage::Settings => ReplyData::Settings {
            settings: settings.clone(),
        },
        ClientMessage::Services => ReplyData::Services {
            services: services(),
        },
        _ => ReplyData::Ack,
    }
}

#[test]
fn an_output_is_chosen_whole_or_by_one_of_its_protocols() {
    let settings = Settings::default();
    let mut app = connected(Instant::now());
    app.handle_key(key(KeyCode::Char('5')));
    reply_with(&mut app, setup_reply(&settings));
    insta::assert_snapshot!(draw(&app, 80, 14));

    app.handle_key(key(KeyCode::Char('j')));
    app.handle_key(key(KeyCode::Enter));
    assert!(matches!(&reply_with(&mut app, setup_reply(&settings))[..],
        [ClientMessage::SelectSink { sink }] if sink == "kef"));
    assert_eq!(app.notice.as_deref(), Some("playing on Tunes"));

    app.handle_key(key(KeyCode::Char('j')));
    app.handle_key(key(KeyCode::Char('j')));
    app.handle_key(key(KeyCode::Enter));
    assert!(matches!(&reply_with(&mut app, setup_reply(&settings))[..],
        [ClientMessage::SelectSink { sink }] if sink == "uuid:kef"));

    app.handle_key(key(KeyCode::Char('f')));
    let sent = reply_with(&mut app, setup_reply(&settings));
    match &sent[..] {
        [ClientMessage::SetSettings { settings }] => {
            assert_eq!(settings.outputs["kef"].mode, OutputMode::Standard);
        }
        other => panic!("{other:?}"),
    }
    assert!(
        matches!(
            app.take_requests()[..],
            [ClientEnvelope {
                message: ClientMessage::Settings,
                ..
            }]
        ),
        "read back once written"
    );
}

#[test]
fn preferences_are_changed_by_writing_the_settings_back_whole() {
    let settings = Settings::default();
    let mut app = connected(Instant::now());
    app.handle_key(key(KeyCode::Char('6')));
    reply_with(&mut app, setup_reply(&settings));
    insta::assert_snapshot!(draw(&app, 90, 16));

    // The streaming order: move Tidal below Spotify.
    app.handle_key(key(KeyCode::Char('j')));
    app.handle_key(key(KeyCode::Char('j')));
    app.handle_key(key(KeyCode::Char('J')));
    match &reply_with(&mut app, setup_reply(&settings))[..] {
        [ClientMessage::SetSettings { settings }] => {
            assert_eq!(settings.streaming.order, [Service::Spotify, Service::Tidal]);
        }
        other => panic!("{other:?}"),
    }

    reply_with(&mut app, setup_reply(&settings)); // the read-back

    // Autoplay, the second to last row.
    app.handle_key(key(KeyCode::Char('G')));
    app.handle_key(key(KeyCode::Char('k')));
    app.handle_key(key(KeyCode::Enter));
    match &reply_with(&mut app, setup_reply(&settings))[..] {
        [ClientMessage::SetSettings { settings }] => {
            assert!(settings.queue.autoplay, "off by default, so turned on");
            assert!(settings.library.identify, "the rest as it was");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(app.notice.as_deref(), Some("autoplay turned on"));
}

#[test]
fn a_device_code_sign_in_polls_until_approved() {
    let settings = Settings::default();
    let start = Instant::now();
    let mut app = connected(start);
    app.handle_key(key(KeyCode::Char('6')));
    reply_with(&mut app, setup_reply(&settings));
    app.handle_key(key(KeyCode::Char('j')));
    app.handle_key(key(KeyCode::Enter));
    reply_with(&mut app, |message| match message {
        ClientMessage::Connect { method } => ReplyData::Connecting {
            method: method.clone(),
            login: LoginFlow::DeviceCode {
                code: DeviceCode {
                    user_code: "JKMFZ".into(),
                    verification_uri: "link.tidal.com".into(),
                    verification_uri_complete: Some("link.tidal.com/JKMFZ".into()),
                    expires_in: 300,
                    interval: 2,
                },
            },
        },
        _ => ReplyData::Ack,
    });
    assert!(draw(&app, 90, 16).contains("code: JKMFZ"));

    app.tick(start + Duration::from_secs(1));
    assert!(app.take_requests().is_empty(), "not before the interval");
    app.tick(start + Duration::from_secs(2));
    let polled = reply_with(&mut app, |_| ReplyData::Login {
        status: LoginStatus::Pending,
    });
    assert!(matches!(&polled[..],
        [ClientMessage::ConnectComplete { method, redirect: None }] if method == "tidal.device"));
    assert!(app.login.is_some());

    app.tick(start + Duration::from_secs(4));
    let done = reply_with(&mut app, |message| match message {
        ClientMessage::ConnectComplete { .. } => ReplyData::Login {
            status: LoginStatus::Authorized,
        },
        _ => ReplyData::Services {
            services: services(),
        },
    });
    assert!(matches!(done[..], [ClientMessage::ConnectComplete { .. }]));
    assert!(app.login.is_none());
    let refreshed = app.take_requests();
    assert!(
        matches!(
            refreshed[..],
            [ClientEnvelope {
                message: ClientMessage::Services,
                ..
            }]
        ),
        "the services are read back"
    );
    assert_eq!(app.notice.as_deref(), Some("signed in with tidal.device"));
}

#[test]
fn a_browser_sign_in_takes_the_pasted_address() {
    let mut app = connected(Instant::now());
    app.handle_key(key(KeyCode::Char('6')));
    let settings = Settings::default();
    reply_with(&mut app, setup_reply(&settings));
    // Signed in already: enter says how to sign out instead of starting over.
    app.handle_key(key(KeyCode::Enter));
    assert!(app.take_requests().is_empty());
    assert_eq!(
        app.notice.as_deref(),
        Some("tidal.pkce is signed in; X signs it out")
    );

    app.login = Some(crate::setup::Login {
        method: "spotify.web".into(),
        flow: LoginFlow::Browser {
            url: "https://accounts.spotify.com/authorize?…".into(),
        },
        input: String::new(),
        next_poll: None,
        interval: Duration::ZERO,
    });
    for c in "http://127.0.0.1:8898/cb?code=x".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    app.handle_key(key(KeyCode::Char('q')));
    assert!(
        !app.should_quit(),
        "q is part of the address while signing in"
    );
    app.handle_key(key(KeyCode::Backspace));
    app.handle_key(key(KeyCode::Enter));
    assert!(matches!(&app.take_requests()[..],
        [ClientEnvelope { message: ClientMessage::ConnectComplete { method, redirect: Some(url) }, .. }]
            if method == "spotify.web" && url == "http://127.0.0.1:8898/cb?code=x"));
}

// --- doc frames ---

/// Colour frames of representative scenes for `docs/tui.md`, written to `docs/assets/`. Not a
/// check: a generator, run on purpose when the TUI's look changes.
///
/// ```sh
/// cargo test -p canon-tui doc_frames -- --ignored
/// ```
#[test]
#[ignore = "writes docs/assets; run on purpose"]
fn doc_frames() {
    let (width, height) = (100, 26);
    let assets = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/assets");
    std::fs::create_dir_all(&assets).unwrap();
    let save = |name: &str, app: &App| {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| render(app, frame)).unwrap();
        let svg = toque::buffer_to_svg(terminal.backend().buffer());
        std::fs::write(assets.join(format!("tui-{name}.svg")), svg).unwrap();
    };

    let tracks: Vec<TrackRef> = [
        ("Speak to Me", "Pink Floyd", 64),
        ("Breathe (In the Air)", "Pink Floyd", 170),
        ("Army of Me", "Björk", 234),
        ("Backstabber", "The Dresden Dolls", 247),
        ("Siberian Khatru", "Yes", 535),
        ("Era", "Opeth", 342),
        ("The Lion's Roar", "Cynic", 274),
        ("Strid", "GoGo Penguin", 488),
        ("Windowpane", "Opeth", 464),
    ]
    .iter()
    .map(|(title, artist, secs)| {
        let mut t = track(title, artist, *secs);
        t.meta.album = None;
        t
    })
    .collect();
    let start = Instant::now();
    let mut app = App::new(start);
    app.resize(height);
    app.apply(ServerMessage::Hello {
        protocol: PROTOCOL_VERSION,
    });
    let mut snapshot = playing(2, 96_000, 1);
    let mut now = tracks[2].clone();
    now.meta.album = Some("Post".into());
    snapshot.track = Some(now);
    snapshot.duration_ms = Some(234_000);
    snapshot.queue.len = tracks.len();
    app.apply(ServerMessage::Snapshot { snapshot });
    answer(&mut app, &tracks, 1);
    save("queue", &app);

    app.handle_key(key(KeyCode::Char('?')));
    save("keys", &app);
    app.handle_key(key(KeyCode::Esc));

    let settings = Settings::default();
    app.handle_key(key(KeyCode::Char('2')));
    reply_with(&mut app, |_| {
        let saved = |title: &str, artist: &str, album: &str, secs: u64| {
            let mut t = track_view(title, artist, true);
            t.album = Some(Named {
                id: EntityId::new(),
                name: album.into(),
            });
            t.duration_ms = Some(secs * 1000);
            t
        };
        ReplyData::Library(LibraryPage {
            total: 290,
            tracks: vec![
                saved(
                    "When The Sun Bursts",
                    "Catching Flies",
                    "Silver Linings",
                    259,
                ),
                saved("Solar Motel", "The Flashbulb", "Nothing Is Real", 250),
                saved("City 66", "Tor", "Oasis Sky", 224),
                saved("Backstabber", "The Dresden Dolls", "The Dresden Dolls", 247),
                saved("My Only Swerving", "El Ten Eleven", "El Ten Eleven", 315),
                saved("Out of Nothing", "Astralia", "Solstice", 517),
                saved("Open", "GoGo Penguin", "GoGo Penguin", 287),
                saved("Strid", "GoGo Penguin", "A Humdrum Star", 488),
                saved(
                    "Mend and Make Safe",
                    "And So I Watch You from Afar",
                    "All Hail Bright Futures",
                    262,
                ),
                saved("you go", "toe", "For Long Tomorrow", 215),
            ],
            ..LibraryPage::default()
        })
    });
    save("library", &app);

    app.handle_key(key(KeyCode::Char('/')));
    for c in "opeth".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    app.handle_key(key(KeyCode::Enter));
    reply_with(&mut app, |_| {
        ReplyData::Search(SearchView {
            tracks: vec![
                track_view("Ghost of Perdition", "Opeth", true),
                track_view("Windowpane", "Opeth", false),
                track_view("Era", "Opeth", true),
            ],
            albums: vec![album_view("Sorceress"), album_view("Blackwater Park")],
            artists: vec![ArtistView {
                id: EntityId::new(),
                name: "Opeth".into(),
                saved: true,
                sources: Vec::new(),
            }],
            playlists: Vec::new(),
        })
    });
    save("search", &app);

    app.handle_key(key(KeyCode::Char('5')));
    reply_with(&mut app, setup_reply(&settings));
    save("outputs", &app);

    app.handle_key(key(KeyCode::Char('6')));
    reply_with(&mut app, setup_reply(&settings));
    save("settings", &app);
}
