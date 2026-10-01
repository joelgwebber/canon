//! Drawing: a pure function of the [`App`] and the frame's size.
//!
//! ```text
//!  canon  1 Queue  2 Library  3 Playlists  4 Search          Tunes · connected
//!  ▶  1  The Lion's Roar              First Aid Kit                 4:05
//!     2  Kindly Bent to Free Us       Cheval Sombre                 3:12
//!  ───────────────────────────────────────────────────────────────────────
//!  ▶ The Lion's Roar — First Aid Kit · The Lion's Roar
//!    1:23 ━━━━━━━━━━━━━━━━━━━━━━━━──────────────────────────────── 4:05
//!    vol 40% · repeat off · tidal flac 16/44.1                    ? keys
//! ```

use std::cell::Cell;
use std::time::Duration;

use canon_core::{LoginFlow, OutputMode, PlaybackState, PlayingFrom, Repeat, TrackRef};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{App, Link};
use crate::browse::{Item, Page, Row, Shelf, Tab};
use crate::setup::{self, Login, SetupRow};

const DIM: Style = Style::new().fg(Color::DarkGray);
const ACCENT: Style = Style::new().fg(Color::Cyan);
/// The row under the cursor: one even band, whatever its columns' own colours (reverse video
/// turns dim columns into grey blocks).
const SELECTED: Style = Style::new().fg(Color::White).bg(Color::Indexed(238));

/// Draw the whole screen.
pub fn render(app: &App, frame: &mut Frame) {
    let [top, body, rule, now] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(3),
    ])
    .areas(frame.area());
    render_top(app, frame, top);
    match app.page() {
        Some(page) => render_page(app, page, frame, body),
        None if app.tab == Tab::Queue => render_queue(app, frame, body),
        None if app.setup_tab().is_some() => render_setup(app, frame, body),
        None => render_empty_tab(app, frame, body),
    }
    frame.render_widget(
        Paragraph::new("─".repeat(usize::from(rule.width))).style(DIM),
        rule,
    );
    render_now(app, frame, now);
    if let Some(login) = &app.login {
        render_login(login, frame);
    }
    if app.help {
        render_help(app, frame);
    }
}

fn render_top(app: &App, frame: &mut Frame, area: Rect) {
    let right_width = app.output_name().width() + " · ".len() + 14;
    let room = usize::from(area.width).saturating_sub(right_width);
    // Full labels when they fit beside the output's name, then shorter ones.
    let label = |index: usize, tab: Tab| -> [String; 3] {
        [
            format!(" {} {} ", index + 1, tab.name()),
            format!(" {} ", tab.name()),
            format!(" {} ", index + 1),
        ]
    };
    let width = |form: usize| -> usize {
        7 + Tab::ALL
            .iter()
            .enumerate()
            .map(|(i, t)| label(i, *t)[form].width())
            .sum::<usize>()
    };
    let form = (0..3).find(|form| width(*form) <= room).unwrap_or(2);
    let mut tabs = vec![Span::styled(" canon ", ACCENT.add_modifier(Modifier::BOLD))];
    for (index, tab) in Tab::ALL.iter().enumerate() {
        let text = label(index, *tab)[form].clone();
        tabs.push(if *tab == app.tab {
            Span::styled(text, Style::new().add_modifier(Modifier::REVERSED))
        } else {
            Span::styled(text, DIM)
        });
    }
    let title = Line::from(tabs);
    let (link, link_style) = match &app.link {
        Link::Connecting => ("connecting…", DIM),
        Link::Connected => ("connected", DIM),
        Link::Lost(_) => ("disconnected", Style::new().fg(Color::Red)),
    };
    let mut right = vec![
        Span::raw(app.output_name().to_owned()),
        Span::styled(" · ", DIM),
    ];
    right.push(Span::styled(link, link_style));
    right.push(Span::raw(" "));
    let right = Line::from(right).right_aligned();
    frame.render_widget(Paragraph::new(title), area);
    frame.render_widget(Paragraph::new(right), area);
}

