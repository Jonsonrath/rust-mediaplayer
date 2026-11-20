//! Playback engine: mpv control, queue and play modes.
pub mod modes;
pub mod mpv;
pub mod queue;

use std::sync::mpsc::Sender;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::event::{AppEvent, Command, EndReason, MpvEvent};
use crate::library::Track;
use modes::RepeatMode;
use mpv::Mpv;
use queue::Queue;

/// Plays shorter than this (and not completed) are not worth a history row.
const MIN_HISTORY_SECS: f64 = 5.0;
/// Position steps larger than this are seeks, not listening time.
const MAX_TICK_SECS: f64 = 2.5;
pub const MAX_VOLUME: i64 = 130;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlayState {
    Playing,
    Paused,
    #[default]
    Stopped,
}

/// Read-only view of the player for the UI and MPRIS.
#[derive(Debug, Clone)]
pub struct PlayerSnapshot {
    pub state: PlayState,
    pub current: Option<Track>,
    /// Seconds.
    pub position: f64,
    pub duration: Option<f64>,
    /// 0..=130
    pub volume: i64,
    pub repeat: RepeatMode,
    pub shuffle: bool,
}

impl Default for PlayerSnapshot {
    fn default() -> Self {
        Self {
            state: PlayState::Stopped,
            current: None,
            position: 0.0,
            duration: None,
            volume: 100,
            repeat: RepeatMode::Off,
            shuffle: false,
        }
    }
}

/// One finished play, for the history table.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayRecord {
    pub track: Track,
    /// Unix seconds.
    pub started_at: i64,
    pub played_secs: f64,
    pub completed: bool,
}

/// The track currently loaded in mpv, for history bookkeeping.
struct Active {
    track: Track,
    started_at: i64,
    /// Seconds actually listened: small forward position steps only, so seeks
    /// and loops do not count.
    played: f64,
    last_pos: f64,
}

/// Single authoritative playback state: mpv + queue + snapshot.
pub struct Player {
    mpv: Mpv,
    queue: Queue,
    snap: PlayerSnapshot,
    active: Option<Active>,
    finished: Vec<PlayRecord>,
    error: Option<String>,
    /// A load was sent and `FileLoaded` has not arrived yet; an `EndFile(Stop)` in
    /// this window belongs to the file being replaced.
    awaiting_load: bool,
    /// Consecutive unplayable files; bounds skipping through a broken queue.
    failures: usize,
}

impl Player {
    pub fn new(tx: Sender<AppEvent>, volume: i64) -> Self {
        let volume = volume.clamp(0, MAX_VOLUME);
        Self::with_mpv(Mpv::new(tx, volume), volume)
    }

    fn with_mpv(mpv: Mpv, volume: i64) -> Self {
        Self {
            mpv,
            queue: Queue::new(),
            snap: PlayerSnapshot {
                volume,
                ..PlayerSnapshot::default()
            },
            active: None,
            finished: Vec::new(),
            error: None,
            awaiting_load: false,
            failures: 0,
        }
    }

    pub fn snapshot(&self) -> &PlayerSnapshot {
        &self.snap
    }

    pub fn queue(&self) -> &Queue {
        &self.queue
    }

    pub fn queue_mut(&mut self) -> &mut Queue {
        &mut self.queue
    }

    /// Last asynchronous error (skipped file, mpv crash); cleared by reading.
    pub fn take_error(&mut self) -> Option<String> {
        self.error.take()
    }

    /// Oldest finished play not yet handed out.
    pub fn take_finished_play(&mut self) -> Option<PlayRecord> {
        (!self.finished.is_empty()).then(|| self.finished.remove(0))
    }

    pub fn shutdown(&mut self) {
        self.finish_active(false);
        self.mpv.shutdown();
    }

