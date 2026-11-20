//! One mpv process controlled over its JSON IPC unix socket.
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

use crate::event::{AppEvent, EndReason, MpvEvent};

const CONNECT_ATTEMPTS: u32 = 100;
const CONNECT_BACKOFF: Duration = Duration::from_millis(30);
/// A wedged mpv must not hang the UI thread on an IPC write.
const WRITE_TIMEOUT: Duration = Duration::from_millis(500);
static SOCKET_SEQ: AtomicU32 = AtomicU32::new(0);

/// Serialize one IPC command line (`{"command":[...]}` + newline).
pub fn encode_command(args: &[Value]) -> String {
    format!("{}\n", json!({ "command": args }))
}

/// Parse one line from mpv into an app event; `None` for lines we do not care about.
pub fn parse_line(line: &str) -> Option<MpvEvent> {
    let v: Value = serde_json::from_str(line).ok()?;
    match v.get("event")?.as_str()? {
        "file-loaded" => Some(MpvEvent::FileLoaded),
        "end-file" => {
            let reason = match v.get("reason").and_then(Value::as_str) {
                Some("eof") => EndReason::Eof,
                Some("stop") => EndReason::Stop,
                Some("quit") => EndReason::Quit,
                Some("error") => EndReason::Error,
                _ => EndReason::Other,
            };
            Some(MpvEvent::EndFile { reason })
        }
        "property-change" => {
            let data = v.get("data");
            match v.get("name")?.as_str()? {
                "time-pos" => data?.as_f64().map(MpvEvent::PositionChanged),
                "duration" => Some(MpvEvent::DurationChanged(data.and_then(Value::as_f64))),
                "pause" => data?.as_bool().map(MpvEvent::PauseChanged),
                "volume" => data?
                    .as_f64()
                    .map(|f| MpvEvent::VolumeChanged(f.round() as i64)),
                "idle-active" => data?.as_bool()?.then_some(MpvEvent::Idle),
                _ => None,
            }
        }
        _ => None,
    }
}

struct Proc {
    child: Child,
    writer: UnixStream,
    socket: PathBuf,
    /// Set when we end the process ourselves, so the reader stays quiet.
    detached: Arc<AtomicBool>,
}

pub struct Mpv {
    tx: Sender<AppEvent>,
    volume: i64,
    proc: Option<Proc>,
    #[cfg(test)]
    extra_args: Vec<String>,
    /// Tests only: `load` succeeds without starting a process.
    #[cfg(test)]
    pub(super) dry: bool,
}

impl Mpv {
    pub fn new(tx: Sender<AppEvent>, volume: i64) -> Self {
        Self {
            tx,
            volume,
            proc: None,
            #[cfg(test)]
            extra_args: Vec::new(),
            #[cfg(test)]
            dry: false,
        }
    }

    /// Start (or restart after a crash) mpv if needed.
    fn ensure(&mut self) -> Result<&mut Proc, String> {
        if let Some(p) = &mut self.proc {
            if matches!(p.child.try_wait(), Ok(None)) {
                // Re-borrow so the early return does not hold `self.proc` across the respawn.
                return self.proc.as_mut().ok_or_else(|| "mpv gone".to_string());
            }
            self.discard();
        }
        let p = self.spawn()?;
        Ok(self.proc.insert(p))
    }