fn render_queue(app: &App, frame: &mut Frame, area: Rect) {
    let rows = usize::from(area.height);
    if app.queue.is_empty() {
        let hint = match app.link {
            Link::Lost(_) => "",
            _ => "  The queue is empty.",
        };
        frame.render_widget(Paragraph::new(hint).style(DIM), area);
        return;
    }
    let offset = scrolled(&app.scroll, app.cursor, app.queue.len(), rows);

    let number_width = app.queue.len().to_string().len();
    let width = usize::from(area.width);
    // Marker, number, gaps and the duration column are fixed; title and artists share the rest.
    let fixed = 3 + number_width + 2 + 2 + 7;
    let flexible = width.saturating_sub(fixed);
    let title_width = flexible * 11 / 20;
    let artist_width = flexible - title_width;

    let lines: Vec<Line> = app
        .queue
        .iter()
        .enumerate()
        .skip(offset)
        .take(rows)
        .map(|(index, track)| {
            let current = index == app.snapshot.queue.index && app.snapshot.track.is_some();
            let marker = if current {
                match app.snapshot.state {
                    PlaybackState::Paused => " ❚❚",
                    _ => " ▶ ",
                }
            } else {
                "   "
            };
            let mut style = Style::new();
            if current {
                style = style.add_modifier(Modifier::BOLD).fg(Color::Cyan);
            }
            let duration = track
                .meta
                .duration_ms
                .map(|ms| clock(Duration::from_millis(ms)))
                .unwrap_or_default();
            let line = Line::from(vec![
                Span::raw(marker),
                Span::styled(format!("{:>number_width$}  ", index + 1), DIM),
                Span::raw(fit(&track.meta.title, title_width)),
                Span::raw("  "),
                Span::styled(fit(&artists(track), artist_width), DIM),
                Span::raw(format!("{duration:>7}")),
            ])
            .style(style);
            if index == app.cursor {
                line.patch_style(SELECTED)
            } else {
                line
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

/// The first row to show so `cursor` is in view, scrolling only as far as needed from where the
/// last frame left off.
fn scrolled(scroll: &Cell<usize>, cursor: usize, len: usize, rows: usize) -> usize {
    let mut offset = scroll.get().min(len.saturating_sub(1));
    if cursor < offset {
        offset = cursor;
    } else if rows > 0 && cursor >= offset + rows {
        offset = cursor + 1 - rows;
    }
    scroll.set(offset);
    offset
}

/// A browsing tab with nothing to show yet: search, before anything has been asked.
fn render_empty_tab(app: &App, frame: &mut Frame, area: Rect) {
    let [heading, _] = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);
    frame.render_widget(Paragraph::new(prompt_line(app, "")), heading);
}

/// The prompt, while it has the keys — saying what it is asking for, since by now it asks for
/// more than a search (canon-4c6e) — else the page's own heading.
fn prompt_line<'a>(app: &App, heading: &'a str) -> Line<'a> {
    match &app.prompt {
        Some(prompt) => Line::from(vec![
            Span::styled(format!("  {}: ", prompt.label()), ACCENT),
            Span::raw(prompt.text.clone()),
            Span::styled("▏", ACCENT),
        ]),
        None if heading.is_empty() => Line::from(Span::styled("  / to search", DIM)),
        None => Line::from(Span::styled(
            heading.to_owned(),
            Style::new().add_modifier(Modifier::BOLD),
        )),
    }
}

fn render_page(app: &App, page: &Page, frame: &mut Frame, area: Rect) {
    let [heading, list] = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);
    let stack = app.tab.stack().map_or(&[][..], |s| &app.stacks[s][..]);
    let mut trail: Vec<String> = stack.iter().map(|page| page.title.clone()).collect();
    if let (Some(total), Some(last)) = (page.total, trail.last_mut()) {
        last.push_str(&format!(" ({total})"));
    }
    let mut crumbs = format!("  {}", trail.join(" › "));
    if app.tab == Tab::Library && stack.len() == 1 {
        crumbs.push_str("   [ ] tracks · albums · artists");
    }
    if app.tab == Tab::Playlists && stack.len() == 1 {
        let shelves: Vec<String> = Shelf::ALL.iter().map(|shelf| shelf.label()).collect();
        crumbs.push_str(&format!("   [ ] {}", shelves.join(" · ")));
    }
    frame.render_widget(Paragraph::new(prompt_line(app, &crumbs)), heading);

    if page.rows.is_empty() {
        let note = if page.loading {
            "  loading…"
        } else {
            "  nothing here"
        };
        frame.render_widget(Paragraph::new(note).style(DIM), list);
        return;
    }
    let rows = usize::from(list.height);
    let offset = scrolled(&page.scroll, page.cursor, page.rows.len(), rows);
    let width = usize::from(list.width);
    let lines: Vec<Line> = page
        .rows
        .iter()
        .enumerate()
        .skip(offset)
        .take(rows)
        .map(|(index, row)| {
            let line = match row {
                Row::Heading(text) => Line::from(Span::styled(
                    format!("  {text}"),
                    ACCENT.add_modifier(Modifier::BOLD),
                )),
                Row::Item(item) => item_line(item, width),
            };
            if index == page.cursor && app.prompt.is_none() {
                line.patch_style(SELECTED)
            } else {
                line
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), list);
}

/// One item as a row of columns: a saved mark, then what identifies it.
fn item_line(item: &Item, width: usize) -> Line<'static> {
    let mark = match item.saved() {
        Some(true) => Span::styled(" ♥ ", ACCENT),
        _ => Span::raw("   "),
    };
    // The mark, gaps and a right-hand column are fixed; the rest is shared out.
    let flexible = width.saturating_sub(3 + 2 + 2 + 7);
    match item {
        Item::Track(track) => {
            let (title, artist) = (flexible * 2 / 5, flexible * 3 / 10);
            let album = flexible - title - artist;
            let names: Vec<&str> = track.artists.iter().map(|a| a.name.as_str()).collect();
            let duration = track
                .duration_ms
                .map(|ms| clock(Duration::from_millis(ms)))
                .unwrap_or_default();
            // A track nothing streamable is bound to can't play as things stand.
            let style = if track.plays_from.is_some() {
                Style::new()
            } else {
                DIM
            };
            Line::from(vec![
                mark,
                Span::styled(fit(&track.title, title), style),
                Span::raw("  "),
                Span::styled(fit(&names.join(", "), artist), DIM),
                Span::raw("  "),
                Span::styled(
                    fit(
                        track.album.as_ref().map_or("", |a| a.name.as_str()),
                        album.saturating_sub(2),
                    ),
                    DIM,
                ),
                Span::raw(format!("{duration:>7}")),
            ])
        }
        Item::Album(album) => {
            let title = flexible * 3 / 5;
            let credit = flexible - title;
            let year = album
                .release_date
                .as_deref()
                .and_then(|date| date.get(..4))
                .unwrap_or_default();
            Line::from(vec![
                mark,
                Span::raw(fit(&album.title, title)),
                Span::raw("  "),
                Span::styled(fit(&album.credit, credit), DIM),
                Span::styled(format!("{year:>9}"), DIM),
            ])
        }
        Item::Artist(artist) => Line::from(vec![mark, Span::raw(artist.name.clone())]),
        Item::Playlist(playlist) => Line::from(vec![
            mark,
            Span::raw(fit(&playlist.name, flexible.saturating_sub(5))),
            Span::styled(format!("{:>9} tracks", playlist.track_count), DIM),
        ]),
        // A service's playlist or mix: no ♥ column to fill, since it is never a library entity.
        Item::Remote(remote) => {
            let name = flexible * 2 / 5;
            let note = flexible.saturating_sub(name + 7);
            let count = remote
                .track_count
                .map(|n| format!("{n:>9} tracks"))
                .unwrap_or_default();
            Line::from(vec![
                mark,
                Span::raw(fit(&remote.name, name)),
                Span::raw("  "),
                Span::styled(fit(&remote.note, note), DIM),
                Span::styled(format!("{count:>16}"), DIM),
            ])
        }
    }
}

