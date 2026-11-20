//! Per-view UI state and key actions; one file per view. No rendering here.
//! Shared helpers and the small dispatch live here; views never need to edit app.rs.
mod browser;
mod history;
mod library;
mod playlists;
mod queue;

use ratatui::widgets::ListState;

use super::App;
use crate::config::keys::Action;
use crate::event::Command;
use crate::library::Track;

pub use browser::BrowserUi;
pub use history::HistoryUi;
pub use library::LibraryUi;
pub use playlists::PlaylistsUi;
pub use queue::QueueUi;

/// Keep a selection valid for a list of `len` rows.
pub fn clamp(s: &mut ListState, len: usize) {
    s.select((len > 0).then(|| s.selected().unwrap_or(0).min(len - 1)));
}

fn step(s: &mut ListState, len: usize, a: Action) {
    if len == 0 {
        s.select(None);
        return;
    }
    let cur = s.selected().unwrap_or(0);
    let n = match a {
        Action::Down => cur + 1,
        Action::Up => cur.saturating_sub(1),
        Action::Top => 0,
        Action::Bottom => len - 1,
        Action::PageDown => cur + 10,
        Action::PageUp => cur.saturating_sub(10),
        _ => cur,
    };
    s.select(Some(n.min(len - 1)));
}

impl App {
    /// Add tracks to the queue with a notification (shared by all views).
    pub(super) fn enqueue(&mut self, tracks: Vec<Track>) {
        if tracks.is_empty() {
            self.notify_error("Nothing to add");
        } else {
            self.command(Command::Enqueue(tracks));
        }
    }

    /// Row count and list state of the focused pane in the current view.
    fn focused_list(&mut self) -> (&mut ListState, usize) {
        match self.view {
            0 => (&mut self.library.list, self.library.filtered.len()),
            1 => (&mut self.browser.list, self.browser.entries.len()),
            2 if self.pl_ui.focus_items => (&mut self.pl_ui.items_list, self.pl_ui.items.len()),
            2 => (&mut self.pl_ui.list, self.playlists.len()),
            3 => (&mut self.queue_ui.list, self.player.queue().len()),
            _ => (&mut self.hist_ui.list, self.history.len()),
        }
    }

    /// Keys that only mean something inside a view.
    pub fn view_action(&mut self, a: Action) {
        if matches!(
            a,
            Action::Down
                | Action::Up
                | Action::Top
                | Action::Bottom
                | Action::PageDown
                | Action::PageUp
        ) {
            let (s, n) = self.focused_list();
            step(s, n, a);
            if self.view == 2 && !self.pl_ui.focus_items {
                self.reload_playlist_items();
            }
            return;
        }
        match self.view {
            0 => self.library_action(a),
            1 => self.browser_action(a),
            2 => self.playlists_action(a),
            3 => self.queue_action(a),
            _ => self.history_action(a),
        }
    }
}
