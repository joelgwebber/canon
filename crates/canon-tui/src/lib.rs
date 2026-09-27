//! canon-tui — the interactive terminal client (yak canon-3db9).
//!
//! One [`App`] holds the state and behaviour with no I/O; [`render`] draws it. Two drivers run
//! it: [`run`], the live terminal, and [`run_headless`], which reads toque's line protocol
//! (`key space`, `wait 2000`, `snapshot`) and prints each frame as text, so an agent or a test
//! can drive the same client a person would.

mod app;
mod browse;
mod headless;
mod link;
mod render;
mod runtime;
mod setup;

pub use app::App;
pub use headless::{Headless, run_headless};
pub use link::{Incoming, Link, connect};
pub use render::render;
pub use runtime::run;

#[cfg(test)]
mod tests;
