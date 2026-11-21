//! The live terminal: keys, server messages and a clock tick feed the [`App`], and each wakes
//! a redraw.
//!
//! Keys are read on a thread of their own (crossterm's reads block) and handed over a channel,
//! so the loop only ever waits in one place.

use std::io::Write as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use base64::Engine as _;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind,
};
use ratatui::crossterm::execute;
use tokio::sync::mpsc;

use crate::app::{ART_SIZE, App};
use crate::link::{Endpoint, Incoming, Link, connect};
use crate::render::render;

/// How often the screen redraws with nothing happening, so the progress bar moves.
const TICK: Duration = Duration::from_millis(250);

/// Run the TUI against the daemon at `endpoint` until the user quits.
///
/// # Errors
/// The daemon couldn't be reached, or the terminal couldn't be driven.
pub async fn run(endpoint: &Endpoint) -> Result<(), String> {
    run_on(connect(endpoint).await?).await
}

/// Run the TUI over an open `link` until the user quits.
///
/// # Errors
/// The terminal couldn't be driven.
pub async fn run_on(mut link: Link) -> Result<(), String> {
    let mut app = App::new(Instant::now());
    app.load_user_keys();

    let mut terminal = ratatui::init();
    // The highlight band comes from the terminal's own colours. Asking needs raw mode, and has
    // to finish before the key reader starts, or the reader would take the reply for keys.
    toque::theme::init("CANON_THEME");
    // So is asking which graphics protocol it speaks, for album art; half-blocks if none.
    if let Ok(picker) = toque::image::Picker::from_query_stdio() {
        app.set_picker(picker);
    }
    // Clicks and the wheel (canon-2feb). Selecting text then wants the terminal's own modifier
    // (Shift in most, Option in iTerm2).
    let _ = execute!(std::io::stdout(), EnableMouseCapture);

    let (keys, mut key_events) = mpsc::unbounded_channel();
    let stop = Arc::new(AtomicBool::new(false));
    let reader = std::thread::spawn({
        let stop = Arc::clone(&stop);
        move || {
            while !stop.load(Ordering::Relaxed) {
                match event::poll(Duration::from_millis(100)) {
                    Ok(true) => match event::read() {
                        Ok(event) => {
                            if keys.send(event).is_err() {
                                return;
                            }
                        }
                        Err(_) => return,
                    },
                    Ok(false) => {}
                    Err(_) => return,
                }
            }
        }
    });

    let mut tick = tokio::time::interval(TICK);
    // Once the connection is gone its channel reports nothing more; stop asking it.
    let mut open = true;
    let result = loop {
        app.tick(Instant::now());
        if let Err(e) = terminal.draw(|frame| render(&app, frame)) {
            break Err(e.to_string());
        }
        tokio::select! {
            Some(event) = key_events.recv() => match event {
                Event::Key(key) if key.kind != KeyEventKind::Release => app.handle_key(key),
                Event::Mouse(mouse) => app.handle_mouse(mouse),
                Event::Resize(_, height) => app.resize(height),
                _ => {}
            },
            incoming = link.incoming.recv(), if open => match incoming {
                Some(Incoming::Message(message)) => app.apply(*message),
                Some(Incoming::Unreadable(what)) => app.unreadable(&what),
                Some(Incoming::Art { entity, picture }) => app.art_arrived(entity, picture),
                Some(Incoming::Closed(why)) => {
                    open = false;
                    app.lost(why);
                }
                None => {
                    open = false;
                    app.lost("the connection task ended".into());
                }
            },
            _ = tick.tick() => {}
        }
        for request in app.take_requests() {
            let _ = link.outgoing.send(request);
        }
        for entity in app.take_art_requests() {
            link.fetch_art(entity, ART_SIZE);
        }
        if let Some(text) = app.take_clipboard() {
            copy(&text);
        }
        if app.should_quit() {
            break Ok(());
        }
        if let Ok(size) = terminal.size() {
            app.resize(size.height);
        }
    };
    let _ = execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    stop.store(true, Ordering::Relaxed);
    let _ = reader.join();
    result
}

/// Put `text` on the clipboard by asking the terminal to (OSC 52), which works over ssh and needs
/// no display server. Terminals that don't allow it ignore the request; under tmux it needs
/// `set -g set-clipboard on`. There is no reply to say whether it worked.
fn copy(text: &str) {
    let encoded = base64::engine::general_purpose::STANDARD.encode(text);
    let mut out = std::io::stdout();
    let _ = write!(out, "\x1b]52;c;{encoded}\x07");
    let _ = out.flush();
}