fn render_now(app: &App, frame: &mut Frame, area: Rect) {
    let [what, progress, status] = Layout::vertical([Constraint::Length(1); 3]).areas(area);
    let snapshot = &app.snapshot;

    let glyph = match snapshot.state {
        PlaybackState::Playing => "▶",
        PlaybackState::Paused => "❚❚",
        PlaybackState::Loading => "…",
        PlaybackState::Error => "!",
        PlaybackState::Idle | PlaybackState::Ended => "■",
    };
    let mut line = vec![Span::styled(format!(" {glyph} "), ACCENT)];
    match &snapshot.track {
        Some(track) => {
            line.push(Span::styled(
                track.meta.title.clone(),
                Style::new().add_modifier(Modifier::BOLD),
            ));
            let credit = artists(track);
            if !credit.is_empty() {
                line.push(Span::styled(" — ", DIM));
                line.push(Span::raw(credit));
            }
            if let Some(album) = &track.meta.album {
                line.push(Span::styled(format!(" · {album}"), DIM));
            }
        }
        None => line.push(Span::styled("nothing playing", DIM)),
    }
    frame.render_widget(Paragraph::new(Line::from(line)), what);

    if snapshot.track.is_some() {
        let at = app.position();
        let total = snapshot.duration_ms.map(Duration::from_millis);
        let (left, right) = (clock(at), total.map(clock).unwrap_or_default());
        let bar_width =
            usize::from(progress.width).saturating_sub(3 + left.len() + 1 + 1 + right.len() + 1);
        let filled = match total {
            Some(total) if !total.is_zero() => {
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    clippy::cast_precision_loss
                )]
                let filled = (at.as_secs_f64() / total.as_secs_f64() * bar_width as f64) as usize;
                filled.min(bar_width)
            }
            _ => 0,
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::raw(format!("   {left} ")),
                Span::styled("━".repeat(filled), ACCENT),
                Span::styled("─".repeat(bar_width - filled), DIM),
                Span::raw(format!(" {right}")),
            ])),
            progress,
        );
    }

    let status_line = match (&app.link, &app.notice) {
        (Link::Lost(why), _) => Line::from(Span::styled(
            format!("   disconnected: {why}"),
            Style::new().fg(Color::Red),
        )),
        (_, Some(notice)) => Line::from(Span::styled(
            format!("   {notice}"),
            Style::new().fg(Color::Yellow),
        )),
        _ => {
            let volume = if snapshot.muted {
                "muted".to_owned()
            } else {
                #[allow(clippy::cast_possible_truncation)]
                let percent = (snapshot.volume * 100.0).round() as u32;
                format!("vol {percent}%")
            };
            let mut parts = vec![volume, format!("repeat {}", repeat(snapshot.queue.repeat))];
            if let Some(from) = &snapshot.playing_from {
                parts.push(playing_from(from));
            }
            Line::from(Span::styled(format!("   {}", parts.join(" · ")), DIM))
        }
    };
    frame.render_widget(Paragraph::new(status_line), status);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled("? keys ", DIM)).right_aligned()),
        status,
    );
}

