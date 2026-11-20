//! MPRIS D-Bus service (zbus blocking). Method calls only send `AppEvent::Command`;
//! property getters read a shared `PlayerSnapshot`.
use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard};

use zbus::blocking::Connection;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};

use crate::event::{AppEvent, Command};
use crate::library::Track;
use crate::playback::modes::RepeatMode;
use crate::playback::{PlayState, PlayerSnapshot};

const BUS_NAME: &str = "org.mpris.MediaPlayer2.rmp";
const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER_IFACE: &str = "org.mpris.MediaPlayer2.Player";
const NO_TRACK: &str = "/org/mpris/MediaPlayer2/TrackList/NoTrack";

type Shared = Arc<Mutex<PlayerSnapshot>>;

fn lock(s: &Shared) -> MutexGuard<'_, PlayerSnapshot> {
    s.lock().unwrap_or_else(|e| e.into_inner())
}

// ---- pure helpers ----

pub fn loop_status_str(m: RepeatMode) -> &'static str {
    match m {
        RepeatMode::Off => "None",
        RepeatMode::All => "Playlist",
        RepeatMode::One => "Track",
    }
}

pub fn parse_loop_status(s: &str) -> Option<RepeatMode> {
    match s {
        "None" => Some(RepeatMode::Off),
        "Playlist" => Some(RepeatMode::All),
        "Track" => Some(RepeatMode::One),
        _ => None,
    }
}

/// App volume 0..=130 percent -> MPRIS 0.0..=1.0 (clamped).
pub fn volume_to_mpris(v: i64) -> f64 {
    (v as f64 / 100.0).clamp(0.0, 1.0)
}

pub fn volume_from_mpris(v: f64) -> i64 {
    (v.clamp(0.0, 1.0) * 100.0).round() as i64
}

fn status_str(s: PlayState) -> &'static str {
    match s {
        PlayState::Playing => "Playing",
        PlayState::Paused => "Paused",
        PlayState::Stopped => "Stopped",
    }
}

fn track_path(t: &Track) -> String {
    format!("/org/rmp/track/{}", t.id)
}

fn secs_to_us(s: f64) -> i64 {
    (s * 1_000_000.0) as i64
}

pub fn build_metadata(snap: &PlayerSnapshot) -> HashMap<String, OwnedValue> {
    let mut m: HashMap<String, Value> = HashMap::new();
    let Some(t) = &snap.current else {
        m.insert("mpris:trackid".into(), obj_path(NO_TRACK));
        return own(m);
    };
    m.insert("mpris:trackid".into(), obj_path(&track_path(t)));
    if let Some(d) = snap.duration.or(t.duration) {
        m.insert("mpris:length".into(), Value::from(secs_to_us(d)));
    }
    m.insert("xesam:title".into(), Value::from(t.title.clone()));
    if let Some(a) = &t.artist {
        m.insert("xesam:artist".into(), Value::from(vec![a.clone()]));
    }
    if let Some(a) = &t.album {
        m.insert("xesam:album".into(), Value::from(a.clone()));
    }
    if let Some(a) = &t.album_artist {
        m.insert("xesam:albumArtist".into(), Value::from(vec![a.clone()]));
    }
    if let Some(n) = t.track_no {
        m.insert("xesam:trackNumber".into(), Value::from(n as i32));
    }
    if let Some(g) = &t.genre {
        m.insert("xesam:genre".into(), Value::from(vec![g.clone()]));
    }
    m.insert("xesam:url".into(), Value::from(file_uri(&t.path)));
    own(m)
}

/// `file://` URI with RFC 3986 percent-encoding (spaces, `#`, `%`, non-ASCII).
fn file_uri(path: &std::path::Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut out = String::from("file://");
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn obj_path(p: &str) -> Value<'static> {
    // Static fallback keeps this infallible; ids are numeric so the path is always valid.
    let path = ObjectPath::try_from(p.to_owned())
        .unwrap_or_else(|_| ObjectPath::from_static_str_unchecked(NO_TRACK));
    Value::from(path)
}

fn own(m: HashMap<String, Value<'_>>) -> HashMap<String, OwnedValue> {
    m.into_iter()
        .filter_map(|(k, v)| OwnedValue::try_from(v).ok().map(|v| (k, v)))
        .collect()
}

