//! The server's album art (yak canon-5a2c): the library's artwork URL for an entity, fetched
//! once, resized to one of the buckets `canon-api` serves, and kept on disk.
//!
//! `canon-api` owns the endpoint (`GET /art/<entity>?size=<px>`) and calls this through
//! [`canon_api::ArtSource`], since it links neither an HTTP client nor an image codec.
//!
//! The cache is `<state_dir>/art/<sha256(url)>-<bucket>.jpg`. It is keyed by the URL, so a
//! changed artwork URL is a different file and nothing is ever invalidated; a hit never touches
//! the network. A file is written to a temporary name and renamed into place, so a reader never
//! sees half a picture, and a failed fetch leaves nothing behind.

use std::path::{Path, PathBuf};
use std::time::Duration;

use canon_api::{ArtSource, BoxFuture};
use canon_core::{EntityId, Error, Result};
use canon_library::Library;
use canon_protocol::EntityKind;
use image::DynamicImage;
use image::codecs::jpeg::JpegEncoder;
use sha2::{Digest, Sha256};

/// The JPEG quality pictures are re-encoded at.
const JPEG_QUALITY: u8 = 85;
/// The most an original may weigh. Cover art is a few hundred kilobytes; this stops a wrong URL
/// from filling memory.
const MAX_ORIGINAL: usize = 16 * 1024 * 1024;

/// Fetches the original picture at a URL. A trait so tests need no network.
pub trait Fetch: Send + Sync {
    /// The bytes at `url`.
    ///
    /// # Errors
    /// The request failed or the server answered with an error status.
    fn fetch<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<Vec<u8>>>;
}

/// [`Fetch`] over HTTP(S).
pub struct HttpFetch {
    client: reqwest::Client,
}

impl HttpFetch {
    /// A client with a request timeout, so a stalled CDN can't hang a request.
    ///
    /// # Errors
    /// The HTTP client couldn't be built.
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent(concat!("canon/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| Error::Source(format!("art: build client: {e}")))?;
        Ok(Self { client })
    }
}

impl Fetch for HttpFetch {
    fn fetch<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<Vec<u8>>> {
        Box::pin(async move {
            let response = self
                .client
                .get(url)
                .send()
                .await
                .map_err(|e| Error::Source(format!("art: GET {url}: {e}")))?;
            let status = response.status();
            if !status.is_success() {
                return Err(Error::Source(format!("art: GET {url}: {status}")));
            }
            if response
                .content_length()
                .is_some_and(|n| n > MAX_ORIGINAL as u64)
            {
                return Err(Error::Source(format!("art: {url} is too large")));
            }
            let bytes = response
                .bytes()
                .await
                .map_err(|e| Error::Source(format!("art: read {url}: {e}")))?;
            if bytes.len() > MAX_ORIGINAL {
                return Err(Error::Source(format!("art: {url} is too large")));
            }
            Ok(bytes.to_vec())
        })
    }
}

/// Serves the library's album art, resized and cached under a directory.
pub struct ServerArt {
    library: Library,
    fetch: Box<dyn Fetch>,
    dir: PathBuf,
}

impl ServerArt {
    /// Serve the art of `library`'s entities, fetched with `fetch` and cached in `dir` (made on
    /// the first write).
    #[must_use]
    pub fn new(library: Library, fetch: Box<dyn Fetch>, dir: PathBuf) -> Self {
        Self {
            library,
            fetch,
            dir,
        }
    }

    /// The artwork URL of a track (its album's) or an album.
    async fn artwork_url(&self, entity: EntityId) -> Result<String> {
        let url = self
            .library
            .run(move |store| {
                Ok(match store.kind_of(entity)? {
                    Some(EntityKind::Track) => store
                        .track_ref(entity)?
                        .and_then(|track| track.meta.artwork_url),
                    Some(EntityKind::Album) => {
                        store.album(entity)?.and_then(|album| album.artwork_url)
                    }
                    _ => None,
                })
            })
            .await?;
        url.ok_or_else(|| Error::NotFound(format!("art for {entity}")))
    }

