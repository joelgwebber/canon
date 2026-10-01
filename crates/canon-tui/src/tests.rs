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
    AlbumDetail, AlbumView, ArtistView, EntityKind, ItemRef, LibraryPage, ListedTrack, MixView,
    Named, PlaylistDetail, PlaylistView, SearchView, ServicePlaylistView, TrackView,
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
    assert!(app.prompt.is_none());
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

// --- remote playlists and mixes ---

fn service_playlist(name: &str, id: &str, track_count: usize) -> ServicePlaylistView {
    ServicePlaylistView {
        service: Service::Tidal,
        id: id.into(),
        name: name.into(),
        track_count,
    }
}

fn mix_view(name: &str, mix: &str, description: &str) -> MixView {
    MixView {
        service: Service::Tidal,
        mix: mix.into(),
        name: name.into(),
        description: description.into(),
    }
}

/// The Playlists tab's shelves: canon's own, then each service's, as `[` and `]` cycle them.
fn remote_reply(message: &ClientMessage) -> ReplyData {
    match message {
        ClientMessage::Library { .. } => ReplyData::Library(LibraryPage {
            total: 1,
            playlists: vec![PlaylistView {
                id: EntityId::new(),
                name: "Jazz practice".into(),
                track_count: 5,
                updated_at: 0,
            }],
            ..LibraryPage::default()
        }),
        ClientMessage::ServicePlaylists { .. } => ReplyData::ServicePlaylists {
            playlists: vec![
                service_playlist("Fantasy", "e7c9…01", 250),
                service_playlist("Jazz-ish", "e7c9…02", 130),
            ],
        },
        ClientMessage::Mixes { .. } => ReplyData::Mixes {
            mixes: vec![mix_view("My Mix 1", "0001", "Opeth, Cynic and more")],
        },
        ClientMessage::ServicePlaylist { .. } | ClientMessage::Mix { .. } => ReplyData::Tracks {
            tracks: vec![
                track_view("Seven Sons of Bjorn", "GoGo Penguin", false),
                track_view("Take Five", "The Dave Brubeck Quartet", true),
            ],
        },
        _ => ReplyData::Ack,
    }
}

#[test]
fn the_playlists_tab_switches_between_canons_own_and_each_services() {
    let mut app = connected(Instant::now());
    app.handle_key(key(KeyCode::Char('3')));
    let asked = reply_with(&mut app, remote_reply);
    assert!(matches!(
        asked[..],
        [ClientMessage::Library {
            kind: EntityKind::Playlist,
            ..
        }]
    ));
    assert_eq!(app.page().unwrap().title, "Playlists");

    // ] walks on to Tidal's own playlists, which are rows with no canon id.
    app.handle_key(key(KeyCode::Char(']')));
    let asked = reply_with(&mut app, remote_reply);
    assert!(matches!(
        asked[..],
        [ClientMessage::ServicePlaylists {
            service: Some(Service::Tidal)
        }]
    ));
    let page = app.page().unwrap();
    assert_eq!(page.title, "Tidal playlists");
    let selected = page.selected().unwrap();
    assert_eq!(selected.name(), "Fantasy");
    assert_eq!(selected.id(), None, "a service playlist is not an entity");
    assert_eq!(selected.saved(), None, "and so has nothing to save");
    insta::assert_snapshot!(draw(&app, 90, 14));

    app.handle_key(key(KeyCode::Char(']')));
    let asked = reply_with(&mut app, remote_reply);
    assert!(matches!(
        asked[..],
        [ClientMessage::Mixes {
            service: Some(Service::Tidal)
        }]
    ));
    assert_eq!(app.page().unwrap().title, "Tidal mixes");

    // Spotify has playlists but no mixes, so the cycle skips that combination.
    app.handle_key(key(KeyCode::Char(']')));
    let asked = reply_with(&mut app, remote_reply);
    assert!(matches!(
        asked[..],
        [ClientMessage::ServicePlaylists {
            service: Some(Service::Spotify)
        }]
    ));
    assert_eq!(app.page().unwrap().title, "Spotify playlists");

    // And round again, backwards.
    app.handle_key(key(KeyCode::Char(']')));
    reply_with(&mut app, remote_reply);
    assert_eq!(app.page().unwrap().title, "Playlists");
    app.handle_key(key(KeyCode::Char('[')));
    reply_with(&mut app, remote_reply);
    assert_eq!(app.page().unwrap().title, "Spotify playlists");
}

