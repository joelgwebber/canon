//! Creating a playlist on Spotify (yak canon-8ed0): the one thing canon writes to the Web API.
//!
//! Two calls, both post-February-2026 shapes (see [`crate::catalog`]'s notes):
//!
//! * `POST /me/playlists` — `POST /users/{user_id}/playlists` was removed, and the replacement
//!   needs no account id at all.
//! * `POST /playlists/{id}/items` (was `/tracks`), a hundred URIs at a time, appending.
//!
//! The playlist is created **private**: an export is the user's own copy, not something to
//! publish on their profile by surprise. Nothing is ever updated or deleted upstream — a second
//! export of the same canon playlist makes a second Spotify playlist, which the description's
//! stamp makes easy to spot and remove by hand.

use async_trait::async_trait;
use canon_core::{Error, Exporter, Result, Service, SourceRef};
use serde::Deserialize;
use serde_json::json;

use crate::SpotifySession;
use crate::session::api_url;

/// How many URIs `POST /playlists/{id}/items` takes at once.
const ADD_BATCH: usize = 100;

#[derive(Debug, Deserialize)]
struct Created {
    id: String,
}

#[async_trait]
impl Exporter for SpotifySession {
    fn service(&self) -> Service {
        Service::Spotify
    }

    async fn create_playlist(
        &self,
        name: &str,
        description: &str,
        tracks: &[SourceRef],
    ) -> Result<SourceRef> {
        let uris: Vec<String> = tracks.iter().map(track_uri).collect::<Result<_>>()?;
        let created: Created = self
            .post(
                &api_url("/me/playlists", &[]),
                &json!({"name": name, "description": description, "public": false}),
            )
            .await?;
        let playlist = SourceRef::Spotify {
            id: created.id.clone(),
        };
        let items = api_url(&format!("/playlists/{}/items", created.id), &[]);
        for (nth, batch) in uris.chunks(ADD_BATCH).enumerate() {
            // The playlist exists from here on, so a failure part-way leaves a short one behind.
            // canon never deletes upstream (yak canon-65f7): say what is there to clean up.
            let _: serde_json::Value = self
                .post(&items, &json!({ "uris": batch }))
                .await
                .map_err(|e| partly_filled(e, name, nth * ADD_BATCH))?;
        }
        Ok(playlist)
    }
}

/// The `spotify:track:…` URI for a binding, or why it isn't one Spotify can add.
fn track_uri(source: &SourceRef) -> Result<String> {
    match source {
        SourceRef::Spotify { id } => Ok(format!("spotify:track:{id}")),
        other => Err(Error::Unsupported(format!(
            "spotify can't put a {} binding in a playlist",
            other.service()
        ))),
    }
}

/// A failure adding tracks, said in terms of what is now on Spotify.
fn partly_filled(e: Error, name: &str, added: usize) -> Error {
    Error::Source(format!(
        "spotify: \"{name}\" was created with its first {added} tracks and then adding failed \
         ({e}); delete it on Spotify and export again"
    ))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::http::HttpResponse;
    use crate::session::testing::{Scripted, signed_in};

    fn sp(id: &str) -> SourceRef {
        SourceRef::Spotify { id: id.into() }
    }

    fn create_url() -> String {
        api_url("/me/playlists", &[])
    }

    fn items_url() -> String {
        api_url("/playlists/pl-1/items", &[])
    }

    fn scripted() -> Arc<Scripted> {
        let http = Arc::new(Scripted::default());
        http.on(&create_url(), HttpResponse::new(201, r#"{"id":"pl-1"}"#))
            .on(
                &items_url(),
                HttpResponse::new(201, r#"{"snapshot_id":"s"}"#),
            );
        http
    }

    /// A create and one add: the playlist is private, carries the stamped description, and the
    /// tracks go in as `spotify:track:` URIs in the order given.
    #[tokio::test]
    async fn a_playlist_is_created_private_and_filled_in_order() {
        let http = scripted();
        let session = signed_in(http.clone()).await;
        let made = session
            .create_playlist(
                "Side two",
                "Exported from canon, 2026-09-30",
                &[sp("t1"), sp("t2")],
            )
            .await
            .unwrap();
        assert_eq!(made, sp("pl-1"));

        let created: serde_json::Value =
            serde_json::from_str(&http.bodies(&create_url())[0]).unwrap();
        assert_eq!(created["name"], "Side two");
        assert_eq!(created["description"], "Exported from canon, 2026-09-30");
        assert_eq!(created["public"], false, "an export is not published");

        let added: serde_json::Value = serde_json::from_str(&http.bodies(&items_url())[0]).unwrap();
        assert_eq!(
            added["uris"],
            serde_json::json!(["spotify:track:t1", "spotify:track:t2"])
        );
    }

    /// More than a hundred tracks go in batches, in order, with none dropped or repeated.
    #[tokio::test]
    async fn more_than_a_hundred_tracks_go_in_batches() {
        let http = scripted();
        let session = signed_in(http.clone()).await;
        let tracks: Vec<SourceRef> = (0..101).map(|n| sp(&format!("t{n}"))).collect();
        session
            .create_playlist("Long", "stamped", &tracks)
            .await
            .unwrap();

        let batches = http.bodies(&items_url());
        assert_eq!(batches.len(), 2);
        let uris: Vec<String> = batches
            .iter()
            .flat_map(|body| {
                let sent: serde_json::Value = serde_json::from_str(body).unwrap();
                sent["uris"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|u| u.as_str().unwrap().to_string())
                    .collect::<Vec<_>>()
            })
            .collect();
        assert_eq!(uris.len(), 101);
        assert_eq!(uris[0], "spotify:track:t0");
        assert_eq!(uris[100], "spotify:track:t100");
    }

    /// A binding on another service never reaches Spotify: the call is refused before anything
    /// is created, so there is nothing upstream to clean up.
    #[tokio::test]
    async fn a_foreign_binding_is_refused_before_anything_is_created() {
        let http = scripted();
        let session = signed_in(http.clone()).await;
        let error = session
            .create_playlist(
                "Mixed",
                "stamped",
                &[sp("t1"), SourceRef::Tidal { id: "9".into() }],
            )
            .await
            .unwrap_err();
        assert!(matches!(error, Error::Unsupported(_)), "{error}");
        assert!(http.bodies(&create_url()).is_empty(), "nothing was created");
    }

    /// Adding failing after the playlist exists says what is on Spotify and what to do, since
    /// canon will not delete it.
    #[tokio::test]
    async fn a_failed_add_says_what_was_left_upstream() {
        let http = Arc::new(Scripted::default());
        http.on(&create_url(), HttpResponse::new(201, r#"{"id":"pl-1"}"#))
            .on(&items_url(), HttpResponse::new(500, "boom"));
        let session = signed_in(http).await;
        let error = session
            .create_playlist("Side two", "stamped", &[sp("t1")])
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("Side two"), "{error}");
        assert!(error.contains("delete it on Spotify"), "{error}");
    }
}