    pub fn handle_command(&mut self, cmd: &Command) -> Result<(), String> {
        self.failures = 0;
        match cmd {
            Command::TogglePause => match self.snap.state {
                PlayState::Stopped => self.play_or_resume(),
                s => self.set_pause(s == PlayState::Playing),
            },
            Command::Play => self.play_or_resume(),
            Command::Pause if self.snap.state == PlayState::Playing => self.set_pause(true),
            Command::Pause => Ok(()),
            Command::Stop => {
                self.stop();
                Ok(())
            }
            Command::Next => {
                let repeat = self.snap.repeat;
                if !self.go(|q| q.next(repeat, true).is_some()) {
                    self.stop();
                }
                Ok(())
            }
            Command::Prev => {
                let repeat = self.snap.repeat;
                let can_seek = self.snap.state != PlayState::Stopped;
                if !self.go(|q| q.prev(repeat).is_some())
                    && can_seek
                    && self.queue.current().is_some()
                {
                    return self.mpv.seek_absolute(0.0);
                }
                Ok(())
            }
            Command::SeekRelative(s) if self.snap.state != PlayState::Stopped => {
                self.mpv.seek_relative(*s)
            }
            Command::SeekAbsolute(s) if self.snap.state != PlayState::Stopped => {
                self.snap.position = s.max(0.0);
                self.mpv.seek_absolute(*s)
            }
            Command::SeekRelative(_) | Command::SeekAbsolute(_) => Ok(()),
            Command::SetVolume(v) => self.set_volume(*v),
            Command::VolumeDelta(d) => self.set_volume(self.snap.volume + d),
            Command::CycleRepeat => self.set_repeat(self.snap.repeat.next()),
            Command::SetRepeat(m) => self.set_repeat(*m),
            Command::ToggleShuffle => {
                self.set_shuffle(!self.snap.shuffle);
                Ok(())
            }
            Command::SetShuffle(on) => {
                self.set_shuffle(*on);
                Ok(())
            }
            Command::PlayTracks { tracks, start } => {
                self.queue.clear();
                let ids = self.queue.push(tracks.clone());
                let Some(&id) = ids.get(*start) else {
                    self.stop();
                    return if tracks.is_empty() {
                        Ok(())
                    } else {
                        Err("start index out of range".into())
                    };
                };
                self.queue.set_current(id);
                self.start_current();
                Ok(())
            }
            Command::Enqueue(tracks) => {
                self.queue.push(tracks.clone());
                Ok(())
            }
            Command::PlayQueueEntry(id) => {
                if !self.queue.set_current(*id) {
                    return Err("queue entry no longer exists".into());
                }
                self.start_current();
                Ok(())
            }
            Command::Quit => {
                self.shutdown();
                Ok(())
            }
            Command::Rescan => Ok(()),
        }
    }

    pub fn handle_mpv_event(&mut self, ev: &MpvEvent) {
        let live = self.snap.state != PlayState::Stopped;
        match ev {
            MpvEvent::PositionChanged(p) if live => {
                self.snap.position = *p;
                if let Some(a) = &mut self.active {
                    let d = *p - a.last_pos;
                    if d > 0.0 && d <= MAX_TICK_SECS {
                        a.played += d;
                    }
                    a.last_pos = *p;
                }
                if *p >= 1.0 {
                    self.failures = 0;
                }
            }
            MpvEvent::DurationChanged(Some(d)) if live => self.snap.duration = Some(*d),
            MpvEvent::PauseChanged(p) if live => {
                self.snap.state = if *p {
                    PlayState::Paused
                } else {
                    PlayState::Playing
                };
            }
            MpvEvent::VolumeChanged(v) => self.snap.volume = *v,
            // `failures` is reset by real progress, not here: a file can load and
            // still fail at once, which must not defeat the skip bound.
            MpvEvent::FileLoaded => self.awaiting_load = false,
            MpvEvent::EndFile { reason } => self.on_end_file(*reason),
            MpvEvent::ProcessExited => {
                self.awaiting_load = false;
                if live {
                    self.error = Some("mpv exited unexpectedly".into());
                }
                self.stop_local();
            }
            _ => {}
        }
    }

