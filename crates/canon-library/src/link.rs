//! Where an entity opens on a service's own site: the one place a service's URL scheme lives, so
//! no client builds a link itself.

use canon_core::SourceRef;
use canon_protocol::{EntityKind, Link};

/// The public link for each of `sources`, in order: what `kind` of entity they bind decides the
/// path. A local file has none, so it contributes nothing.
#[must_use]
pub fn links(kind: EntityKind, sources: &[SourceRef]) -> Vec<Link> {
    sources
        .iter()
        .filter_map(|source| {
            let (host, id) = match source {
                SourceRef::Tidal { id } => ("https://tidal.com/browse", id),
                SourceRef::Spotify { id } => ("https://open.spotify.com", id),
                SourceRef::Local { .. } => return None,
            };
            Some(Link {
                service: source.service(),
                url: format!("{host}/{}/{id}", kind.as_str()),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use canon_core::Service;

    use super::*;

    fn bound() -> Vec<SourceRef> {
        vec![
            SourceRef::Tidal {
                id: "55391786".into(),
            },
            SourceRef::Local {
                path: PathBuf::from("/music/a.flac"),
            },
            SourceRef::Spotify {
                id: "4uLU6hMCjMI75M1A2tKUQC".into(),
            },
        ]
    }

    #[test]
    fn each_kind_links_to_its_service_page_and_local_has_none() {
        let urls = |kind| {
            links(kind, &bound())
                .into_iter()
                .map(|link| (link.service, link.url))
                .collect::<Vec<_>>()
        };
        for kind in ["track", "album", "artist", "playlist"] {
            let parsed = EntityKind::parse(kind).unwrap();
            assert_eq!(
                urls(parsed),
                vec![
                    (
                        Service::Tidal,
                        format!("https://tidal.com/browse/{kind}/55391786")
                    ),
                    (
                        Service::Spotify,
                        format!("https://open.spotify.com/{kind}/4uLU6hMCjMI75M1A2tKUQC")
                    ),
                ]
            );
        }
        assert!(
            links(
                EntityKind::Track,
                &[SourceRef::Local {
                    path: PathBuf::from("/x.flac")
                }]
            )
            .is_empty()
        );
    }
}
