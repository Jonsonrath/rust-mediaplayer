//! Shared event/command contracts between UI, playback, scanner and MPRIS.
use crossterm::event::KeyEvent;

use crate::library::Track;
use crate::playback::modes::RepeatMode;

/// Everything the main loop receives on its single mpsc channel.
#[derive(Debug, Clone)]
pub enum AppEvent {
    Key(KeyEvent),
    Resize,
    Tick,
    Mpv(MpvEvent),
    Scan(ScanEvent),
    /// Sent by the MPRIS thread.
    Command(Command),
}

/// A user or MPRIS intent. UI/MPRIS emit these; the app applies them to playback state.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    TogglePause,
    Play,
    Pause,
    Stop,
    Next,
    Prev,
    SeekRelative(f64),
    SeekAbsolute(f64),
    /// 0..=130
    SetVolume(i64),
    VolumeDelta(i64),
    CycleRepeat,
    SetRepeat(RepeatMode),
    ToggleShuffle,
    SetShuffle(bool),
    /// Replace the queue with `tracks` and start playing `tracks[start]`.
    PlayTracks {
        tracks: Vec<Track>,
        start: usize,
    },
    Enqueue(Vec<Track>),
    /// Play the queue entry with this entry id.
    PlayQueueEntry(u64),
    Quit,
    Rescan,
}

/// Events parsed from mpv's IPC socket by the reader thread.
#[derive(Debug, Clone, PartialEq)]
pub enum MpvEvent {
    PositionChanged(f64),
    DurationChanged(Option<f64>),
    PauseChanged(bool),
    VolumeChanged(i64),
    FileLoaded,
    EndFile { reason: EndReason },
    Idle,
    ProcessExited,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    Eof,
    Stop,
    Quit,
    Error,
    Other,
}

/// Events from the background library scanner.
#[derive(Debug, Clone, PartialEq)]
pub enum ScanEvent {
    Progress {
        scanned: usize,
    },
    Finished {
        added: usize,
        updated: usize,
        removed: usize,
    },
    Error(String),
}
