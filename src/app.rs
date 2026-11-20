//! Application state and event dispatch. Rendering reads this; it never mutates it.
mod dialogs;
pub mod views;

use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent};

use crate::config::keys::{Action, KeyMap};
use crate::event::{AppEvent, Command, MpvEvent, ScanEvent};
use crate::library::Track;
use crate::library::database::{Database, HistoryEntry, Playlist};
use crate::library::scanner;
use crate::mpris::Mpris;
use crate::playback::{PlayState, Player};
use crate::ui::theme::Theme;
use views::{BrowserUi, HistoryUi, LibraryUi, PlaylistsUi, QueueUi};

pub const VIEW_NAMES: [&str; 5] = ["LIBRARY", "FILES", "PLAYLISTS", "QUEUE", "HISTORY"];
const STATUS_TTL: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, PartialEq)]
pub enum TextPurpose {
    NewPlaylist,
    RenamePlaylist(i64),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Confirm {
    ClearHistory,
    ClearQueue,
    DeletePlaylist(i64),
}

/// A modal dialog; while open all keys go to it.
#[derive(Debug, Clone, PartialEq)]
pub enum Dialog {
    Text(TextPurpose, String),
    Confirm(Confirm),
    /// Full (possibly long) error message with a scroll offset.
    Error {
        msg: String,
        scroll: u16,
    },
    Help {
        scroll: u16,
    },
    /// Add-to-playlist picker; `sel` indexes `App::playlists`.
    Picker {
        tracks: Vec<Track>,
        sel: usize,
    },
}

pub struct App {
    /// Clone to hand to background threads.
    pub tx: Sender<AppEvent>,
    pub theme: Theme,
    pub keys: KeyMap,
    /// Active view index 0..5.
    pub view: usize,
    pub player: Player,
    pub db: Database,
    pub mpris: Option<Mpris>,
    /// Transient status line message (error flag); expires via Tick.
    pub status: Option<(String, bool)>,
    status_until: Option<Instant>,
    pub quit: bool,
    /// Set by `handle` when a redraw is needed; cleared by the main loop.
    pub dirty: bool,
    /// Library cache; reloaded only after a scan or explicit edits.
    pub tracks: Vec<Track>,
    pub playlists: Vec<Playlist>,
    pub history: Vec<HistoryEntry>,
    /// Live library search text while typing (`Some`), else `None`.
    pub search: Option<String>,
    /// Open modal dialog, if any.
    pub dialog: Option<Dialog>,
    pub library: LibraryUi,
    pub browser: BrowserUi,
    pub pl_ui: PlaylistsUi,
    pub queue_ui: QueueUi,
    pub hist_ui: HistoryUi,
    pub media_dirs: Vec<PathBuf>,
    /// None when the DB is the in-memory fallback (scanner needs a file).
    pub db_path: Option<PathBuf>,
    pub scanning: bool,
    /// Files seen so far by the running scan.
    pub scan_count: usize,
}

impl App {
    pub fn new(
        tx: Sender<AppEvent>,
        player: Player,
        db: Database,
        keys: KeyMap,
        theme: Theme,
    ) -> Self {
        let mut app = Self {
            tx,
            theme,
            keys,
            view: 0,
            player,
            db,
            mpris: None,
            status: None,
            status_until: None,
            quit: false,
            dirty: true,
            tracks: Vec::new(),
            playlists: Vec::new(),
            history: Vec::new(),
            search: None,
            dialog: None,
            library: LibraryUi::default(),
            browser: BrowserUi::default(),
            pl_ui: PlaylistsUi::default(),
            queue_ui: QueueUi::default(),
            hist_ui: HistoryUi::default(),
            media_dirs: Vec::new(),
            db_path: None,
            scanning: false,
            scan_count: 0,
        };
        app.reload_library();
        app.reload_playlists();
        app.reload_history();
        app
    }

    /// Success/info notification (~3s).
    pub fn notify(&mut self, msg: impl Into<String>) {
        self.set_status(msg, false);
    }

    /// Error notification (~3s); `?` opens the full text.
    pub fn notify_error(&mut self, msg: impl Into<String>) {
        self.set_status(msg, true);
    }

    pub fn set_status(&mut self, msg: impl Into<String>, error: bool) {
        self.status = Some((msg.into(), error));
        self.status_until = Some(Instant::now() + STATUS_TTL);
        self.dirty = true;
    }

    /// Put the persisted queue back (missing files are skipped by the DB).
    pub fn restore_queue(&mut self) {
        match self.db.load_queue() {
            Ok(t) => {
                self.player.queue_mut().push(t);
            }
            Err(e) => self.notify_error(format!("Queue restore failed: {e}")),
        }
        views::clamp(&mut self.queue_ui.list, self.player.queue().len());
    }

    /// Save the queue, stop mpv and flush history. Call once on the way out.
    pub fn shutdown(&mut self) {
        let tracks: Vec<Track> = self
            .player
            .queue()
            .entries()
            .iter()
            .map(|e| e.track.clone())
            .collect();
        if let Err(e) = self.db.save_queue(&tracks) {
            eprintln!("rmp: could not save queue: {e}");
        }
        self.player.shutdown();
        self.sync();
    }

    pub fn start_scan(&mut self) {
        let Some(path) = self.db_path.clone() else {
            return self.notify_error("Cannot scan: library database unavailable");
        };
        if self.scanning {
            return self.notify("Scan already running");
        }
        self.scanning = true;
        self.scan_count = 0;
        scanner::spawn_scan(path, self.media_dirs.clone(), self.tx.clone());
        self.notify("Scanning…");
    }