#[test]
fn a_service_playlist_and_a_mix_open_as_their_tracks() {
    let mut app = connected(Instant::now());
    app.handle_key(key(KeyCode::Char('3')));
    app.handle_key(key(KeyCode::Char(']')));
    reply_with(&mut app, remote_reply);

    app.handle_key(key(KeyCode::Enter));
    let asked = reply_with(&mut app, remote_reply);
    match &asked[..] {
        [
            ClientMessage::ServicePlaylist {
                item:
                    ItemRef::Service {
                        service: Service::Tidal,
                        id,
                        kind: EntityKind::Playlist,
                    },
            },
        ] => assert_eq!(id, "e7c9…01"),
        other => panic!("{other:?}"),
    }
    let page = app.page().unwrap();
    assert_eq!(page.title, "Fantasy");
    assert_eq!(page.loaded(), 2, "its tracks, in the service's order");
    app.handle_key(key(KeyCode::Char('h')));
    assert_eq!(app.page().unwrap().title, "Tidal playlists");

    app.handle_key(key(KeyCode::Char(']')));
    reply_with(&mut app, remote_reply);
    app.handle_key(key(KeyCode::Enter));
    let asked = reply_with(&mut app, remote_reply);
    assert!(matches!(&asked[..],
        [ClientMessage::Mix { service: Some(Service::Tidal), mix }] if mix == "0001"));
    assert_eq!(app.page().unwrap().title, "My Mix 1");
}

#[test]
fn a_remote_list_is_queued_whole_by_the_same_keys_as_anything_else() {
    let mut app = connected(Instant::now());
    app.handle_key(key(KeyCode::Char('3')));
    app.handle_key(key(KeyCode::Char(']')));
    reply_with(&mut app, remote_reply);
    app.handle_key(key(KeyCode::Char('a')));
    let sent = reply_with(&mut app, remote_reply);
    match &sent[..] {
        [ClientMessage::QueueAdd { items, .. }] => assert!(matches!(
            &items[..],
            [ItemRef::Service {
                kind: EntityKind::Playlist,
                ..
            }]
        )),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        app.notice.as_deref(),
        Some("added \"Fantasy\" to the queue")
    );

    app.handle_key(key(KeyCode::Char('*')));
    assert!(app.take_requests().is_empty(), "there is nothing to save");
    assert_eq!(
        app.notice.as_deref(),
        Some("a playlist isn't saved; copy it in with c instead")
    );
}

