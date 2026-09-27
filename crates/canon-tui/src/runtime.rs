//! The live terminal: keys, server messages and a clock tick feed the [`App`], and each wakes
//! a redraw.
//!
//! Keys are read on a thread of their own (crossterm's reads block) and handed over a channel,
//! so the loop only ever waits in one place.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{self, Event, KeyEventKind};
use tokio::sync::mpsc;

use crate::app::App;
use crate::link::{Incoming, connect};
use crate::render::render;

/// How often the screen redraws with nothing happening, so the progress bar moves.
const TICK: Duration = Duration::from_millis(250);

/// Run the TUI against the daemon at `addr` until the user quits.
///
/// # Errors
/// The daemon couldn't be reached, or the terminal couldn't be driven.
pub async fn run(addr: &str) -> Result<(), String> {
    let mut link = connect(addr).await?;
    let mut app = App::new(Instant::now());

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

    let mut terminal = ratatui::init();
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
                Event::Resize(_, height) => app.resize(height),
                _ => {}
            },
            incoming = link.incoming.recv(), if open => match incoming {
                Some(Incoming::Message(message)) => app.apply(*message),
                Some(Incoming::Unreadable(what)) => app.unreadable(&what),
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
        if app.should_quit() {
            break Ok(());
        }
        if let Ok(size) = terminal.size() {
            app.resize(size.height);
        }
    };
    ratatui::restore();
    stop.store(true, Ordering::Relaxed);
    let _ = reader.join();
    result
}