    fn spawn(&self) -> Result<Proc, String> {
        let dir = socket_dir().map_err(|e| format!("cannot create socket dir: {e}"))?;
        let socket = dir.join(format!(
            "mpv-{}-{}.sock",
            std::process::id(),
            SOCKET_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&socket);
        let mut cmd = Command::new("mpv");
        cmd.arg("--idle=yes")
            .arg(format!("--input-ipc-server={}", socket.display()))
            .arg("--no-terminal")
            .arg("--force-window=no")
            .arg("--load-scripts=no")
            .arg("--keep-open=no")
            .arg(format!("--volume={}", self.volume));
        #[cfg(test)]
        cmd.args(&self.extra_args);
        let mut child = cmd
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => {
                    "mpv is not installed (not found in PATH)".to_string()
                }
                _ => format!("cannot start mpv: {e}"),
            })?;
        let stream = match connect(&mut child, &socket) {
            Ok(s) => s,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_file(&socket);
                return Err(e);
            }
        };
        let detached = Arc::new(AtomicBool::new(false));
        if let Err(e) = stream.set_write_timeout(Some(WRITE_TIMEOUT)) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(&socket);
            return Err(format!("mpv socket: {e}"));
        }
        let mut proc = Proc {
            child,
            writer: stream,
            socket,
            detached,
        };
        let reader = proc
            .writer
            .try_clone()
            .map_err(|e| format!("mpv socket: {e}"))?;
        spawn_reader(reader, self.tx.clone(), proc.detached.clone());
        for (id, name) in [
            (1, "time-pos"),
            (2, "duration"),
            (3, "pause"),
            (4, "volume"),
            (5, "idle-active"),
        ] {
            write_cmd(
                &mut proc.writer,
                &[json!("observe_property"), json!(id), json!(name)],
            )?;
        }
        // Closing the video window must stop playback, not quit the process.
        write_cmd(
            &mut proc.writer,
            &[json!("keybind"), json!("CLOSE_WIN"), json!("stop")],
        )?;
        Ok(proc)
    }

    /// Drop the current process (kill + reap + remove socket).
    fn discard(&mut self) {
        if let Some(mut p) = self.proc.take() {
            p.detached.store(true, Ordering::SeqCst);
            let _ = write_cmd(&mut p.writer, &[json!("quit")]);
            for _ in 0..20 {
                if matches!(p.child.try_wait(), Ok(Some(_))) {
                    break;
                }
                thread::sleep(Duration::from_millis(25));
            }
            let _ = p.child.kill();
            let _ = p.child.wait();
            let _ = std::fs::remove_file(&p.socket);
        }
    }

    /// Send to a running process; no-op when nothing is running.
    fn send(&mut self, args: &[Value]) -> Result<(), String> {
        let Some(p) = &mut self.proc else {
            return Ok(());
        };
        let res = write_cmd(&mut p.writer, args);
        if res.is_err() {
            self.discard();
            let _ = self.tx.send(AppEvent::Mpv(MpvEvent::ProcessExited));
        }
        res
    }

    /// Load and play a file. Audio disables video first so no window opens.
    pub fn load(&mut self, path: &Path, is_video: bool) -> Result<(), String> {
        let path = path.to_str().ok_or("path is not valid UTF-8")?.to_string();
        #[cfg(test)]
        if self.dry {
            return Ok(());
        }
        // A just-killed mpv may not be reapable yet; retry once on a fresh process.
        let mut res = self.load_once(&path, is_video);
        if res.is_err() {
            res = self.load_once(&path, is_video);
        }
        res
    }

    fn load_once(&mut self, path: &str, is_video: bool) -> Result<(), String> {
        let proc = self.ensure()?;
        let vid = if is_video { "auto" } else { "no" };
        let res = write_cmd(
            &mut proc.writer,
            &[json!("set_property"), json!("vid"), json!(vid)],
        )
        .and_then(|_| {
            write_cmd(
                &mut proc.writer,
                &[json!("loadfile"), json!(path), json!("replace")],
            )
        });
        if res.is_err() {
            self.discard();
        }
        res
    }

    pub fn set_pause(&mut self, pause: bool) -> Result<(), String> {
        self.send(&[json!("set_property"), json!("pause"), json!(pause)])
    }

    pub fn seek_relative(&mut self, secs: f64) -> Result<(), String> {
        self.send(&[json!("seek"), json!(secs), json!("relative")])
    }

    pub fn seek_absolute(&mut self, secs: f64) -> Result<(), String> {
        self.send(&[json!("seek"), json!(secs), json!("absolute")])
    }

    pub fn set_volume(&mut self, volume: i64) -> Result<(), String> {
        self.volume = volume;
        self.send(&[json!("set_property"), json!("volume"), json!(volume)])
    }

    /// Repeat One: loop the current file forever.
    pub fn set_loop_file(&mut self, on: bool) -> Result<(), String> {
        let v = if on { "inf" } else { "no" };
        self.send(&[json!("set_property"), json!("loop-file"), json!(v)])
    }

    pub fn stop(&mut self) -> Result<(), String> {
        self.send(&[json!("stop")])
    }

    pub fn shutdown(&mut self) {
        self.discard();
    }
}

