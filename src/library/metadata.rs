//! Tag and media-property reading. Never decodes audio; never panics.
use std::path::Path;
use std::process::{Command, Stdio};

use lofty::file::{AudioFile, TaggedFileExt};
use lofty::tag::{Accessor, ItemKey};
use serde_json::Value;

use super::{MediaKind, Track, is_media_path};

/// Best-effort track description; falls back to filename-only info.
pub fn read_track(path: &Path) -> Track {
    let mut t = Track {
        id: 0,
        path: path.to_path_buf(),
        title: path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned()),
        artist: None,
        album: None,
        album_artist: None,
        genre: None,
        track_no: None,
        year: None,
        duration: None,
        format: path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_uppercase()),
        size: std::fs::metadata(path).map(|m| m.len()).unwrap_or(0),
        width: None,
        height: None,
        is_video: is_media_path(path) == Some(MediaKind::Video),
    };
    let tagged = !t.is_video && read_with_lofty(&mut t);
    if !tagged || t.duration.is_none() || t.is_video {
        read_with_ffprobe(&mut t);
    }
    t
}

fn read_with_lofty(t: &mut Track) -> bool {
    let Ok(file) = lofty::read_from_path(&t.path) else {
        return false;
    };
    let secs = file.properties().duration().as_secs_f64();
    t.duration = (secs > 0.0).then_some(secs);
    if let Some(tag) = file.primary_tag().or_else(|| file.first_tag()) {
        let clean = |s: Option<std::borrow::Cow<str>>| {
            s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
        };
        if let Some(title) = clean(tag.title()) {
            t.title = title;
        }
        t.artist = clean(tag.artist());
        t.album = clean(tag.album());
        t.genre = clean(tag.genre());
        t.album_artist = clean(tag.get_string(ItemKey::AlbumArtist).map(Into::into));
        t.track_no = tag.track();
        t.year = tag.date().map(|d| i32::from(d.year));
    }
    true
}

/// Fill gaps from ffprobe; silently does nothing if it is missing or fails.
fn read_with_ffprobe(t: &mut Track) {
    let Ok(out) = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(&t.path)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    else {
        return;
    };
    if !out.status.success() {
        return;
    }
    let Ok(json) = serde_json::from_slice::<Value>(&out.stdout) else {
        return;
    };
    let format = &json["format"];
    if t.duration.is_none() {
        t.duration = format["duration"]
            .as_str()
            .and_then(|d| d.parse::<f64>().ok())
            .filter(|d| *d > 0.0);
    }
    if let Some(video) = json["streams"]
        .as_array()
        .and_then(|s| s.iter().find(|s| s["codec_type"] == "video"))
    {
        t.width = video["width"].as_u64().map(|w| w as u32);
        t.height = video["height"].as_u64().map(|h| h as u32);
    }
    let tag = |k: &str| {
        let tags = &format["tags"];
        tags[k]
            .as_str()
            .or_else(|| tags[k.to_ascii_uppercase()].as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
    };
    if let Some(title) = tag("title") {
        t.title = title;
    }
    t.artist = t.artist.take().or_else(|| tag("artist"));
    t.album = t.album.take().or_else(|| tag("album"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn garbage_file_falls_back_to_filename() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("My Song.mp3");
        std::fs::write(&p, b"not really audio at all").unwrap();
        let t = read_track(&p);
        assert_eq!(t.title, "My Song");
        assert_eq!(t.format.as_deref(), Some("MP3"));
        assert_eq!(t.size, 23);
        assert!(!t.is_video);
    }

    #[test]
    fn missing_file_does_not_panic() {
        let t = read_track(Path::new("/no/such/clip.mkv"));
        assert_eq!(t.title, "clip");
        assert!(t.is_video);
        assert_eq!(t.size, 0);
    }

    #[test]
    fn wav_duration() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("tone.wav");
        // 1 second of 8 kHz mono 8-bit silence
        let data = vec![128u8; 8000];
        let mut w = Vec::new();
        w.extend(b"RIFF");
        w.extend((36 + data.len() as u32).to_le_bytes());
        w.extend(b"WAVEfmt ");
        w.extend(16u32.to_le_bytes());
        w.extend(1u16.to_le_bytes()); // PCM
        w.extend(1u16.to_le_bytes()); // mono
        w.extend(8000u32.to_le_bytes());
        w.extend(8000u32.to_le_bytes()); // byte rate
        w.extend(1u16.to_le_bytes()); // block align
        w.extend(8u16.to_le_bytes()); // bits
        w.extend(b"data");
        w.extend((data.len() as u32).to_le_bytes());
        w.extend(&data);
        std::fs::write(&p, w).unwrap();
        let t = read_track(&p);
        assert_eq!(t.title, "tone");
        let d = t.duration.unwrap();
        assert!((d - 1.0).abs() < 0.05, "duration {d}");
    }
}