    pub fn handle(&mut self, ev: AppEvent) {
        match ev {
            AppEvent::Key(k) => {
                self.on_key(k);
                self.sync();
            }
            AppEvent::Resize => self.dirty = true,
            AppEvent::Tick => {
                if self.status_until.is_some_and(|t| Instant::now() >= t) {
                    self.status = None;
                    self.status_until = None;
                    self.dirty = true;
                }
                self.dirty |=
                    self.player.snapshot().state == PlayState::Playing || self.status.is_some();
            }
            AppEvent::Command(c) => {
                self.command(c);
                self.sync();
            }
            AppEvent::Mpv(e) => {
                self.on_mpv(&e);
                self.sync();
            }
            AppEvent::Scan(e) => self.on_scan(e),
        }
    }

    fn on_mpv(&mut self, e: &MpvEvent) {
        self.player.handle_mpv_event(e);
    }

    fn on_scan(&mut self, e: ScanEvent) {
        match e {
            ScanEvent::Progress { scanned } => {
                self.scan_count = scanned;
                self.notify(format!("Scanning… {scanned} files"))
            }
            ScanEvent::Error(m) => self.notify_error(m),
            ScanEvent::Finished {
                added,
                updated,
                removed,
            } => {
                self.scanning = false;
                self.reload_library();
                self.notify(format!(
                    "Scan done: {added} added, {updated} updated, {removed} removed"
                ));
            }
        }
        self.dirty = true;
    }

    /// After player activity: write history, surface errors, refresh MPRIS, request redraw.
    fn sync(&mut self) {
        let mut wrote = false;
        while let Some(r) = self.player.take_finished_play() {
            match self
                .db
                .add_history(&r.track, r.started_at, r.played_secs, r.completed)
            {
                Ok(()) => wrote = true,
                Err(e) => self.notify_error(format!("History write failed: {e}")),
            }
        }
        if wrote {
            self.reload_history();
        }
        if let Some(e) = self.player.take_error() {
            self.notify_error(e);
        }
        if let Some(m) = &self.mpris {
            m.update(self.player.snapshot());
        }
        self.dirty = true;
    }

    /// Apply a command from the keyboard or MPRIS.
    pub fn command(&mut self, c: Command) {
        self.dirty = true;
        match c {
            Command::Quit => self.quit = true,
            Command::Rescan => self.start_scan(),
            Command::Enqueue(ref t) => {
                let n = t.len();
                match self.player.handle_command(&c) {
                    Ok(()) => self.notify(format!(
                        "Added {n} track{} to queue",
                        if n == 1 { "" } else { "s" }
                    )),
                    Err(e) => self.notify_error(e),
                }
                views::clamp(&mut self.queue_ui.list, self.player.queue().len());
            }
            _ => {
                match self.player.handle_command(&c) {
                    Ok(()) => {
                        let snap = self.player.snapshot();
                        match c {
                            Command::CycleRepeat | Command::SetRepeat(_) => {
                                let m = format!("Repeat {:?}", snap.repeat).to_uppercase();
                                self.notify(m)
                            }
                            Command::ToggleShuffle | Command::SetShuffle(_) => {
                                let on = if snap.shuffle { "ON" } else { "OFF" };
                                self.notify(format!("Shuffle {on}"))
                            }
                            _ => {}
                        }
                    }
                    Err(e) => self.notify_error(e),
                }
                views::clamp(&mut self.queue_ui.list, self.player.queue().len());
            }
        }
    }

    fn on_key(&mut self, k: KeyEvent) {
        if self.dialog.is_some() || self.search.is_some() {
            return self.on_input_key(k);
        }
        if self.view == 1 && k.code == KeyCode::Backspace {
            return self.view_action(Action::Left);
        }
        let Some(a) = self.keys.action(&k) else {
            return;
        };
        let cmd = match a {
            Action::ViewLibrary => return self.view = 0,
            Action::ViewFiles => return self.view = 1,
            Action::ViewPlaylists => return self.view = 2,
            Action::ViewQueue => return self.view = 3,
            Action::ViewHistory => return self.view = 4,
            Action::Quit => Command::Quit,
            Action::Rescan => Command::Rescan,
            Action::TogglePause => Command::TogglePause,
            Action::Stop => Command::Stop,
            Action::Next => Command::Next,
            Action::Prev => Command::Prev,
            Action::CycleRepeat => Command::CycleRepeat,
            Action::ToggleShuffle => Command::ToggleShuffle,
            Action::SeekBack => Command::SeekRelative(-5.0),
            Action::SeekForward => Command::SeekRelative(5.0),
            Action::VolumeDown => Command::VolumeDelta(-5),
            Action::VolumeUp => Command::VolumeDelta(5),
            Action::Search => {
                if self.view == 0 {
                    self.search = Some(self.library.filter.clone());
                }
                return;
            }
            Action::Help => {
                self.dialog = Some(match &self.status {
                    Some((m, true)) => Dialog::Error {
                        msg: m.clone(),
                        scroll: 0,
                    },
                    _ => Dialog::Help { scroll: 0 },
                });
                return;
            }
            _ => return self.view_action(a),
        };
        self.command(cmd);
    }
}

#[cfg(test)]
mod tests;