/// The Outputs or Settings tab.
fn render_setup(app: &App, frame: &mut Frame, area: Rect) {
    let Some(tab) = app.setup_tab() else {
        return;
    };
    let [heading, list] = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);
    let hint = match tab {
        0 => "  Outputs   enter: play there · f: flow / standard",
        _ => "  Settings   enter: sign in / toggle · X: sign out · J K: reorder",
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            hint,
            Style::new().add_modifier(Modifier::BOLD),
        ))),
        heading,
    );
    let rows = app.setup_rows();
    if rows.is_empty() {
        frame.render_widget(Paragraph::new("  loading…").style(DIM), list);
        return;
    }
    let cursor = app.setup_cursor[tab];
    let height = usize::from(list.height);
    let offset = scrolled(&app.setup_scroll[tab], cursor, rows.len(), height);
    let width = usize::from(list.width);
    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .skip(offset)
        .take(height)
        .map(|(index, row)| {
            let line = setup_line(app, row, width);
            if index == cursor && app.login.is_none() {
                line.patch_style(SELECTED)
            } else {
                line
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), list);
}

fn setup_line(app: &App, row: &SetupRow, width: usize) -> Line<'static> {
    let here = |on: bool| {
        if on {
            Span::styled(" ● ", ACCENT)
        } else {
            Span::raw("   ")
        }
    };
    match row {
        SetupRow::Heading(text) => Line::from(Span::styled(
            format!("  {text}"),
            ACCENT.add_modifier(Modifier::BOLD),
        )),
        SetupRow::Output(sink) => {
            let on = app.is_output(&sink.id) || sink.protocols.iter().any(|p| app.is_output(&p.id));
            let protocols: Vec<&str> = if sink.protocols.is_empty() {
                vec![setup::protocol(sink.kind)]
            } else {
                sink.protocols
                    .iter()
                    .map(|p| setup::protocol(p.kind))
                    .collect()
            };
            let mode = setup::mode(app.settings.as_ref(), sink).map_or("", |mode| match mode {
                OutputMode::Flow => "flow",
                OutputMode::Standard => "standard",
            });
            let name = width.saturating_sub(3 + 16 + 10);
            Line::from(vec![
                here(on),
                Span::raw(fit(&sink.name, name)),
                Span::styled(format!("{:<16}", protocols.join(" · ")), DIM),
                Span::styled(format!("{mode:>10}"), DIM),
            ])
        }
        SetupRow::Endpoint { endpoint, .. } => Line::from(vec![
            here(app.is_output(&endpoint.id)),
            Span::styled(format!("  over {}", setup::protocol(endpoint.kind)), DIM),
        ]),
        SetupRow::Connection(connection) => {
            let state = setup::health(connection);
            let style = match connection.health {
                canon_core::Health::Ok => Style::new(),
                canon_core::Health::NeedsLogin => DIM,
                _ => Style::new().fg(Color::Yellow),
            };
            let label = (width.saturating_sub(3 + 20 + 2) / 3).min(32);
            Line::from(vec![
                Span::raw("   "),
                Span::raw(format!("{:<20}", connection.id)),
                Span::styled(fit(&connection.label, label), DIM),
                Span::raw("  "),
                Span::styled(format!("{state} · {}", setup::grants(connection)), style),
            ])
        }
        SetupRow::Order(service) => {
            let rank = app
                .settings
                .as_ref()
                .and_then(|s| s.streaming.order.iter().position(|o| o == service))
                .map_or(0, |at| at + 1);
            Line::from(vec![
                Span::styled(format!("   {rank}. "), DIM),
                Span::raw(service.to_string()),
            ])
        }
        SetupRow::Toggle(toggle, on) => Line::from(vec![
            Span::styled(if *on { " [x] " } else { " [ ] " }, ACCENT),
            Span::raw(toggle.label()),
        ]),
    }
}

