//! # canon-spotify
//!
//! The Spotify Web API for a personal development-mode app: an OAuth Authorization Code + PKCE
//! login for a client id the user registers, and a [`canon_core::Catalog`] over search, albums,
//! artists, the saved library and the user's own playlists (yak canon-8b07).
//!
//! No audio: the Web API has none. This crate is the `spotify.web` connection of
//! `docs/connections.md`; audio would come from librespot, separately.
//!
//! ```text
//! let session = SpotifySession::restore(Arc::new(ReqwestHttp::new()?), TokenStore::new(path), id).await?;
//! println!("{}", session.login_url().await?);   // open it, consent, copy the address bar
//! session.complete_login(&pasted).await?;       // the redirect URL, or its code
//! let playlists = session.playlists().await?;   // via canon_core::Catalog
//! ```
//!
//! Live use needs a Spotify developer app whose owner has Premium, with
//! [`DEFAULT_REDIRECT_URI`] (or whatever [`SpotifySession::with_redirect_uri`] is given)
//! registered on it, and each user (at most five) added to the app's allowlist.

mod auth;
mod catalog;
pub mod connector;
mod http;
mod session;
mod store;

pub use auth::{DEFAULT_REDIRECT_URI, SCOPES};
pub use connector::SpotifyConnector;
pub use http::{HttpResponse, ReqwestHttp, SpotifyHttp};
pub use session::SpotifySession;
pub use store::{PersistedTokens, TokenStore};