    async fn picture(&self, entity: EntityId, size: u32) -> Result<Vec<u8>> {
        let url = self.artwork_url(entity).await?;
        let path = cache_path(&self.dir, &url, size);
        if let Ok(hit) = tokio::fs::read(&path).await {
            tracing::debug!(%entity, size, "art: cache hit {}", path.display());
            return Ok(hit);
        }
        let original = self.fetch.fetch(&url).await?;
        let jpeg = tokio::task::spawn_blocking(move || resize(&original, size))
            .await
            .map_err(|e| Error::Source(format!("art: resize task: {e}")))??;
        if let Err(e) = store(&path, &jpeg).await {
            // The picture is good; failing to keep it only costs a refetch.
            tracing::warn!("art: couldn't cache {}: {e}", path.display());
        }
        tracing::info!(%entity, size, bytes = jpeg.len(), "art: fetched {url}, cached {}", path.display());
        Ok(jpeg)
    }
}

impl ArtSource for ServerArt {
    fn art(&self, entity: EntityId, size: u32) -> BoxFuture<'_, Result<Vec<u8>>> {
        Box::pin(self.picture(entity, size))
    }
}

/// Where the picture at `url` at `size` is kept.
fn cache_path(dir: &Path, url: &str, size: u32) -> PathBuf {
    let hash: String = Sha256::digest(url.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    dir.join(format!("{hash}-{size}.jpg"))
}

/// Write `bytes` at `path` atomically: to a temporary name beside it, then renamed over it.
async fn store(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let temp = path.with_extension(format!("tmp-{}", EntityId::new().0));
    let written = tokio::fs::write(&temp, bytes).await;
    let renamed = match written {
        Ok(()) => tokio::fs::rename(&temp, path).await,
        Err(e) => Err(e),
    };
    if renamed.is_err() {
        let _ = tokio::fs::remove_file(&temp).await;
    }
    renamed
}

/// `original` (JPEG or PNG) as a JPEG no larger than `size` on its long side, aspect kept. A
/// picture already within `size` is re-encoded, never enlarged.
fn resize(original: &[u8], size: u32) -> Result<Vec<u8>> {
    let image = image::load_from_memory(original)
        .map_err(|e| Error::Source(format!("art: not a picture canon can read: {e}")))?;
    let image = if image.width() > size || image.height() > size {
        image.thumbnail(size, size)
    } else {
        image
    };
    // JPEG has no alpha: a PNG's transparency would fail the encode.
    let rgb = DynamicImage::ImageRgb8(image.to_rgb8());
    let mut out = Vec::new();
    rgb.write_with_encoder(JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY))
        .map_err(|e| Error::Source(format!("art: encode: {e}")))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use canon_library::{Album, AlbumTrack, Store, Track};
    use image::ImageFormat;

    use super::*;

    /// A fetcher that serves one generated picture, or fails, and counts its calls.
    struct FakeFetch {
        body: Mutex<Option<Vec<u8>>>,
        calls: AtomicUsize,
    }

    impl FakeFetch {
        fn serving(width: u32, height: u32) -> Self {
            let image = DynamicImage::ImageRgb8(image::RgbImage::new(width, height));
            let mut out = std::io::Cursor::new(Vec::new());
            image.write_to(&mut out, ImageFormat::Png).unwrap();
            Self {
                body: Mutex::new(Some(out.into_inner())),
                calls: AtomicUsize::new(0),
            }
        }

        fn failing() -> Self {
            Self {
                body: Mutex::new(None),
                calls: AtomicUsize::new(0),
            }
        }
    }

    impl Fetch for std::sync::Arc<FakeFetch> {
        fn fetch<'a>(&'a self, _url: &'a str) -> BoxFuture<'a, Result<Vec<u8>>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let body = self.body.lock().unwrap().clone();
            Box::pin(async move { body.ok_or_else(|| Error::Source("art: 503".into())) })
        }
    }

    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("canon-art-{name}-{}", EntityId::new().0))
    }

    /// A library with a track on an album that has art, an album without any, and its ids:
    /// `(track on the album with art, album with art, album without art)`.
    async fn library() -> (Library, EntityId, EntityId, EntityId) {
        let library = Library::new(Store::open_in_memory().unwrap());
        let ids = library
            .run(|store| {
                let album = |title: &str, art: Option<&str>| Album {
                    title: title.into(),
                    credit: "Someone".into(),
                    artists: Vec::new(),
                    release_date: None,
                    barcode: None,
                    mbid: None,
                    group_mbid: None,
                    artwork_url: art.map(Into::into),
                };
                let with = store.add_album(&album("With", Some("https://cdn/cover.png")))?;
                let without = store.add_album(&album("Without", None))?;
                let track = store.add_track(&Track {
                    title: "Song".into(),
                    credit: "Someone".into(),
                    artists: Vec::new(),
                    duration_ms: None,
                    isrcs: Vec::new(),
                    mbid: None,
                })?;
                store.set_tracklist(
                    with,
                    &[AlbumTrack {
                        disc: 1,
                        position: 1,
                        track,
                    }],
                )?;
                Ok((track, with, without))
            })
            .await
            .unwrap();
        (library, ids.0, ids.1, ids.2)
    }

    fn dimensions(jpeg: &[u8]) -> (u32, u32) {
        assert_eq!(&jpeg[..2], [0xFF, 0xD8], "a JPEG");
        let image = image::load_from_memory(jpeg).unwrap();
        (image.width(), image.height())
    }

    fn files(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    #[tokio::test]
    async fn a_second_request_for_the_same_bucket_is_a_cache_hit() {
        let (library, track, album, _) = library().await;
        let fetch = std::sync::Arc::new(FakeFetch::serving(800, 400));
        let dir = scratch("hit");
        let art = ServerArt::new(library, Box::new(fetch.clone()), dir.clone());

        let first = art.picture(track, 160).await.unwrap();
        assert_eq!(dimensions(&first), (160, 80), "long side 160, aspect kept");
        // The album and the track share a URL, so they share the file too.
        let again = art.picture(album, 160).await.unwrap();
        assert_eq!(again, first);
        assert_eq!(fetch.calls.load(Ordering::SeqCst), 1, "no second fetch");
        assert_eq!(files(&dir).len(), 1);
    }

    #[tokio::test]
    async fn buckets_are_different_files_and_a_small_original_is_not_enlarged() {
        let (library, _, album, _) = library().await;
        let fetch = std::sync::Arc::new(FakeFetch::serving(100, 100));
        let dir = scratch("buckets");
        let art = ServerArt::new(library, Box::new(fetch.clone()), dir.clone());

        assert_eq!(dimensions(&art.picture(album, 64).await.unwrap()), (64, 64));
        assert_eq!(
            dimensions(&art.picture(album, 320).await.unwrap()),
            (100, 100),
            "never enlarged"
        );
        let names = files(&dir);
        assert_eq!(names.len(), 2, "{names:?}");
        assert!(names[0].ends_with("-320.jpg") && names[1].ends_with("-64.jpg"));
        assert_eq!(fetch.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_failed_fetch_caches_nothing() {
        let (library, track, _, _) = library().await;
        let fetch = std::sync::Arc::new(FakeFetch::failing());
        let dir = scratch("fail");
        let art = ServerArt::new(library, Box::new(fetch.clone()), dir.clone());

        let error = art.picture(track, 160).await.unwrap_err();
        assert!(matches!(error, Error::Source(_)), "{error}");
        assert!(files(&dir).is_empty(), "nothing written");

        // Not a picture is a failure too, and also caches nothing.
        *fetch.body.lock().unwrap() = Some(b"<html>nope</html>".to_vec());
        assert!(art.picture(track, 160).await.is_err());
        assert!(files(&dir).is_empty());
    }

    #[tokio::test]
    async fn unknown_entities_and_ones_without_art_are_not_found() {
        let (library, _, _, without) = library().await;
        let fetch = std::sync::Arc::new(FakeFetch::serving(10, 10));
        let art = ServerArt::new(library, Box::new(fetch.clone()), scratch("missing"));

        for entity in [without, EntityId::new()] {
            let error = art.picture(entity, 160).await.unwrap_err();
            assert!(matches!(error, Error::NotFound(_)), "{error}");
        }
        assert_eq!(fetch.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn the_cache_key_is_the_url_and_the_bucket() {
        let dir = Path::new("/c");
        let a = cache_path(dir, "https://x/a.jpg", 160);
        assert_eq!(a, cache_path(dir, "https://x/a.jpg", 160));
        assert_ne!(a, cache_path(dir, "https://x/b.jpg", 160));
        assert_ne!(a, cache_path(dir, "https://x/a.jpg", 320));
        let name = a.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(name.len(), 64 + "-160.jpg".len(), "{name}");
    }
}