    fn on_end_file(&mut self, reason: EndReason) {
        // A late Eof/Error after Stop must not restart playback.
        if self.active.is_none() && matches!(reason, EndReason::Eof | EndReason::Error) {
            return;
        }
        match reason {
            EndReason::Eof => {
                self.failures = 0;
                self.finish_active(true);
                self.advance_after_end();
            }
            EndReason::Error => {
                self.awaiting_load = false;
                self.finish_active(false);
                let msg = self.snap.current.as_ref().map_or_else(
                    || "Cannot play file".to_string(),
                    |t| format!("Cannot play {}", t.title),
                );
                if self.skip_failed(msg) {
                    self.start_current();
                }
            }
            // Stop for a replaced file, or after our own stop: nothing to do.
            EndReason::Stop if self.awaiting_load || self.snap.state == PlayState::Stopped => {}
            // The video window was closed: treat as stop.
            EndReason::Stop => self.stop_local(),
            EndReason::Quit | EndReason::Other => {}
        }
    }

    fn advance_after_end(&mut self) {
        if self.queue.next(self.snap.repeat, false).is_some() {
            self.start_current();
        } else {
            self.stop_local();
        }
    }

    /// Run a queue move; start the entry it selected. False if it moved nowhere.
    fn go(&mut self, mv: impl FnOnce(&mut Queue) -> bool) -> bool {
        let moved = mv(&mut self.queue);
        if moved {
            self.start_current();
        }
        moved
    }

    fn play_or_resume(&mut self) -> Result<(), String> {
        if self.snap.state == PlayState::Paused {
            return self.set_pause(false);
        }
        if self.snap.state == PlayState::Stopped {
            if self.queue.current().is_none() {
                self.queue.next(self.snap.repeat, true);
            }
            self.start_current();
        }
        Ok(())
    }

    fn set_pause(&mut self, pause: bool) -> Result<(), String> {
        self.snap.state = if pause {
            PlayState::Paused
        } else {
            PlayState::Playing
        };
        self.mpv.set_pause(pause)
    }

    fn set_volume(&mut self, v: i64) -> Result<(), String> {
        self.snap.volume = v.clamp(0, MAX_VOLUME);
        self.mpv.set_volume(self.snap.volume)
    }

    fn set_repeat(&mut self, mode: RepeatMode) -> Result<(), String> {
        self.snap.repeat = mode;
        self.mpv.set_loop_file(mode == RepeatMode::One)
    }

    fn set_shuffle(&mut self, on: bool) {
        self.snap.shuffle = on;
        self.queue.set_shuffle(on);
    }

    fn stop(&mut self) {
        self.stop_local();
        if let Err(e) = self.mpv.stop() {
            self.error = Some(e);
        }
    }

    /// Reset to Stopped without talking to mpv.
    fn stop_local(&mut self) {
        self.finish_active(false);
        self.snap.state = PlayState::Stopped;
        self.snap.position = 0.0;
    }

    fn finish_active(&mut self, completed: bool) {
        if let Some(a) = self.active.take()
            && (completed || a.played >= MIN_HISTORY_SECS)
        {
            self.finished.push(PlayRecord {
                track: a.track,
                started_at: a.started_at,
                played_secs: a.played,
                completed,
            });
        }
    }

    /// Record a failure and move to the next entry. False when playback must stop
    /// (end of queue, or every entry has failed in a row).
    fn skip_failed(&mut self, msg: String) -> bool {
        self.error = Some(msg);
        self.failures += 1;
        if self.failures > self.queue.len() || self.queue.next(self.snap.repeat, true).is_none() {
            self.stop_local();
            return false;
        }
        true
    }

    /// Load the queue's current entry into mpv, skipping unplayable files.
    fn start_current(&mut self) {
        self.finish_active(false);
        loop {
            let Some(track) = self.queue.current().map(|e| e.track.clone()) else {
                self.stop_local();
                return;
            };
            if !track.path.exists() {
                let msg = format!("File not found: {}", track.path.display());
                if self.skip_failed(msg) {
                    continue;
                }
                return;
            }
            let loaded = self
                .mpv
                .load(&track.path, track.is_video)
                .and_then(|()| self.mpv.set_pause(false))
                .and_then(|()| self.mpv.set_loop_file(self.snap.repeat == RepeatMode::One));
            if let Err(e) = loaded {
                self.error = Some(e);
                self.stop_local();
                return;
            }
            self.awaiting_load = true;
            self.snap.state = PlayState::Playing;
            self.snap.position = 0.0;
            self.snap.duration = track.duration;
            self.snap.current = Some(track.clone());
            self.active = Some(Active {
                track,
                started_at: unix_now(),
                played: 0.0,
                last_pos: 0.0,
            });
            return;
        }
    }
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::mpsc;

