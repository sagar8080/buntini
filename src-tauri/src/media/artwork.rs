//! Covers for the now-playing card, as files the island can draw. Music and TV hand over
//! the cover they keep; Spotify's comes from its image server over HTTPS. A browser's isn't
//! readable, so its card keeps the placeholder; another player's shows the app's own icon.
//! Only the last few are kept, and the folder
//! starts empty each time Buntini does.
use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};

use super::player::{Player, Reading};

/// Covers kept at once.
const KEEP: usize = 8;
/// A cover bigger than this isn't one.
const MAX_BYTES: usize = 4 << 20;

/// Empties the folder: covers from before don't outlive the run that showed them.
pub fn reset(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::create_dir_all(dir);
}

/// The cover for what `reading` plays, fetched or saved once and then reused.
pub async fn cover(dir: &Path, reading: &Reading) -> Option<PathBuf> {
    let key = match (reading.player, &reading.artwork_url) {
        // Nothing is asked of an app that macOS says plays; the card shows the app instead.
        (Player::Browser | Player::Other, _) => return None,
        (Player::Spotify, Some(url)) => hash(url),
        (Player::Spotify, None) => return None,
        (player, None) => hash(&format!("{}\u{1f}{}", player.id(), reading.track)),
        (_, Some(_)) => return None,
    };
    if let Some(found) = existing(dir, &key) {
        // Newest again, so pruning keeps it.
        let _ = std::fs::File::options().write(true).open(&found).and_then(|file| file.set_modified(std::time::SystemTime::now()));
        return Some(found);
    }
    std::fs::create_dir_all(dir).ok()?;
    let part = dir.join(format!("{key}.part"));
    let saved = match (reading.player, &reading.artwork_url) {
        (Player::Spotify, Some(url)) => download(url).await.and_then(|bytes| std::fs::write(&part, bytes).ok()).is_some(),
        (player, _) => super::player::save_artwork(player, &part).await.unwrap_or(false),
    };
    if !saved {
        let _ = std::fs::remove_file(&part);
        return None;
    }
    let kind = std::fs::File::open(&part).ok().and_then(|mut file| {
        use std::io::Read;
        let mut head = [0u8; 12];
        file.read_exact(&mut head).ok()?;
        image_kind(&head)
    });
    let Some(kind) = kind.filter(|_| std::fs::metadata(&part).is_ok_and(|meta| meta.len() as usize <= MAX_BYTES)) else {
        let _ = std::fs::remove_file(&part);
        return None;
    };
    let path = dir.join(format!("{key}.{kind}"));
    std::fs::rename(&part, &path).ok()?;
    prune(dir, &path);
    Some(path)
}

fn hash(text: &str) -> String {
    Sha256::digest(text.as_bytes()).iter().take(10).map(|byte| format!("{byte:02x}")).collect()
}

fn existing(dir: &Path, key: &str) -> Option<PathBuf> {
    ["jpg", "png", "tiff", "gif", "webp"].iter().map(|kind| dir.join(format!("{key}.{kind}"))).find(|path| path.is_file())
}

/// What the first bytes say the picture is; anything else isn't kept.
pub fn image_kind(head: &[u8]) -> Option<&'static str> {
    match head {
        [0xFF, 0xD8, 0xFF, ..] => Some("jpg"),
        [0x89, b'P', b'N', b'G', ..] => Some("png"),
        [b'I', b'I', 0x2A, 0x00, ..] | [b'M', b'M', 0x00, 0x2A, ..] => Some("tiff"),
        [b'G', b'I', b'F', b'8', ..] => Some("gif"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("webp"),
        _ => None,
    }
}

/// Spotify's own image server, over HTTPS, and nowhere else.
pub fn spotify_image_host(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else { return false };
    let host = parsed.host_str().unwrap_or_default();
    parsed.scheme() == "https"
        && parsed.port().is_none()
        && parsed.username().is_empty()
        && (host == "i.scdn.co" || host.ends_with(".scdn.co") || host.ends_with(".spotifycdn.com"))
}

async fn download(url: &str) -> Option<Vec<u8>> {
    if !spotify_image_host(url) {
        return None;
    }
    let client = crate::web::private_client_builder().timeout(Duration::from_secs(12)).build().ok()?;
    let mut response = client.get(url).send().await.ok()?.error_for_status().ok()?;
    if response.content_length().is_some_and(|length| length as usize > MAX_BYTES) {
        return None;
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        bytes.extend_from_slice(&chunk);
        if bytes.len() > MAX_BYTES {
            return None;
        }
    }
    image_kind(&bytes).map(|_| bytes)
}

/// Keeps the newest few covers.
fn prune(dir: &Path, keep: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path != keep && path.extension().is_some_and(|ext| ext != "part"))
        .filter_map(|path| Some((std::fs::metadata(&path).ok()?.modified().ok()?, path)))
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, path) in files.into_iter().skip(KEEP - 1) {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_spotifys_image_server_is_asked() {
        assert!(spotify_image_host("https://i.scdn.co/image/ab67616d0000b273"));
        assert!(spotify_image_host("https://mosaic.scdn.co/640/abc"));
        assert!(spotify_image_host("https://image-cdn-ak.spotifycdn.com/image/ab67"));
        assert!(!spotify_image_host("http://i.scdn.co/image/ab67"));
        assert!(!spotify_image_host("https://i.scdn.co.example.com/image"));
        assert!(!spotify_image_host("https://example.com/i.scdn.co"));
        assert!(!spotify_image_host("https://user@i.scdn.co/image"));
        assert!(!spotify_image_host("https://i.scdn.co:8443/image"));
        assert!(!spotify_image_host("file:///etc/passwd"));
    }

    #[test]
    fn a_file_is_kept_only_when_it_is_a_picture() {
        assert_eq!(image_kind(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0, 0, 0, 0, 0, 0, 0]), Some("jpg"));
        assert_eq!(image_kind(b"\x89PNG\r\n\x1a\n\0\0\0\0"), Some("png"));
        assert_eq!(image_kind(b"RIFF\0\0\0\0WEBP"), Some("webp"));
        assert_eq!(image_kind(b"<html><body>"), None);
    }

    /// No script goes to an app named after the player, which macOS would look for.
    #[tokio::test]
    async fn nothing_is_asked_of_an_app_macos_says_plays() {
        let dir = tempfile::tempdir().unwrap();
        for (player, app) in [(Player::Browser, "com.google.Chrome"), (Player::Other, "com.apple.podcasts")] {
            let reading = Reading {
                player,
                playing: true,
                title: "The Daily".into(),
                artist: None,
                album: None,
                duration: None,
                position: None,
                shuffle: None,
                repeat: None,
                volume: None,
                artwork_url: Some("https://i.scdn.co/image/ab67".into()),
                track: "t".into(),
                at: 0.0,
                app: Some(app.into()),
                previous: None,
                next: None,
            };
            assert_eq!(cover(dir.path(), &reading).await, None);
        }
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn only_the_newest_covers_stay() {
        let dir = tempfile::tempdir().unwrap();
        let mut last = PathBuf::new();
        for n in 0..12 {
            last = dir.path().join(format!("{n:02}.jpg"));
            std::fs::write(&last, [0xFF, 0xD8, 0xFF]).unwrap();
            let at = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_000 + n);
            std::fs::File::options().write(true).open(&last).unwrap().set_modified(at).unwrap();
        }
        prune(dir.path(), &last);
        let mut left: Vec<String> = std::fs::read_dir(dir.path()).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, ["04.jpg", "05.jpg", "06.jpg", "07.jpg", "08.jpg", "09.jpg", "10.jpg", "11.jpg"]);
    }
}