impl Drop for Mpv {
    fn drop(&mut self) {
        self.discard();
    }
}

fn write_cmd(w: &mut UnixStream, args: &[Value]) -> Result<(), String> {
    w.write_all(encode_command(args).as_bytes())
        .map_err(|e| format!("mpv IPC write failed: {e}"))
}

fn connect(child: &mut Child, socket: &Path) -> Result<UnixStream, String> {
    for _ in 0..CONNECT_ATTEMPTS {
        if let Ok(s) = UnixStream::connect(socket) {
            return Ok(s);
        }
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!("mpv exited at startup ({status})"));
        }
        thread::sleep(CONNECT_BACKOFF);
    }
    Err("timed out connecting to mpv".to_string())
}

fn spawn_reader(stream: UnixStream, tx: Sender<AppEvent>, detached: Arc<AtomicBool>) {
    thread::spawn(move || {
        for line in BufReader::new(stream).lines() {
            let Ok(line) = line else { break };
            if let Some(ev) = parse_line(&line)
                && tx.send(AppEvent::Mpv(ev)).is_err()
            {
                return;
            }
        }
        if !detached.load(Ordering::SeqCst) {
            let _ = tx.send(AppEvent::Mpv(MpvEvent::ProcessExited));
        }
    });
}

/// `$XDG_RUNTIME_DIR/rmp`, else `/tmp/rmp-<uid>`; always mode 0700.
fn socket_dir() -> std::io::Result<PathBuf> {
    let dir = match std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()) {
        Some(base) => PathBuf::from(base).join("rmp"),
        None => PathBuf::from(format!(
            "/tmp/rmp-{}",
            std::fs::metadata("/proc/self")?.uid()
        )),
    };
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)?;
    // Refuse a symlink or a dir someone else owns (predictable /tmp name).
    let meta = std::fs::symlink_metadata(&dir)?;
    if !meta.is_dir() || meta.uid() != std::fs::metadata("/proc/self")?.uid() {
        return Err(std::io::Error::other("socket dir not a directory we own"));
    }
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Instant;

    #[test]
    fn encodes_commands() {
        let s = encode_command(&[json!("set_property"), json!("pause"), json!(true)]);
        assert_eq!(s, "{\"command\":[\"set_property\",\"pause\",true]}\n");
        let s = encode_command(&[json!("loadfile"), json!("/a b/\"q\".mp3"), json!("replace")]);
        let v: Value = serde_json::from_str(s.trim()).unwrap();
        assert_eq!(v["command"][1], "/a b/\"q\".mp3");
    }

    #[test]
    fn parses_property_changes() {
        let p = |l| parse_line(l);
        assert_eq!(
            p(r#"{"event":"property-change","id":1,"name":"time-pos","data":12.5}"#),
            Some(MpvEvent::PositionChanged(12.5))
        );
        assert_eq!(
            p(r#"{"event":"property-change","id":2,"name":"duration","data":200.0}"#),
            Some(MpvEvent::DurationChanged(Some(200.0)))
        );
        assert_eq!(
            p(r#"{"event":"property-change","id":2,"name":"duration"}"#),
            Some(MpvEvent::DurationChanged(None))
        );
        assert_eq!(
            p(r#"{"event":"property-change","id":3,"name":"pause","data":true}"#),
            Some(MpvEvent::PauseChanged(true))
        );
        assert_eq!(
            p(r#"{"event":"property-change","id":4,"name":"volume","data":74.6}"#),
            Some(MpvEvent::VolumeChanged(75))
        );
        assert_eq!(
            p(r#"{"event":"property-change","id":5,"name":"idle-active","data":true}"#),
            Some(MpvEvent::Idle)
        );
        assert_eq!(
            p(r#"{"event":"property-change","id":5,"name":"idle-active","data":false}"#),
            None
        );
        assert_eq!(
            p(r#"{"event":"property-change","id":1,"name":"time-pos"}"#),
            None
        );
    }

    #[test]
    fn parses_end_file_and_noise() {
        let reason = |r: &str| parse_line(&format!(r#"{{"event":"end-file","reason":"{r}"}}"#));
        assert_eq!(
            reason("eof"),
            Some(MpvEvent::EndFile {
                reason: EndReason::Eof
            })
        );
        assert_eq!(
            reason("stop"),
            Some(MpvEvent::EndFile {
                reason: EndReason::Stop
            })
        );
        assert_eq!(
            reason("quit"),
            Some(MpvEvent::EndFile {
                reason: EndReason::Quit
            })
        );
        assert_eq!(
            reason("error"),
            Some(MpvEvent::EndFile {
                reason: EndReason::Error
            })
        );
        assert_eq!(
            reason("redirect"),
            Some(MpvEvent::EndFile {
                reason: EndReason::Other
            })
        );
        assert_eq!(
            parse_line(r#"{"event":"file-loaded"}"#),
            Some(MpvEvent::FileLoaded)
        );
        assert_eq!(parse_line(r#"{"error":"success","request_id":0}"#), None);
        assert_eq!(parse_line("not json"), None);
        assert_eq!(parse_line(r#"{"event":"start-file"}"#), None);
    }

    fn silent_wav(secs: u32) -> Vec<u8> {
        let rate = 8000u32;
        let data_len = rate * secs;
        let mut w = Vec::new();
        w.extend(b"RIFF");
        w.extend((36 + data_len).to_le_bytes());
        w.extend(b"WAVEfmt ");
        w.extend(16u32.to_le_bytes());
        w.extend(1u16.to_le_bytes()); // PCM
        w.extend(1u16.to_le_bytes()); // mono
        w.extend(rate.to_le_bytes());
        w.extend(rate.to_le_bytes()); // byte rate (8-bit mono)
        w.extend(1u16.to_le_bytes());
        w.extend(8u16.to_le_bytes());
        w.extend(b"data");
        w.extend(data_len.to_le_bytes());
        w.extend(std::iter::repeat_n(128u8, data_len as usize));
        w
    }

    fn wait_for(rx: &mpsc::Receiver<AppEvent>, want: impl Fn(&MpvEvent) -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            match rx.recv_timeout(left) {
                Ok(AppEvent::Mpv(ev)) if want(&ev) => return true,
                Ok(_) => {}
                Err(_) => return false,
            }
        }
        false
    }

    #[test]
    fn plays_real_wav_with_mpv() {
        if Command::new("mpv").arg("--version").output().is_err() {
            eprintln!("skipping: mpv not on PATH");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("s.wav");
        std::fs::write(&wav, silent_wav(1)).unwrap();
        let (tx, rx) = mpsc::channel();
        let mut mpv = Mpv::new(tx, 50);
        mpv.extra_args = vec!["--ao=null".into(), "--vo=null".into()];
        mpv.load(&wav, false).unwrap();
        assert!(
            wait_for(&rx, |e| *e == MpvEvent::FileLoaded),
            "no FileLoaded"
        );
        assert!(
            wait_for(&rx, |e| matches!(
                e,
                MpvEvent::EndFile {
                    reason: EndReason::Eof
                }
            )),
            "no EndFile eof"
        );
        // Second load reuses the same process; shutdown removes the socket.
        mpv.load(&wav, false).unwrap();
        assert!(wait_for(&rx, |e| *e == MpvEvent::FileLoaded));
        let socket = mpv.proc.as_ref().unwrap().socket.clone();
        mpv.shutdown();
        assert!(!socket.exists());
    }

    fn have_mpv() -> bool {
        Command::new("mpv").arg("--version").output().is_ok()
    }

    fn test_mpv(tx: mpsc::Sender<AppEvent>) -> Mpv {
        let mut m = Mpv::new(tx, 50);
        m.extra_args = vec!["--ao=null".into(), "--vo=null".into()];
        m
    }

    fn pid(m: &Mpv) -> u32 {
        m.proc.as_ref().unwrap().child.id()
    }

    #[test]
    fn respawns_after_kill_9() {
        if !have_mpv() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("s.wav");
        std::fs::write(&wav, silent_wav(2)).unwrap();
        let (tx, rx) = mpsc::channel();
        let mut mpv = test_mpv(tx);
        mpv.load(&wav, false).unwrap();
        assert!(wait_for(&rx, |e| *e == MpvEvent::FileLoaded));
        let old = pid(&mpv);
        Command::new("kill")
            .args(["-9", &old.to_string()])
            .status()
            .unwrap();
        assert!(wait_for(&rx, |e| *e == MpvEvent::ProcessExited));
        mpv.load(&wav, false).unwrap();
        assert_ne!(pid(&mpv), old);
        assert!(wait_for(&rx, |e| *e == MpvEvent::FileLoaded));
        mpv.shutdown();
    }

    #[test]
    fn wedged_mpv_does_not_hang_writes() {
        if !have_mpv() {
            return;
        }
        let (tx, _rx) = mpsc::channel();
        let mut mpv = test_mpv(tx);
        mpv.load(Path::new("/nonexistent.wav"), false).unwrap();
        Command::new("kill")
            .args(["-STOP", &pid(&mpv).to_string()])
            .status()
            .unwrap();
        let start = Instant::now();
        let mut failed = false;
        for _ in 0..200_000 {
            if mpv.set_volume(10).is_err() {
                failed = true;
                break;
            }
        }
        assert!(failed, "write to stopped mpv never failed");
        mpv.shutdown();
        assert!(start.elapsed() < Duration::from_secs(8));
        assert!(mpv.proc.is_none());
    }

    #[test]
    fn audio_disables_vid_and_repeat_one_sets_loop() {
        if !have_mpv() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("s.wav");
        std::fs::write(&wav, silent_wav(1)).unwrap();
        let (tx, rx) = mpsc::channel();
        let mut mpv = test_mpv(tx);
        mpv.load(&wav, true).unwrap();
        mpv.load(&wav, false).unwrap();
        mpv.set_loop_file(true).unwrap();
        assert!(wait_for(&rx, |e| *e == MpvEvent::FileLoaded));
        // Query mpv directly over a second connection to the same socket.
        let sock = mpv.proc.as_ref().unwrap().socket.clone();
        let mut c = UnixStream::connect(sock).unwrap();
        c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut r = BufReader::new(c.try_clone().unwrap());
        let mut get = |name: &str| -> Value {
            write_cmd(&mut c, &[json!("get_property"), json!(name)]).unwrap();
            loop {
                let mut l = String::new();
                r.read_line(&mut l).unwrap();
                let v: Value = serde_json::from_str(&l).unwrap();
                if v.get("error").is_some() {
                    return v["data"].clone();
                }
            }
        };
        assert_eq!(get("vid"), json!(false));
        assert_eq!(get("loop-file"), json!("inf"));
        mpv.shutdown();
    }

    #[test]
    fn socket_dir_is_0700() {
        let d = socket_dir().unwrap();
        assert_eq!(
            std::fs::metadata(d).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
}