// ---- interfaces ----

struct Root {
    tx: Sender<AppEvent>,
}

#[zbus::interface(name = "org.mpris.MediaPlayer2")]
impl Root {
    fn raise(&self) {}

    fn quit(&self) {
        let _ = self.tx.send(AppEvent::Command(Command::Quit));
    }

    #[zbus(property)]
    fn can_quit(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_raise(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn identity(&self) -> &str {
        "rmp"
    }
    #[zbus(property)]
    fn supported_uri_schemes(&self) -> Vec<String> {
        vec!["file".into()]
    }
    #[zbus(property)]
    fn supported_mime_types(&self) -> Vec<String> {
        vec![]
    }
}

struct PlayerIface {
    tx: Sender<AppEvent>,
    state: Shared,
}

impl PlayerIface {
    fn send(&self, c: Command) {
        let _ = self.tx.send(AppEvent::Command(c));
    }
}

#[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
impl PlayerIface {
    fn play_pause(&self) {
        self.send(Command::TogglePause);
    }
    fn play(&self) {
        self.send(Command::Play);
    }
    fn pause(&self) {
        self.send(Command::Pause);
    }
    fn stop(&self) {
        self.send(Command::Stop);
    }
    fn next(&self) {
        self.send(Command::Next);
    }
    fn previous(&self) {
        self.send(Command::Prev);
    }
    fn seek(&self, offset_us: i64) {
        self.send(Command::SeekRelative(offset_us as f64 / 1e6));
    }
    fn set_position(&self, track_id: ObjectPath<'_>, position_us: i64) {
        let cur = lock(&self.state).current.as_ref().map(track_path);
        if cur.as_deref() == Some(track_id.as_str()) {
            self.send(Command::SeekAbsolute(position_us as f64 / 1e6));
        }
    }
    fn open_uri(&self, _uri: &str) {}

    #[zbus(property)]
    fn playback_status(&self) -> &str {
        status_str(lock(&self.state).state)
    }
    #[zbus(property)]
    fn loop_status(&self) -> &str {
        loop_status_str(lock(&self.state).repeat)
    }
    #[zbus(property)]
    fn set_loop_status(&self, v: &str) -> zbus::Result<()> {
        let m =
            parse_loop_status(v).ok_or_else(|| zbus::Error::Failure("bad LoopStatus".into()))?;
        self.send(Command::SetRepeat(m));
        Ok(())
    }
    #[zbus(property)]
    fn rate(&self) -> f64 {
        1.0
    }
    #[zbus(property)]
    fn set_rate(&self, _v: f64) {}
    #[zbus(property)]
    fn shuffle(&self) -> bool {
        lock(&self.state).shuffle
    }
    #[zbus(property)]
    fn set_shuffle(&self, v: bool) {
        self.send(Command::SetShuffle(v));
    }
    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedValue> {
        build_metadata(&lock(&self.state))
    }
    #[zbus(property)]
    fn volume(&self) -> f64 {
        volume_to_mpris(lock(&self.state).volume)
    }
    #[zbus(property)]
    fn set_volume(&self, v: f64) {
        self.send(Command::SetVolume(volume_from_mpris(v)));
    }
    #[zbus(property(emits_changed_signal = "false"))]
    fn position(&self) -> i64 {
        secs_to_us(lock(&self.state).position)
    }
    #[zbus(property)]
    fn minimum_rate(&self) -> f64 {
        1.0
    }
    #[zbus(property)]
    fn maximum_rate(&self) -> f64 {
        1.0
    }
    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_play(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_pause(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_seek(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_control(&self) -> bool {
        true
    }
}

// ---- handle ----

pub struct Mpris {
    conn: Connection,
    state: Shared,
}

pub fn spawn(tx: Sender<AppEvent>) -> Result<Mpris, String> {
    spawn_with_name(tx, BUS_NAME)
}

fn spawn_with_name(tx: Sender<AppEvent>, name: &str) -> Result<Mpris, String> {
    let state: Shared = Arc::default();
    let conn = zbus::blocking::connection::Builder::session()
        .and_then(|b| b.name(name.to_owned()))
        .and_then(|b| b.serve_at(PATH, Root { tx: tx.clone() }))
        .and_then(|b| {
            b.serve_at(
                PATH,
                PlayerIface {
                    tx,
                    state: state.clone(),
                },
            )
        })
        .and_then(|b| b.build())
        .map_err(|e| format!("MPRIS unavailable: {e}"))?;
    Ok(Mpris { conn, state })
}

impl Mpris {
    /// Store the snapshot and emit PropertiesChanged for changed properties.
    pub fn update(&self, snap: &PlayerSnapshot) {
        let old = std::mem::replace(&mut *lock(&self.state), snap.clone());
        let mut changed: HashMap<&str, Value> = HashMap::new();
        if old.state != snap.state {
            changed.insert("PlaybackStatus", Value::from(status_str(snap.state)));
        }
        if old.repeat != snap.repeat {
            changed.insert("LoopStatus", Value::from(loop_status_str(snap.repeat)));
        }
        if old.shuffle != snap.shuffle {
            changed.insert("Shuffle", Value::from(snap.shuffle));
        }
        if old.volume != snap.volume {
            changed.insert("Volume", Value::from(volume_to_mpris(snap.volume)));
        }
        if old.current.as_ref().map(|t| t.id) != snap.current.as_ref().map(|t| t.id)
            || old.duration != snap.duration
        {
            changed.insert("Metadata", Value::from(build_metadata(snap)));
        }
        if changed.is_empty() {
            return;
        }
        let _ = self.conn.emit_signal(
            None::<&str>,
            PATH,
            "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
            &(PLAYER_IFACE, changed, Vec::<&str>::new()),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    #[test]
    fn loop_roundtrip() {
        for m in [RepeatMode::Off, RepeatMode::All, RepeatMode::One] {
            assert_eq!(parse_loop_status(loop_status_str(m)), Some(m));
        }
        assert_eq!(parse_loop_status("x"), None);
    }

    #[test]
    fn volume_conv() {
        assert_eq!(volume_from_mpris(0.5), 50);
        assert_eq!(volume_from_mpris(2.0), 100);
        assert_eq!(volume_to_mpris(130), 1.0);
        assert_eq!(volume_to_mpris(25), 0.25);
    }

    #[test]
    fn metadata_empty_and_track() {
        let mut s = PlayerSnapshot::default();
        assert!(build_metadata(&s).contains_key("mpris:trackid"));
        s.current = Some(Track {
            id: 7,
            path: "/m/a.mp3".into(),
            title: "T".into(),
            artist: Some("A".into()),
            album: None,
            album_artist: None,
            genre: None,
            track_no: Some(3),
            year: None,
            duration: Some(2.0),
            format: None,
            size: 0,
            width: None,
            height: None,
            is_video: false,
        });
        let m = build_metadata(&s);
        assert_eq!(m.len(), 6);
        assert_eq!(
            i64::try_from(m["mpris:length"].try_clone().unwrap()).unwrap(),
            2_000_000
        );
        assert_eq!(
            String::try_from(m["xesam:url"].try_clone().unwrap()).unwrap(),
            "file:///m/a.mp3"
        );
    }

    #[test]
    fn uri_is_percent_encoded() {
        assert_eq!(
            file_uri(std::path::Path::new("/m/a b#c%é.mp3")),
            "file:///m/a%20b%23c%25%C3%A9.mp3"
        );
    }

    #[test]
    fn bus_roundtrip() {
        let name = format!("org.mpris.MediaPlayer2.rmp.test{}", std::process::id());
        let (tx, rx) = channel();
        let Ok(m) = spawn_with_name(tx, &name) else {
            eprintln!("skipping: no session bus");
            return;
        };
        let proxy = zbus::blocking::Proxy::new(&m.conn, name.as_str(), PATH, PLAYER_IFACE).unwrap();
        let snap = PlayerSnapshot {
            state: PlayState::Playing,
            ..Default::default()
        };
        m.update(&snap);
        let st: String = proxy.get_property("PlaybackStatus").unwrap();
        assert_eq!(st, "Playing");
        proxy.call_method("Next", &()).unwrap();
        assert!(matches!(
            rx.recv().unwrap(),
            AppEvent::Command(Command::Next)
        ));
    }
}