    fn track(path: &std::path::Path) -> Track {
        Track {
            id: 0,
            path: path.to_path_buf(),
            title: path.file_name().unwrap().to_string_lossy().into_owned(),
            artist: None,
            album: None,
            album_artist: None,
            genre: None,
            track_no: None,
            year: None,
            duration: Some(100.0),
            format: None,
            size: 0,
            width: None,
            height: None,
            is_video: false,
        }
    }

    fn player() -> Player {
        let (tx, _rx) = mpsc::channel();
        let mut mpv = Mpv::new(tx, 80);
        mpv.dry = true;
        Player::with_mpv(mpv, 80)
    }

    /// `n` existing files plus the tracks for them.
    fn files(n: usize) -> (tempfile::TempDir, Vec<Track>) {
        let dir = tempfile::tempdir().unwrap();
        let tracks = (0..n)
            .map(|i| {
                let p = dir.path().join(format!("t{i}.mp3"));
                std::fs::write(&p, b"x").unwrap();
                track(&p)
            })
            .collect();
        (dir, tracks)
    }

    fn eof() -> MpvEvent {
        MpvEvent::EndFile {
            reason: EndReason::Eof,
        }
    }

    fn title(p: &Player) -> String {
        p.snapshot().current.as_ref().unwrap().title.clone()
    }

    #[test]
    fn play_tracks_starts_at_index() {
        let (_d, t) = files(3);
        let mut p = player();
        p.handle_command(&Command::PlayTracks {
            tracks: t,
            start: 1,
        })
        .unwrap();
        assert_eq!(title(&p), "t1.mp3");
        assert_eq!(p.snapshot().state, PlayState::Playing);
        assert_eq!(p.queue().len(), 3);
    }

    #[test]
    fn eof_advances_then_stops_at_end() {
        let (_d, t) = files(2);
        let mut p = player();
        p.handle_command(&Command::PlayTracks {
            tracks: t,
            start: 0,
        })
        .unwrap();
        p.handle_mpv_event(&eof());
        assert_eq!(title(&p), "t1.mp3");
        p.handle_mpv_event(&eof());
        assert_eq!(p.snapshot().state, PlayState::Stopped);
    }

    #[test]
    fn repeat_all_wraps_and_one_repeats() {
        let (_d, t) = files(2);
        let mut p = player();
        p.handle_command(&Command::SetRepeat(RepeatMode::All))
            .unwrap();
        p.handle_command(&Command::PlayTracks {
            tracks: t,
            start: 1,
        })
        .unwrap();
        p.handle_mpv_event(&eof());
        assert_eq!(title(&p), "t0.mp3");
        p.handle_command(&Command::SetRepeat(RepeatMode::One))
            .unwrap();
        p.handle_mpv_event(&eof());
        assert_eq!(title(&p), "t0.mp3");
        p.handle_command(&Command::Next).unwrap();
        assert_eq!(title(&p), "t1.mp3");
    }

    #[test]
    fn cycle_repeat_and_shuffle_toggle() {
        let mut p = player();
        let modes: Vec<_> = (0..3)
            .map(|_| {
                p.handle_command(&Command::CycleRepeat).unwrap();
                p.snapshot().repeat
            })
            .collect();
        assert_eq!(modes, [RepeatMode::All, RepeatMode::One, RepeatMode::Off]);
        p.handle_command(&Command::ToggleShuffle).unwrap();
        assert!(p.snapshot().shuffle && p.queue().shuffle());
    }

    #[test]
    fn volume_is_clamped() {
        let mut p = player();
        p.handle_command(&Command::SetVolume(500)).unwrap();
        assert_eq!(p.snapshot().volume, 130);
        p.handle_command(&Command::VolumeDelta(-500)).unwrap();
        assert_eq!(p.snapshot().volume, 0);
    }