/// A sign-in in progress: what to do, and the box to paste the result into.
///
/// The address spans the whole width with no border or indent beside it, cut into full rows, so
/// a plain mouse selection takes the address and nothing else (a browser drops the line breaks
/// when it is pasted). It is on the clipboard already, if the terminal allows that.
fn render_login(login: &Login, frame: &mut Frame) {
    let area = frame.area();
    let width = area.width.max(1);
    let url = crate::app::login_url(&login.flow);
    let mut lines = vec![Line::from(Span::styled(
        format!(" Signing in with {}", login.method),
        Style::new().add_modifier(Modifier::BOLD),
    ))];
    lines.push(Line::raw(match login.flow {
        LoginFlow::Browser { .. } => " Open this in a browser and sign in:",
        LoginFlow::DeviceCode { .. } => " Open this, and enter the code if asked:",
    }));
    let chars: Vec<char> = url.chars().collect();
    for row in chars.chunks(usize::from(width)) {
        lines.push(Line::from(Span::styled(
            row.iter().collect::<String>(),
            ACCENT,
        )));
    }
    lines.push(Line::from(Span::styled(
        " (copied to the clipboard, if your terminal allows it; ctrl-y copies it again)",
        DIM,
    )));
    match &login.flow {
        LoginFlow::Browser { .. } => {
            lines.push(Line::raw(
                " Then paste the address you land on here, and press enter:",
            ));
            lines.push(Line::from(vec![
                Span::raw(" > "),
                Span::raw(login.input.clone()),
                Span::styled("▏", ACCENT),
            ]));
        }
        LoginFlow::DeviceCode { code } => {
            lines.push(Line::from(vec![
                Span::raw(" code: "),
                Span::styled(code.user_code.clone(), ACCENT.add_modifier(Modifier::BOLD)),
            ]));
            lines.push(Line::from(Span::styled(" waiting for approval…", DIM)));
        }
    }
    lines.push(Line::from(Span::styled(" esc cancels", DIM)));
    let height = u16::try_from(lines.len()).unwrap_or(u16::MAX) + 2;
    let popup = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height: height.min(area.height),
    };
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::TOP | Borders::BOTTOM)),
        popup,
    );
}

