//! Playlists view: state and key actions.
use ratatui::widgets::ListState;

use super::{Action, App, Command, clamp};
use crate::app::{Confirm, Dialog, TextPurpose};
use crate::library::Track;
use crate::library::database::{Playlist, PlaylistItem};

#[derive(Default)]
pub struct PlaylistsUi {
    pub list: ListState,
    pub items: Vec<PlaylistItem>,
    pub items_list: ListState,
    /// True when the contents pane has focus.
    pub focus_items: bool,
}

impl App {
    pub fn reload_playlists(&mut self) {
        match self.db.playlists() {
            Ok(p) => self.playlists = p,
            Err(e) => self.notify_error(format!("Playlists load failed: {e}")),
        }
        clamp(&mut self.pl_ui.list, self.playlists.len());
        self.reload_playlist_items();
    }

    pub fn reload_playlist_items(&mut self) {
        self.pl_ui.items = match self
            .selected_playlist()
            .map(|p| self.db.playlist_items(p.id))
        {
            Some(Ok(i)) => i,
            Some(Err(e)) => {
                self.notify_error(format!("Playlist load failed: {e}"));
                Vec::new()
            }
            None => Vec::new(),
        };
        clamp(&mut self.pl_ui.items_list, self.pl_ui.items.len());
    }

    pub fn selected_playlist(&self) -> Option<&Playlist> {
        self.playlists.get(self.pl_ui.list.selected()?)
    }

    pub(super) fn playlists_action(&mut self, a: Action) {
        let items_focus = self.pl_ui.focus_items;
        match a {
            Action::FocusNext => self.pl_ui.focus_items = !items_focus,
            Action::Right => self.pl_ui.focus_items = true,
            Action::Left => self.pl_ui.focus_items = false,
            Action::NewPlaylist => {
                self.dialog = Some(Dialog::Text(TextPurpose::NewPlaylist, String::new()))
            }
            Action::Rename => {
                if let Some(p) = self.selected_playlist() {
                    self.dialog = Some(Dialog::Text(
                        TextPurpose::RenamePlaylist(p.id),
                        p.name.clone(),
                    ));
                }
            }
            Action::Delete => {
                if let Some(p) = self.selected_playlist() {
                    self.dialog = Some(Dialog::Confirm(Confirm::DeletePlaylist(p.id)));
                }
            }
            Action::Enter | Action::Enqueue => {
                let tracks: Vec<Track> = self.pl_ui.items.iter().map(|i| i.track.clone()).collect();
                let sel = self.pl_ui.items_list.selected().unwrap_or(0);
                match (a, items_focus) {
                    (Action::Enter, true) => {
                        self.command(Command::PlayTracks { tracks, start: sel })
                    }
                    (Action::Enter, false) if !tracks.is_empty() => {
                        self.command(Command::PlayTracks { tracks, start: 0 })
                    }
                    (Action::Enqueue, true) => {
                        self.enqueue(tracks.get(sel).cloned().into_iter().collect())
                    }
                    (Action::Enqueue, false) => self.enqueue(tracks),
                    _ => {}
                }
            }
            Action::Remove | Action::MoveUp | Action::MoveDown if items_focus => {
                let Some(i) = self.pl_ui.items_list.selected() else {
                    return;
                };
                let id = self.pl_ui.items[i].item_id;
                let r = match a {
                    Action::Remove => self.db.remove_playlist_item(id),
                    _ => self
                        .db
                        .move_playlist_item(id, a == Action::MoveUp)
                        .map(|moved| {
                            if moved {
                                let n = if a == Action::MoveUp {
                                    i.saturating_sub(1)
                                } else {
                                    i + 1
                                };
                                self.pl_ui.items_list.select(Some(n));
                            }
                        }),
                };
                match r {
                    Ok(()) => {
                        let keep = self.pl_ui.items_list.selected();
                        self.reload_playlists();
                        if keep.is_some() {
                            self.pl_ui.items_list.select(keep);
                            clamp(&mut self.pl_ui.items_list, self.pl_ui.items.len());
                        }
                        match a {
                            Action::Remove => self.notify("Removed from playlist"),
                            Action::MoveUp => self.notify("Moved up"),
                            _ => self.notify("Moved down"),
                        }
                    }
                    Err(e) => self.notify_error(e.to_string()),
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

    fn track(title: &str) -> Track {
        Track {
            id: 0,
            path: format!("/nonexistent/rmp-test/{title}.mp3").into(),
            title: title.into(),
            artist: Some("Artist".into()),
            album: None,
            album_artist: None,
            genre: None,
            track_no: None,
            year: None,
            duration: Some(61.0),
            format: None,
            size: 0,
            width: None,
            height: None,
            is_video: false,
        }
    }

    fn app() -> App {
        let (tx, _rx) = mpsc::channel();
        let db = Database::open_in_memory().unwrap();
        let mut w = Vec::new();
        let keys = KeyMap::from_config(&Default::default(), &mut w);
        App::new(tx.clone(), Player::new(tx, 100), db, keys, Theme::default())
    }

    #[test]
    fn move_updates_selection_and_notifies() {
        let mut a = app();
        let pid = a.db.create_playlist("p").unwrap();
        a.db.add_to_playlist(pid, &[track("a"), track("b"), track("c")])
            .unwrap();
        a.reload_playlists();
        a.pl_ui.focus_items = true;
        a.pl_ui.items_list.select(Some(2));
        a.playlists_action(Action::MoveUp);
        assert_eq!(a.pl_ui.items_list.selected(), Some(1));
        assert_eq!(a.pl_ui.items[1].track.title, "c");
        a.playlists_action(Action::Remove);
        assert_eq!(a.pl_ui.items.len(), 2);
        assert_eq!(a.pl_ui.items_list.selected(), Some(1));
    }
}