    #[test]
    fn pause_toggles_state() {
        let (_d, t) = files(1);
        let mut p = player();
        p.handle_command(&Command::PlayTracks {
            tracks: t,
            start: 0,
        })
        .unwrap();
        p.handle_command(&Command::TogglePause).unwrap();
        assert_eq!(p.snapshot().state, PlayState::Paused);
        p.handle_command(&Command::Play).unwrap();
        assert_eq!(p.snapshot().state, PlayState::Playing);
        p.handle_command(&Command::Stop).unwrap();
        assert_eq!(p.snapshot().state, PlayState::Stopped);
    }

    #[test]
    fn missing_file_is_skipped_with_error() {
        let (_d, mut t) = files(2);
        t.insert(0, track(&PathBuf::from("/nonexistent/gone.mp3")));
        let mut p = player();
        p.handle_command(&Command::PlayTracks {
            tracks: t,
            start: 0,
        })
        .unwrap();
        assert_eq!(title(&p), "t0.mp3");
        assert!(p.take_error().unwrap().contains("gone.mp3"));
        assert!(p.take_error().is_none());
    }

    #[test]
    fn all_broken_queue_terminates_even_with_repeat_all_and_one() {
        for mode in [RepeatMode::Off, RepeatMode::All, RepeatMode::One] {
            let t: Vec<_> = (0..3)
                .map(|i| track(&PathBuf::from(format!("/nonexistent/{i}.mp3"))))
                .collect();
            let mut p = player();
            p.handle_command(&Command::SetRepeat(mode)).unwrap();
            p.handle_command(&Command::PlayTracks {
                tracks: t,
                start: 0,
            })
            .unwrap();
            assert_eq!(p.snapshot().state, PlayState::Stopped, "{mode:?}");
            assert!(p.take_error().is_some());
        }
    }

    #[test]
    fn mpv_error_end_skips_to_next() {
        let (_d, t) = files(2);
        let mut p = player();
        p.handle_command(&Command::PlayTracks {
            tracks: t,
            start: 0,
        })
        .unwrap();
        p.handle_mpv_event(&MpvEvent::EndFile {
            reason: EndReason::Error,
        });
        assert_eq!(title(&p), "t1.mp3");
        assert!(p.take_error().unwrap().contains("t0.mp3"));
    }

    #[test]
    fn stop_from_replaced_file_or_closed_window() {
        let (_d, t) = files(2);
        let mut p = player();
        p.handle_command(&Command::PlayTracks {
            tracks: t,
            start: 0,
        })
        .unwrap();
        // Replacement of the previous file reports stop before FileLoaded: ignored.
        p.handle_mpv_event(&MpvEvent::EndFile {
            reason: EndReason::Stop,
        });
        assert_eq!(p.snapshot().state, PlayState::Playing);
        p.handle_mpv_event(&MpvEvent::FileLoaded);
        // Window closed later: stop.
        p.handle_mpv_event(&MpvEvent::EndFile {
            reason: EndReason::Stop,
        });
        assert_eq!(p.snapshot().state, PlayState::Stopped);
    }

    #[test]
    fn process_exit_stops_and_reports() {
        let (_d, t) = files(1);
        let mut p = player();
        p.handle_command(&Command::PlayTracks {
            tracks: t,
            start: 0,
        })
        .unwrap();
        p.handle_mpv_event(&MpvEvent::ProcessExited);
        assert_eq!(p.snapshot().state, PlayState::Stopped);
        assert!(p.take_error().is_some());
    }

    #[test]
    fn mpv_events_update_snapshot() {
        let (_d, t) = files(1);
        let mut p = player();
        p.handle_command(&Command::PlayTracks {
            tracks: t,
            start: 0,
        })
        .unwrap();
        p.handle_mpv_event(&MpvEvent::PositionChanged(12.0));
        p.handle_mpv_event(&MpvEvent::DurationChanged(Some(240.0)));
        p.handle_mpv_event(&MpvEvent::PauseChanged(true));
        p.handle_mpv_event(&MpvEvent::VolumeChanged(55));
        let s = p.snapshot();
        assert_eq!((s.position, s.duration, s.volume), (12.0, Some(240.0), 55));
        assert_eq!(s.state, PlayState::Paused);
    }

