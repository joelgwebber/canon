//! The TUI driven headlessly through toque: the same [`App`] against a real daemon, with keys
//! from a script and each frame printed as text.
//!
//! Replies and snapshots arrive on their own time, so before each frame [`Headless`] *settles*:
//! it sends what the last key asked for and applies what comes back until every reply is in and
//! the connection has gone quiet. `wait <ms>` lets playback move on meanwhile.

use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::crossterm::event::KeyEvent;
use tokio::runtime::Handle;
use toque::{DriverOpts, HeadlessApp};

use crate::app::{App, Link as LinkState};
use crate::link::{Incoming, Link, connect};
use crate::render::render;

/// How long the connection must be quiet, with no replies owed, for the app to count as
/// settled.
const QUIET: Duration = Duration::from_millis(150);
/// The longest a frame waits for owed replies before drawing anyway.
const PATIENCE: Duration = Duration::from_secs(10);

/// The [`App`], its connection, and the runtime the connection runs on.
pub struct Headless {
    app: App,
    link: Link,
    runtime: Handle,
    open: bool,
}

impl Headless {
    /// Wrap an app and its connection. `runtime` runs the connection; the headless driver
    /// blocks on it, so it must not be called from inside that runtime's async context.
    #[must_use]
    pub fn new(app: App, link: Link, runtime: Handle) -> Self {
        Self {
            app,
            link,
            runtime,
            open: true,
        }
    }

    fn flush(&mut self) {
        for request in self.app.take_requests() {
            let _ = self.link.outgoing.send(request);
        }
    }

    /// Apply what arrives within `within`. Returns whether anything did.
    fn receive(&mut self, within: Duration) -> bool {
        if !self.open {
            return false;
        }
        let incoming = &mut self.link.incoming;
        match self
            .runtime
            .block_on(async { tokio::time::timeout(within, incoming.recv()).await })
        {
            Ok(Some(Incoming::Message(message))) => self.app.apply(*message),
            Ok(Some(Incoming::Unreadable(what))) => self.app.unreadable(&what),
            Ok(Some(Incoming::Closed(why))) => {
                self.open = false;
                self.app.lost(why);
            }
            Ok(None) => {
                self.open = false;
                self.app.lost("the connection task ended".into());
            }
            Err(_) => return false,
        }
        self.app.tick(Instant::now());
        true
    }
}

impl HeadlessApp for Headless {
    fn render(&self, frame: &mut Frame) {
        render(&self.app, frame);
    }

    fn handle_key(&mut self, key: KeyEvent) {
        self.app.handle_key(key);
        self.flush();
    }

    fn on_resize(&mut self, _width: u16, height: u16) {
        self.app.resize(height);
    }

    fn state_header(&self) -> String {
        self.app.state_header()
    }

    fn should_quit(&self) -> bool {
        self.app.should_quit()
    }

    fn settle(&mut self) {
        let give_up = Instant::now() + PATIENCE;
        loop {
            self.flush();
            let owed = self.app.awaiting_replies() || self.app.link == LinkState::Connecting;
            let now = Instant::now();
            if owed && now >= give_up {
                break;
            }
            let within = if owed { give_up - now } else { QUIET };
            if !self.receive(within) && !owed {
                break;
            }
            if !self.open {
                break;
            }
        }
        self.app.tick(Instant::now());
    }

    fn wait(&mut self, duration: Duration) {
        let until = Instant::now() + duration;
        while let Some(left) = until.checked_duration_since(Instant::now()) {
            if left.is_zero() {
                break;
            }
            self.flush();
            if !self.receive(left) && !self.open {
                std::thread::sleep(left);
            }
        }
        self.app.tick(Instant::now());
    }
}

/// Drive the TUI headlessly against the daemon at `addr`: toque's protocol on stdin, a text
/// frame per action on stdout. Blocks; call it off `runtime`'s async threads (for example from
/// `spawn_blocking`).
///
/// # Errors
/// The daemon couldn't be reached, or stdin/stdout failed.
pub fn run_headless(runtime: Handle, addr: &str, opts: DriverOpts) -> Result<(), String> {
    let link = runtime.block_on(connect(addr))?;
    let headless = Headless::new(App::new(Instant::now()), link, runtime);
    toque::run(headless, opts).map_err(|e| e.to_string())
}