/// Keys for the Outputs and Settings tabs.
const SETUP_KEYS: &[(&str, &str)] = &[
    ("j k ↑ ↓", "move"),
    ("enter", "play there / sign in / toggle"),
    ("f", "output: flow or standard mode"),
    ("J K", "streaming order: move down / up"),
    ("X", "sign out"),
];

/// Keys that work everywhere.
const GLOBAL_KEYS: &[(&str, &str)] = &[
    ("tab 1-6", "switch view"),
    ("/", "search"),
    ("space", "play / pause"),
    ("n  p", "next / previous"),
    ("← →", "seek 10s"),
    ("+ -  m", "volume, mute"),
    ("s  r", "shuffle, repeat"),
    ("q", "quit"),
];

/// Keys for the queue.
const QUEUE_KEYS: &[(&str, &str)] = &[
    ("j k ↑ ↓", "move"),
    ("g G  .", "top, bottom, playing"),
    ("enter", "play this entry"),
    ("d", "remove from the queue"),
    ("J K", "move the entry down / up"),
];

/// Keys for a list of tracks, albums, artists or playlists.
const PAGE_KEYS: &[(&str, &str)] = &[
    ("j k ↑ ↓", "move"),
    ("enter l", "play from here / open"),
    ("h esc", "back"),
    ("a", "add to the queue"),
    ("A", "play next"),
    ("P", "play now"),
    ("*", "save / unsave"),
    ("c", "copy this list into a new playlist"),
    ("M", "merge it into a playlist…"),
    ("R", "rename this playlist"),
    ("D", "delete this playlist (twice to confirm)"),
    ("N", "new empty playlist"),
    ("[ ]", "switch the listing (kinds, services)"),
];

fn render_help(app: &App, frame: &mut Frame) {
    let local = match app.tab {
        Tab::Queue => QUEUE_KEYS,
        Tab::Outputs | Tab::Settings => SETUP_KEYS,
        _ => PAGE_KEYS,
    };
    let keys: Vec<&(&str, &str)> = local.iter().chain(GLOBAL_KEYS).collect();
    let area = frame.area();
    let height = u16::try_from(keys.len()).unwrap_or(u16::MAX) + 2;
    let width = 50.min(area.width);
    let popup = Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height: height.min(area.height),
    };
    let lines: Vec<Line> = keys
        .iter()
        .map(|(keys, what)| {
            Line::from(vec![
                Span::styled(format!(" {keys:>9}  "), ACCENT),
                Span::raw(*what),
            ])
        })
        .collect();
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" keys ")),
        popup,
    );
}

fn artists(track: &TrackRef) -> String {
    track.meta.artists.join(", ")
}

fn repeat(mode: Repeat) -> &'static str {
    match mode {
        Repeat::Off => "off",
        Repeat::All => "all",
        Repeat::One => "one",
    }
}

/// Where the audio comes from, as `canon control` shows it: `tidal flac 16/44.1`.
fn playing_from(from: &PlayingFrom) -> String {
    let codec = serde_json::to_value(from.codec)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    let khz = f64::from(from.sample_rate) / 1000.0;
    match from.bit_depth {
        Some(bits) => format!("{} {codec} {bits}/{khz}", from.source.service()),
        None => format!("{} {codec} {khz}", from.source.service()),
    }
}

/// `m:ss`, or `h:mm:ss` past an hour.
fn clock(at: Duration) -> String {
    let secs = at.as_secs();
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// `text` padded or cut to `width` terminal columns, cut with an ellipsis.
fn fit(text: &str, width: usize) -> String {
    if text.width() <= width {
        return format!("{text}{}", " ".repeat(width - text.width()));
    }
    let mut cut = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if used + w + 1 > width {
            break;
        }
        cut.push(c);
        used += w;
    }
    if width > 0 {
        cut.push('…');
        used += 1;
    }
    format!("{cut}{}", " ".repeat(width.saturating_sub(used)))
}