#[test]
fn copying_a_service_playlist_makes_a_canon_one_of_the_same_name() {
    let mut app = connected(Instant::now());
    app.handle_key(key(KeyCode::Char('3')));
    app.handle_key(key(KeyCode::Char(']')));
    reply_with(&mut app, remote_reply);

    // On the listing: the row under the cursor.
    app.handle_key(key(KeyCode::Char('c')));
    let sent = reply_with(&mut app, remote_reply);
    match &sent[..] {
        [ClientMessage::PlaylistCreate { name, items }] => {
            assert_eq!(name, "Fantasy");
            assert!(matches!(
                &items[..],
                [ItemRef::Service {
                    kind: EntityKind::Playlist,
                    ..
                }]
            ));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        app.notice.as_deref(),
        Some("copied \"Fantasy\" into a new playlist")
    );

    // Inside it, with the cursor on a track: the page's own list, not that one track.
    app.handle_key(key(KeyCode::Enter));
    reply_with(&mut app, remote_reply);
    app.handle_key(key(KeyCode::Char('j')));
    app.handle_key(key(KeyCode::Char('c')));
    let sent = reply_with(&mut app, remote_reply);
    match &sent[..] {
        [ClientMessage::PlaylistCreate { name, items }] => {
            assert_eq!(name, "Fantasy");
            assert!(matches!(
                &items[..],
                [ItemRef::Service {
                    kind: EntityKind::Playlist,
                    ..
                }]
            ));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn merging_picks_its_target_from_canons_own_playlists() {
    let mut app = connected(Instant::now());
    app.handle_key(key(KeyCode::Char('3')));
    app.handle_key(key(KeyCode::Char(']')));
    app.handle_key(key(KeyCode::Char(']')));
    reply_with(&mut app, remote_reply);
    assert_eq!(app.page().unwrap().title, "Tidal mixes");

    // M holds the mix and shows canon's own playlists to choose from.
    app.handle_key(key(KeyCode::Char('M')));
    let asked = reply_with(&mut app, remote_reply);
    assert!(matches!(
        asked[..],
        [ClientMessage::Library {
            kind: EntityKind::Playlist,
            ..
        }]
    ));
    assert_eq!(app.page().unwrap().title, "Playlists");
    assert_eq!(
        app.notice.as_deref(),
        Some("merging \"My Mix 1\": enter on a playlist to take it, esc to cancel")
    );

    // Enter takes that playlist as the target instead of opening it.
    app.handle_key(key(KeyCode::Enter));
    let sent = reply_with(&mut app, remote_reply);
    match &sent[..] {
        [
            ClientMessage::PlaylistAdd {
                items,
                at: None,
                merge: true,
                ..
            },
        ] => assert!(matches!(&items[..], [ItemRef::Mix { mix, .. }] if mix == "0001")),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        app.notice.as_deref(),
        Some("merged \"My Mix 1\" into \"Jazz practice\"")
    );
    assert!(app.merging.is_none());

    // The picker is read again on the spot, so the target's new length shows (canon-28ff).
    assert!(matches!(
        reply_with(&mut app, remote_reply)[..],
        [ClientMessage::Library {
            kind: EntityKind::Playlist,
            offset: 0,
            ..
        }]
    ));

    // With nothing held, enter opens the playlist as it always did.
    app.handle_key(key(KeyCode::Enter));
    assert!(matches!(
        reply_with(&mut app, remote_reply)[..],
        [ClientMessage::Playlist { .. }]
    ));
}

#[test]
fn escape_gives_up_on_a_merge_before_it_means_back() {
    let mut app = connected(Instant::now());
    app.handle_key(key(KeyCode::Char('3')));
    app.handle_key(key(KeyCode::Char(']')));
    reply_with(&mut app, remote_reply);
    app.handle_key(key(KeyCode::Enter)); // into the Tidal playlist
    reply_with(&mut app, remote_reply);

    app.handle_key(key(KeyCode::Char('M')));
    reply_with(&mut app, remote_reply);
    assert!(app.merging.is_some(), "the whole playlist, from inside it");

    app.handle_key(key(KeyCode::Esc));
    assert!(app.merging.is_none());
    assert_eq!(app.notice.as_deref(), Some("merge cancelled"));
    assert_eq!(
        app.page().unwrap().title,
        "Playlists",
        "still on the picker, which esc now leaves alone"
    );
}

#[test]
fn a_search_lists_the_playlists_it_found_alongside_the_rest() {
    let mut app = connected(Instant::now());
    app.handle_key(key(KeyCode::Char('/')));
    app.handle_key(key(KeyCode::Char('x')));
    app.handle_key(key(KeyCode::Enter));
    reply_with(&mut app, |_| {
        ReplyData::Search(SearchView {
            tracks: vec![track_view("Era", "Opeth", false)],
            playlists: vec![service_playlist("Jazz-ish", "e7c9…02", 130)],
            ..SearchView::default()
        })
    });
    let page = app.page().unwrap();
    assert!(matches!(&page.rows[2], Row::Heading(h) if h == "Playlists"));

    // A found playlist opens the same way one listed from the service does.
    app.handle_key(key(KeyCode::Char('j')));
    app.handle_key(key(KeyCode::Enter));
    let asked = reply_with(&mut app, remote_reply);
    assert!(matches!(asked[..], [ClientMessage::ServicePlaylist { .. }]));
    assert_eq!(app.page().unwrap().title, "Jazz-ish");
}

// --- canon's own playlists: copy, rename, delete, and the refresh after each ---

/// A daemon that keeps canon's own playlists and *does* the playlist ops asked of it, so what a
/// reload brings back is what actually changed (canon-28ff) rather than a scripted constant.
struct Mine(std::cell::RefCell<Vec<PlaylistView>>);

impl Mine {
    fn new(names: &[&str]) -> Self {
        let kept = names
            .iter()
            .map(|name| PlaylistView {
                id: EntityId::new(),
                name: (*name).into(),
                track_count: 5,
                updated_at: 0,
            })
            .collect();
        Self(std::cell::RefCell::new(kept))
    }

    fn names(&self) -> Vec<String> {
        self.0.borrow().iter().map(|p| p.name.clone()).collect()
    }

    fn reply(&self) -> impl Fn(&ClientMessage) -> ReplyData + '_ {
        move |message| {
            let mut mine = self.0.borrow_mut();
            let named =
                |mine: &[PlaylistView], id: &EntityId| mine.iter().find(|p| p.id == *id).cloned();
            match message {
                ClientMessage::Library {
                    kind: EntityKind::Playlist,
                    ..
                } => ReplyData::Library(LibraryPage {
                    total: mine.len(),
                    playlists: mine.clone(),
                    ..LibraryPage::default()
                }),
                ClientMessage::Playlist { playlist } => match named(&mine, playlist) {
                    Some(playlist) => ReplyData::Playlist(PlaylistDetail {
                        playlist,
                        tracks: vec![track_view("Take Five", "The Dave Brubeck Quartet", true)],
                    }),
                    None => ReplyData::Ack,
                },
                ClientMessage::PlaylistCreate { name, .. } => {
                    mine.push(PlaylistView {
                        id: EntityId::new(),
                        name: name.clone(),
                        track_count: 5,
                        updated_at: 0,
                    });
                    ReplyData::Ack
                }
                ClientMessage::PlaylistRename { playlist, name } => {
                    if let Some(found) = mine.iter_mut().find(|p| p.id == *playlist) {
                        found.name = name.clone();
                    }
                    ReplyData::Ack
                }
                ClientMessage::PlaylistDelete { playlist } => {
                    mine.retain(|p| p.id != *playlist);
                    ReplyData::Ack
                }
                _ => ReplyData::Ack,
            }
        }
    }
}

/// The Playlists tab on canon's own shelf, listing `mine`.
fn on_my_playlists(mine: &Mine) -> App {
    let mut app = connected(Instant::now());
    app.handle_key(key(KeyCode::Char('3')));
    reply_with(&mut app, mine.reply());
    app
}

/// What the page on screen shows, row by row.
fn listed(app: &App) -> Vec<String> {
    app.page()
        .unwrap()
        .rows
        .iter()
        .filter_map(|row| match row {
            Row::Item(item) => Some(item.name().to_owned()),
            Row::Heading(_) => None,
        })
        .collect()
}

fn typed(app: &mut App, text: &str) {
    for c in text.chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
}

#[test]
fn duplicating_one_of_canons_own_playlists_asks_what_to_call_the_copy() {
    let mine = Mine::new(&["Road trip", "Jazz practice"]);
    let mut app = on_my_playlists(&mine);

    app.handle_key(key(KeyCode::Char('c')));
    assert!(
        app.take_requests().is_empty(),
        "nothing is created until it has a name"
    );
    let prompt = app.prompt.as_ref().expect("the prompt is open");
    assert_eq!(prompt.label(), "copy as");
    assert_eq!(
        prompt.text, "Road trip copy",
        "pre-filled, so enter alone no longer makes a second \"Road trip\""
    );
    assert!(draw(&app, 90, 14).contains("copy as: Road trip copy"));

    // Edited, then taken.
    for _ in 0.."copy".len() {
        app.handle_key(key(KeyCode::Backspace));
    }
    typed(&mut app, "again");
    app.handle_key(key(KeyCode::Enter));
    match &reply_with(&mut app, mine.reply())[..] {
        [ClientMessage::PlaylistCreate { name, items }] => {
            assert_eq!(name, "Road trip again");
            assert!(matches!(items[..], [ItemRef::Entity { .. }]));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        app.notice.as_deref(),
        Some("copied into \"Road trip again\"")
    );

    // And the listing shows it without the tab being left and re-entered (canon-28ff).
    reply_with(&mut app, mine.reply());
    assert_eq!(
        listed(&app),
        ["Road trip", "Jazz practice", "Road trip again"]
    );
}

#[test]
fn copying_a_service_playlist_still_takes_its_name_without_asking() {
    let mut app = connected(Instant::now());
    app.handle_key(key(KeyCode::Char('3')));
    app.handle_key(key(KeyCode::Char(']')));
    reply_with(&mut app, remote_reply);

    app.handle_key(key(KeyCode::Char('c')));
    assert!(app.prompt.is_none(), "upstream already named it");
    assert!(matches!(
        &reply_with(&mut app, remote_reply)[..],
        [ClientMessage::PlaylistCreate { name, .. }] if name == "Fantasy"
    ));
    assert!(
        app.take_requests().is_empty(),
        "and a service's own listing is not worth re-asking for"
    );
}

#[test]
fn renaming_a_playlist_shows_the_new_name_where_it_is() {
    let mine = Mine::new(&["Road trip", "Jazz practice"]);
    let mut app = on_my_playlists(&mine);
    app.handle_key(key(KeyCode::Char('j')));

    app.handle_key(key(KeyCode::Char('R')));
    let prompt = app.prompt.as_ref().expect("the prompt is open");
    assert_eq!(prompt.label(), "rename to");
    assert_eq!(
        prompt.text, "Jazz practice",
        "starting from the name it has"
    );
    for _ in 0.."practice".len() {
        app.handle_key(key(KeyCode::Backspace));
    }
    typed(&mut app, "hour");
    app.handle_key(key(KeyCode::Enter));
    match &reply_with(&mut app, mine.reply())[..] {
        [ClientMessage::PlaylistRename { name, .. }] => assert_eq!(name, "Jazz hour"),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        app.notice.as_deref(),
        Some("renamed \"Jazz practice\" to \"Jazz hour\"")
    );

    reply_with(&mut app, mine.reply());
    assert_eq!(listed(&app), ["Road trip", "Jazz hour"]);
    assert_eq!(app.page().unwrap().cursor, 1, "left on the row it renamed");
}

#[test]
fn a_playlist_renamed_from_inside_retitles_the_page() {
    let mine = Mine::new(&["Road trip"]);
    let mut app = on_my_playlists(&mine);
    app.handle_key(key(KeyCode::Enter));
    reply_with(&mut app, mine.reply());
    assert_eq!(app.page().unwrap().title, "Road trip");

    // The cursor is on a track, so `R` takes the playlist being shown, as `c` and `M` do.
    app.handle_key(key(KeyCode::Char('R')));
    typed(&mut app, " 2026");
    app.handle_key(key(KeyCode::Enter));
    assert!(matches!(
        &reply_with(&mut app, mine.reply())[..],
        [ClientMessage::PlaylistRename { name, .. }] if name == "Road trip 2026"
    ));
    reply_with(&mut app, mine.reply());
    assert_eq!(app.page().unwrap().title, "Road trip 2026");
}

#[test]
fn deleting_a_playlist_takes_two_presses_and_escape_calls_it_off() {
    let mine = Mine::new(&["Road trip", "Jazz practice"]);
    let mut app = on_my_playlists(&mine);

    // Once: armed, and said so. Nothing has been asked of the daemon.
    app.handle_key(key(KeyCode::Char('D')));
    assert!(app.take_requests().is_empty());
    assert_eq!(
        app.notice.as_deref(),
        Some("delete \"Road trip\" and its history? D again to confirm, esc to cancel")
    );
    app.handle_key(key(KeyCode::Esc));
    assert!(app.deleting.is_none());
    assert_eq!(app.notice.as_deref(), Some("delete cancelled"));
    assert!(app.take_requests().is_empty());
    assert_eq!(mine.names(), ["Road trip", "Jazz practice"]);

    // Armed on one row, then moved to another: the second press arms that one instead.
    app.handle_key(key(KeyCode::Char('D')));
    app.handle_key(key(KeyCode::Char('j')));
    app.handle_key(key(KeyCode::Char('D')));
    assert!(app.take_requests().is_empty(), "a different playlist");
    assert_eq!(
        app.notice.as_deref(),
        Some("delete \"Jazz practice\" and its history? D again to confirm, esc to cancel")
    );

    // Twice over, and it goes — and goes from the listing too (canon-28ff).
    app.handle_key(key(KeyCode::Char('D')));
    assert!(matches!(
        &reply_with(&mut app, mine.reply())[..],
        [ClientMessage::PlaylistDelete { .. }]
    ));
    assert_eq!(app.notice.as_deref(), Some("deleted \"Jazz practice\""));
    assert_eq!(mine.names(), ["Road trip"]);
    reply_with(&mut app, mine.reply());
    assert_eq!(listed(&app), ["Road trip"]);
    assert_eq!(
        app.page().unwrap().cursor,
        0,
        "the cursor falls back to the row above the one that went"
    );
}

#[test]
fn only_canons_own_playlists_can_be_renamed_or_deleted() {
    let mut app = connected(Instant::now());
    app.handle_key(key(KeyCode::Char('3')));
    app.handle_key(key(KeyCode::Char(']')));
    reply_with(&mut app, remote_reply);
    assert_eq!(app.page().unwrap().title, "Tidal playlists");

    app.handle_key(key(KeyCode::Char('D')));
    assert!(app.deleting.is_none());
    assert!(app.take_requests().is_empty());
    assert_eq!(
        app.notice.as_deref(),
        Some("only one of canon's own playlists can be deleted")
    );

    app.handle_key(key(KeyCode::Char('R')));
    assert!(app.prompt.is_none());
    assert_eq!(
        app.notice.as_deref(),
        Some("only one of canon's own playlists can be renamed")
    );
}

#[test]
fn a_new_empty_playlist_is_made_from_the_playlists_tab() {
    let mine = Mine::new(&["Road trip"]);
    let mut app = on_my_playlists(&mine);

    app.handle_key(key(KeyCode::Char('N')));
    let prompt = app.prompt.as_ref().expect("the prompt is open");
    assert_eq!(prompt.label(), "new playlist");
    assert_eq!(prompt.text, "", "nothing to start from");
    typed(&mut app, "Winter");
    app.handle_key(key(KeyCode::Enter));
    match &reply_with(&mut app, mine.reply())[..] {
        [ClientMessage::PlaylistCreate { name, items }] => {
            assert_eq!(name, "Winter");
            assert!(items.is_empty(), "empty, with nothing copied into it");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(app.notice.as_deref(), Some("made \"Winter\""));
    reply_with(&mut app, mine.reply());
    assert_eq!(listed(&app), ["Road trip", "Winter"]);
}

#[test]
fn escape_leaves_any_prompt_without_doing_it() {
    let mine = Mine::new(&["Road trip"]);
    let mut app = on_my_playlists(&mine);
    for open in [KeyCode::Char('R'), KeyCode::Char('c'), KeyCode::Char('N')] {
        app.handle_key(key(open));
        assert!(app.prompt.is_some());
        typed(&mut app, "x");
        app.handle_key(key(KeyCode::Esc));
        assert!(app.prompt.is_none());
        assert!(app.take_requests().is_empty());
    }
    // An empty answer is no answer: enter closes the prompt and asks for nothing.
    app.handle_key(key(KeyCode::Char('N')));
    app.handle_key(key(KeyCode::Enter));
    assert!(app.prompt.is_none());
    assert!(app.take_requests().is_empty());
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
    assert_eq!(
        app.take_clipboard().as_deref(),
        Some("link.tidal.com/JKMFZ"),
        "the address goes on the clipboard as the sign-in starts"
    );
    assert_eq!(app.take_clipboard(), None, "once");

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
    app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL));
    assert_eq!(
        app.take_clipboard().as_deref(),
        Some("https://accounts.spotify.com/authorize?…"),
        "ctrl-y copies the address again, and isn't typed"
    );
    app.handle_key(key(KeyCode::Enter));
    assert!(matches!(&app.take_requests()[..],
        [ClientEnvelope { message: ClientMessage::ConnectComplete { method, redirect: Some(url) }, .. }]
            if method == "spotify.web" && url == "http://127.0.0.1:8898/cb?code=x"));
}

/// A long address is drawn as whole rows of nothing but the address, so a mouse selection of
/// those rows is the address.
#[test]
fn a_sign_in_address_fills_whole_rows_with_nothing_beside_it() {
    let mut app = connected(Instant::now());
    let url = format!("https://accounts.spotify.com/authorize?{}", "x".repeat(150));
    app.login = Some(crate::setup::Login {
        method: "spotify.web".into(),
        flow: LoginFlow::Browser { url: url.clone() },
        input: String::new(),
        next_poll: None,
        interval: Duration::ZERO,
    });
    let frame = draw(&app, 80, 20);
    let rows: Vec<&str> = frame
        .lines()
        .skip_while(|row| !row.starts_with("https://"))
        .take(3)
        .collect();
    assert_eq!(rows.concat().trim_end(), url, "{frame}");
    assert_eq!(rows[0].chars().count(), 80, "{frame}");
    assert_eq!(rows[1].chars().count(), 80, "{frame}");
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

    // The Playlists tab, switched over to Tidal's own playlists.
    app.handle_key(key(KeyCode::Char('3')));
    app.handle_key(key(KeyCode::Char(']')));
    reply_with(&mut app, |_| ReplyData::ServicePlaylists {
        playlists: vec![
            service_playlist("Fantasy", "0e4a1", 250),
            service_playlist("Jazz-ish", "0e4a2", 130),
            service_playlist("Jazz practice", "0e4a3", 5),
            service_playlist("Slow and low", "0e4a4", 64),
            service_playlist("Winter", "0e4a5", 41),
        ],
    });
    save("playlists", &app);

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
            playlists: vec![
                service_playlist("This Is Opeth", "0e4a6", 50),
                service_playlist("Prog Essentials", "0e4a7", 100),
            ],
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
