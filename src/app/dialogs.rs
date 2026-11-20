//! Modal dialog and search-input key handling plus what confirmed dialogs do.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{App, Confirm, Dialog, TextPurpose};
use crate::config::keys::Action;
use crate::library::Track;

impl App {
    /// Open the add-to-playlist picker for `tracks`.
    pub fn open_picker(&mut self, tracks: Vec<Track>) {
        if tracks.is_empty() {
            self.notify_error("Nothing to add");
        } else if self.playlists.is_empty() {
            self.notify_error("Create a playlist first (view 3, N)");
        } else {
            let sel = self.pl_ui.list.selected().unwrap_or(0);
            self.dialog = Some(Dialog::Picker { tracks, sel });
        }
    }

    /// Keys while a text input or dialog is open: never shortcuts.
    pub(super) fn on_input_key(&mut self, k: KeyEvent) {
        if let Some(d) = self.dialog.take() {
            self.dialog = self.dialog_key(d, k);
        } else {
            self.search_key(k);
        }
    }

    /// Handle a key for dialog `d`; returns the dialog to keep open, if any.
    fn dialog_key(&mut self, d: Dialog, k: KeyEvent) -> Option<Dialog> {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let act = self.keys.action(&k);
        let close =
            matches!(k.code, KeyCode::Esc | KeyCode::Char('q')) || act == Some(Action::Help);
        let delta: i32 = match act {
            Some(Action::Down) => 1,
            Some(Action::Up) => -1,
            Some(Action::PageDown) => 10,
            Some(Action::PageUp) => -10,
            _ => 0,
        };
        match d {
            Dialog::Confirm(c) => {
                if matches!(k.code, KeyCode::Char('y') | KeyCode::Enter) {
                    self.submit_confirm(c);
                }
                None
            }
            Dialog::Text(p, mut b) => match k.code {
                KeyCode::Char(c) if !ctrl => {
                    b.push(c);
                    Some(Dialog::Text(p, b))
                }
                KeyCode::Backspace => {
                    b.pop();
                    Some(Dialog::Text(p, b))
                }
                KeyCode::Enter => {
                    self.submit_text(p, b.trim());
                    None
                }
                KeyCode::Esc => None,
                _ => Some(Dialog::Text(p, b)),
            },
            Dialog::Error { msg, scroll } => {
                if close || k.code == KeyCode::Enter {
                    return None;
                }
                // shortcut: rough cap (no wrap info here); render clamps nothing, extra scroll just shows blank.
                let max = (msg.chars().count() / 20) as i32;
                let scroll = (scroll as i32 + delta).clamp(0, max) as u16;
                Some(Dialog::Error { msg, scroll })
            }
            Dialog::Help { scroll } => {
                if close {
                    return None;
                }
                let scroll = (scroll as i32 + delta).clamp(0, 60) as u16;
                Some(Dialog::Help { scroll })
            }
            Dialog::Picker { tracks, sel } => {
                if close {
                    return None;
                }
                let last = self.playlists.len().saturating_sub(1);
                let sel = match act {
                    Some(Action::Top) => 0,
                    Some(Action::Bottom) => last,
                    _ => (sel as i32 + delta).clamp(0, last as i32) as usize,
                };
                if k.code == KeyCode::Enter {
                    self.add_to_playlist(sel, &tracks);
                    return None;
                }
                Some(Dialog::Picker { tracks, sel })
            }
        }
    }

    fn add_to_playlist(&mut self, idx: usize, tracks: &[Track]) {
        let Some(p) = self.playlists.get(idx).cloned() else {
            return;
        };
        match self.db.add_to_playlist(p.id, tracks) {
            Ok(()) => {
                self.notify(format!("Added {} to {}", tracks.len(), p.name));
                self.reload_playlists();
            }
            Err(e) => self.notify_error(e.to_string()),
        }
    }

    /// Live search input in the footer bar.
    fn search_key(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let Some(buf) = &mut self.search else { return };
        match k.code {
            KeyCode::Char(c) if !ctrl => buf.push(c),
            KeyCode::Backspace => {
                buf.pop();
            }
            KeyCode::Enter => {
                self.search = None;
                return;
            }
            KeyCode::Esc => {
                self.search = None;
                self.library.filter.clear();
                self.refilter();
                return;
            }
            _ => return,
        }
        if let Some(s) = &self.search {
            self.library.filter = s.clone();
            self.refilter();
        }
    }

    /// Run a confirmed text dialog.
    pub fn submit_text(&mut self, purpose: TextPurpose, text: &str) {
        let (r, done) = match purpose {
            TextPurpose::NewPlaylist => (
                self.db.create_playlist(text).map(|_| ()),
                "Playlist created",
            ),
            TextPurpose::RenamePlaylist(id) => {
                (self.db.rename_playlist(id, text), "Playlist renamed")
            }
        };
        match r {
            Ok(()) => {
                self.reload_playlists();
                self.notify(done);
            }
            Err(e) => self.notify_error(e.to_string()),
        }
    }

    /// Run a confirmed destructive action.
    pub fn submit_confirm(&mut self, c: Confirm) {
        let (r, done) = match c {
            Confirm::ClearQueue => {
                self.clear_queue();
                return self.notify("Queue cleared");
            }
            Confirm::ClearHistory => (
                self.db.clear_history().map(|()| self.reload_history()),
                "History cleared",
            ),
            Confirm::DeletePlaylist(id) => (
                self.db
                    .delete_playlist(id)
                    .map(|()| self.reload_playlists()),
                "Playlist deleted",
            ),
        };
        match r {
            Ok(()) => self.notify(done),
            Err(e) => self.notify_error(e.to_string()),
        }
    }
}