    #[test]
    fn history_records_only_real_plays() {
        let (_d, t) = files(3);
        let mut p = player();
        p.handle_command(&Command::PlayTracks {
            tracks: t,
            start: 0,
        })
        .unwrap();
        p.handle_mpv_event(&MpvEvent::PositionChanged(2.0));
        p.handle_command(&Command::Next).unwrap(); // 2s: too short
        assert!(p.take_finished_play().is_none());
        for s in 1..=30 {
            p.handle_mpv_event(&MpvEvent::PositionChanged(f64::from(s)));
        }
        p.handle_command(&Command::Next).unwrap(); // 30s: counts
        let r = p.take_finished_play().unwrap();
        assert_eq!((r.track.title.as_str(), r.completed), ("t1.mp3", false));
        assert_eq!(r.played_secs, 30.0);
        p.handle_mpv_event(&eof()); // completed, even if short
        let r = p.take_finished_play().unwrap();
        assert!(r.completed && r.started_at > 0);
        assert!(p.take_finished_play().is_none());
    }

    #[test]
    fn next_at_end_of_queue_stops_and_prev_goes_back() {
        let (_d, t) = files(2);
        let mut p = player();
        p.handle_command(&Command::PlayTracks {
            tracks: t,
            start: 1,
        })
        .unwrap();
        p.handle_command(&Command::Prev).unwrap();
        assert_eq!(title(&p), "t0.mp3");
        p.handle_command(&Command::Next).unwrap();
        p.handle_command(&Command::Next).unwrap();
        assert_eq!(p.snapshot().state, PlayState::Stopped);
    }

    #[test]
    fn enqueue_and_play_entry() {
        let (_d, t) = files(3);
        let mut p = player();
        p.handle_command(&Command::Enqueue(t)).unwrap();
        assert_eq!(p.snapshot().state, PlayState::Stopped);
        let id = p.queue().entries()[2].id;
        p.handle_command(&Command::PlayQueueEntry(id)).unwrap();
        assert_eq!(title(&p), "t2.mp3");
        assert!(p.handle_command(&Command::PlayQueueEntry(999)).is_err());
        let _ = p.handle_command(&Command::Stop);
        p.handle_command(&Command::Play).unwrap();
        assert_eq!(title(&p), "t2.mp3");
    }

    #[test]
    fn seek_is_not_listening_time() {
        let (_d, t) = files(2);
        let mut p = player();
        p.handle_command(&Command::PlayTracks {
            tracks: t,
            start: 0,
        })
        .unwrap();
        p.handle_mpv_event(&MpvEvent::PositionChanged(1.0));
        p.handle_mpv_event(&MpvEvent::PositionChanged(90.0));
        p.handle_command(&Command::Next).unwrap();
        assert!(p.take_finished_play().is_none());
    }

    #[test]
    fn load_then_fail_loop_terminates() {
        let (_d, t) = files(3);
        let mut p = player();
        p.handle_command(&Command::SetRepeat(RepeatMode::All))
            .unwrap();
        p.handle_command(&Command::PlayTracks {
            tracks: t,
            start: 0,
        })
        .unwrap();
        for _ in 0..10 {
            p.handle_mpv_event(&MpvEvent::FileLoaded);
            p.handle_mpv_event(&MpvEvent::EndFile {
                reason: EndReason::Error,
            });
        }
        assert_eq!(p.snapshot().state, PlayState::Stopped);
    }

    #[test]
    fn late_eof_after_stop_does_not_restart() {
        let (_d, t) = files(2);
        let mut p = player();
        p.handle_command(&Command::PlayTracks {
            tracks: t,
            start: 0,
        })
        .unwrap();
        p.handle_command(&Command::Stop).unwrap();
        p.handle_mpv_event(&eof());
        assert_eq!(p.snapshot().state, PlayState::Stopped);
    }

    #[test]
    fn empty_play_tracks_is_fine() {
        let mut p = player();
        p.handle_command(&Command::PlayTracks {
            tracks: vec![],
            start: 0,
        })
        .unwrap();
        assert_eq!(p.snapshot().state, PlayState::Stopped);
        p.handle_command(&Command::Next).unwrap();
        p.handle_command(&Command::Prev).unwrap();
    }
}
