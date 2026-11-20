//! File browser view: state, directory loading and key actions.
use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use ratatui::widgets::ListState;

use super::{Action, App, Command, clamp};
use crate::library::{Track, is_media_path, metadata};

pub struct FsEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    /// Playable by extension; unsupported files should be drawn muted.
    pub supported: bool,
    /// Size and mtime read once at load time (never in draw).
    pub size: u64,
    pub modified: Option<SystemTime>,
}

#[derive(Default)]
pub struct BrowserUi {
    pub dir: PathBuf,
    pub entries: Vec<FsEntry>,
    pub list: ListState,
    /// Set when `dir` could not be read; the view shows it inline.
    pub err: Option<String>,
    /// Item count of the selected directory, cached per path (see `dir_count`).
    count: RefCell<Option<(PathBuf, Option<usize>)>>,
}

impl BrowserUi {
    /// Number of items in `path`; read once per selection change, not per frame.
    pub fn dir_count(&self, path: &Path) -> Option<usize> {
        let mut c = self.count.borrow_mut();
        if let Some((p, n)) = c.as_ref()
            && p == path
        {
            return *n;
        }
        let n = fs::read_dir(path).ok().map(|rd| rd.count());
        *c = Some((path.to_path_buf(), n));
        n
    }
}

/// Recursively collect supported media files (sorted, symlinked dirs skipped).
fn media_files(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut paths: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    paths.sort();
    for p in paths {
        if p.is_symlink() && p.is_dir() {
            continue;
        }
        if p.is_dir() {
            if depth < 32 {
                media_files(&p, out, depth + 1);
            }
        } else if is_media_path(&p).is_some() {
            out.push(p);
        }
    }
}

impl App {
    /// Read a directory into the file browser (dirs first, then by name).
    pub fn load_dir(&mut self, dir: PathBuf) {
        let rd = match fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(e) => {
                // Stay usable: show the error inline and let h/Backspace go to the parent.
                self.browser.dir = dir;
                self.browser.entries.clear();
                self.browser.err = Some(e.to_string());
                self.browser.list = ListState::default();
                *self.browser.count.borrow_mut() = None;
                return;
            }
        };
        let mut entries: Vec<FsEntry> = rd
            .flatten()
            .map(|e| {
                let path = e.path();
                let is_dir = path.is_dir();
                let md = e.metadata().ok();
                FsEntry {
                    size: md.as_ref().map_or(0, |m| m.len()),
                    modified: md.and_then(|m| m.modified().ok()),
                    name: e.file_name().to_string_lossy().into_owned(),
                    supported: !is_dir && is_media_path(&path).is_some(),
                    path,
                    is_dir,
                }
            })
            .collect();
        entries.sort_by(|a, b| {
            (!a.is_dir, a.name.to_lowercase()).cmp(&(!b.is_dir, b.name.to_lowercase()))
        });
        self.browser.dir = dir;
        self.browser.err = None;
        *self.browser.count.borrow_mut() = None;
        self.browser.entries = entries;
        self.browser.list = ListState::default();
        clamp(&mut self.browser.list, self.browser.entries.len());
    }

    fn browser_parent(&mut self) {
        if let Some(p) = self.browser.dir.parent().map(Path::to_path_buf) {
            let from = self.browser.dir.clone();
            self.load_dir(p);
            if let Some(i) = self.browser.entries.iter().position(|e| e.path == from) {
                self.browser.list.select(Some(i));
            }
        }
    }

    /// Indexed track if known, else read tags from the file.
    fn track_for(&self, p: &Path) -> Track {
        self.db
            .track_by_path(p)
            .ok()
            .flatten()
            .unwrap_or_else(|| metadata::read_track(p))
    }

    fn browser_tracks(&self, e: &FsEntry) -> Vec<Track> {
        let mut files = Vec::new();
        if e.is_dir {
            media_files(&e.path, &mut files, 0);
        } else if e.supported {
            files.push(e.path.clone());
        }
        files.iter().map(|p| self.track_for(p)).collect()
    }

    fn browser_enter(&mut self) {
        let Some(e) = self
            .browser
            .list
            .selected()
            .and_then(|i| self.browser.entries.get(i))
        else {
            return;
        };
        if e.is_dir {
            let p = e.path.clone();
            self.load_dir(p);
        } else if e.supported {
            let tracks = vec![self.track_for(&e.path)];
            self.command(Command::PlayTracks { tracks, start: 0 });
        } else {
            self.notify_error("Unsupported file type");
        }
    }

    pub fn init_browser(&mut self) {
        let start = self
            .media_dirs
            .iter()
            .find(|d| d.is_dir())
            .cloned()
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("/"));
        self.load_dir(start);
    }

    pub(super) fn browser_action(&mut self, a: Action) {
        match a {
            Action::Left => self.browser_parent(),
            Action::Enter | Action::Right => self.browser_enter(),
            Action::Enqueue | Action::AddToPlaylist => {
                let tracks = match self
                    .browser
                    .list
                    .selected()
                    .and_then(|i| self.browser.entries.get(i))
                {
                    Some(e) if e.is_dir || e.supported => self.browser_tracks(e),
                    Some(_) => return self.notify_error("Unsupported file type"),
                    None => return,
                };
                if a == Action::Enqueue {
                    self.enqueue(tracks);
                } else if tracks.is_empty() {
                    self.notify_error("Nothing to add");
                } else {
                    self.open_picker(tracks);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;
    use crate::config::keys::KeyMap;
    use crate::library::database::Database;
    use crate::playback::Player;
    use crate::ui::theme::Theme;

    fn app() -> App {
        let (tx, _rx) = mpsc::channel();
        let db = Database::open_in_memory().unwrap();
        let mut w = Vec::new();
        let keys = KeyMap::from_config(&Default::default(), &mut w);
        App::new(tx.clone(), Player::new(tx, 100), db, keys, Theme::default())
    }

    #[test]
    fn selection_after_parent_and_error() {
        let d = tempfile::tempdir().unwrap();
        for n in ["b", "a", "z"] {
            std::fs::create_dir(d.path().join(n)).unwrap();
        }
        std::fs::write(d.path().join("a/x.mp3"), b"").unwrap();
        let mut a = app();
        a.load_dir(d.path().to_path_buf());
        assert_eq!(a.browser.entries[0].name, "a");
        a.load_dir(d.path().join("z"));
        assert!(a.browser.entries.is_empty() && a.browser.err.is_none());
        a.browser_action(Action::Left);
        assert_eq!(a.browser.list.selected(), Some(2));
        a.load_dir(d.path().join("missing"));
        assert!(a.browser.err.is_some());
        a.browser_action(Action::Left);
        assert!(a.browser.err.is_none());
        assert_eq!(a.browser.dir, d.path());
        assert_eq!(a.browser.dir_count(&d.path().join("a")), Some(1));
    }
}
