//! Media library: tracks, SQLite index, scanner, metadata.
use std::path::{Path, PathBuf};

pub mod database;
pub mod metadata;
pub mod scanner;

/// One playable media file.
#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    /// DB id; 0 if not indexed.
    pub id: i64,
    pub path: PathBuf,
    /// Filename fallback already applied.
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub genre: Option<String>,
    pub track_no: Option<u32>,
    pub year: Option<i32>,
    /// Seconds.
    pub duration: Option<f64>,
    pub format: Option<String>,
    pub size: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub is_video: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Audio,
    Video,
}

const AUDIO_EXT: &[&str] = &[
    "mp3", "flac", "wav", "ogg", "oga", "opus", "aac", "m4a", "wma", "aiff", "aif", "ape", "wv",
    "mka", "mpc",
];
const VIDEO_EXT: &[&str] = &[
    "mp4", "mkv", "webm", "avi", "mov", "m4v", "wmv", "flv", "mpg", "mpeg", "ts", "ogv", "3gp",
];

/// Classify a path by (case-insensitive) extension; `None` if unsupported.
pub fn is_media_path(p: &Path) -> Option<MediaKind> {
    let ext = p.extension()?.to_str()?.to_ascii_lowercase();
    if AUDIO_EXT.contains(&ext.as_str()) {
        Some(MediaKind::Audio)
    } else if VIDEO_EXT.contains(&ext.as_str()) {
        Some(MediaKind::Video)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies() {
        assert_eq!(is_media_path(Path::new("a/B.FLAC")), Some(MediaKind::Audio));
        assert_eq!(is_media_path(Path::new("x.mkv")), Some(MediaKind::Video));
        assert_eq!(is_media_path(Path::new("x.txt")), None);
        assert_eq!(is_media_path(Path::new("noext")), None);
    }
}
