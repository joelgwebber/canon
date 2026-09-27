//! Drawing: a pure function of the [`App`] and the frame's size.
//!
//! ```text
//!  canon · queue                                          Tunes · connected
//!  ▶  1  The Lion's Roar              First Aid Kit                 4:05
//!     2  Kindly Bent to Free Us       Cheval Sombre                 3:12
//!  ───────────────────────────────────────────────────────────────────────
//!  ▶ The Lion's Roar — First Aid Kit · The Lion's Roar
//!    1:23 ━━━━━━━━━━━━━━━━━━━━━━━━──────────────────────────────── 4:05
//!    vol 40% · repeat off · tidal flac 16/44.1                    ? keys
//! ```

use std::time::Duration;

use canon_core::{PlaybackState, PlayingFrom, Repeat, TrackRef};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{App, Link};

const DIM: Style = Style::new().fg(Color::DarkGray);
const ACCENT: Style = Style::new().fg(Color::Cyan);

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
    render_queue(app, frame, body);
    frame.render_widget(
        Paragraph::new("─".repeat(usize::from(rule.width))).style(DIM),
        rule,
    );
    render_now(app, frame, now);
    if app.help {
        render_help(frame);
    }
}

fn render_top(app: &App, frame: &mut Frame, area: Rect) {
    let title = Line::from(vec![
        Span::styled(" canon", ACCENT.add_modifier(Modifier::BOLD)),
        Span::styled(" · queue", DIM),
    ]);
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
    // Scroll only as far as needed to keep the cursor in view.
    let mut offset = app.scroll.get().min(app.queue.len().saturating_sub(1));
    if app.cursor < offset {
        offset = app.cursor;
    } else if app.cursor >= offset + rows {
        offset = app.cursor + 1 - rows;
    }
    app.scroll.set(offset);

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
            if index == app.cursor {
                style = style.add_modifier(Modifier::REVERSED);
            }
            let duration = track
                .meta
                .duration_ms
                .map(|ms| clock(Duration::from_millis(ms)))
                .unwrap_or_default();
            Line::from(vec![
                Span::raw(marker),
                Span::styled(format!("{:>number_width$}  ", index + 1), DIM),
                Span::raw(fit(&track.meta.title, title_width)),
                Span::raw("  "),
                Span::styled(fit(&artists(track), artist_width), DIM),
                Span::raw(format!("{duration:>7}")),
            ])
            .style(style)
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
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

/// The key reference.
const KEYS: &[(&str, &str)] = &[
    ("space", "play / pause"),
    ("n  p", "next / previous"),
    ("← →", "seek 10s"),
    ("+ -  m", "volume, mute"),
    ("s  r", "shuffle, repeat"),
    ("j k ↑ ↓", "move"),
    ("g G  .", "top, bottom, playing"),
    ("enter", "play this entry"),
    ("d", "remove from the queue"),
    ("J K", "move the entry down / up"),
    ("q", "quit"),
];

fn render_help(frame: &mut Frame) {
    let area = frame.area();
    let height = u16::try_from(KEYS.len()).unwrap_or(u16::MAX) + 2;
    let width = 44.min(area.width);
    let popup = Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height: height.min(area.height),
    };
    let lines: Vec<Line> = KEYS
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
