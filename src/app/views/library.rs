//! Library view: state, grouping, filtering and key actions.
use std::cell::Cell;

use ratatui::widgets::ListState;

use super::{Action, App, Command, clamp};
use crate::library::Track;

/// Which pane has focus. Panes hidden by the current breakpoint fall back to `Tracks`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Focus {
    Artists,
    Albums,
    #[default]
    Tracks,
}

pub fn artist_of(t: &Track) -> &str {
    t.artist
        .as_deref()
        .or(t.album_artist.as_deref())
        .unwrap_or("Unknown artist")
}

pub fn album_of(t: &Track) -> &str {
    t.album.as_deref().unwrap_or("Unknown album")
}

/// Artist/album groups; rebuilt only when the track list changes, never per frame.
#[derive(Default)]
pub struct Groups {
    /// Sorted unique artists.
    pub artists: Vec<String>,
    /// Sorted unique (artist, album) pairs.
    pairs: Vec<(String, String)>,
    /// `tracks.len()` this was built for.
    n: usize,
    pub has_video: bool,
}

impl Groups {
    pub fn build(tracks: &[Track]) -> Self {
        let mut p: Vec<(&str, &str)> = tracks.iter().map(|t| (artist_of(t), album_of(t))).collect();
        p.sort_by_cached_key(|&(a, b)| (a.to_lowercase(), b.to_lowercase(), a, b));
        p.dedup();
        let mut artists: Vec<String> = Vec::new();
        for (a, _) in &p {
            if artists.last().is_none_or(|l| l != a) {
                artists.push((*a).to_string());
            }
        }
        Self {
            artists,
            pairs: p.iter().map(|&(a, b)| (a.into(), b.into())).collect(),
            n: tracks.len(),
            has_video: tracks.iter().any(|t| t.is_video),
        }
    }

    /// Unique album names of `artist` (all artists when `None`), sorted.
    pub fn albums_of(&self, artist: Option<&str>) -> Vec<String> {
        let mut v: Vec<&str> = self
            .pairs
            .iter()
            .filter(|(a, _)| artist.is_none_or(|x| x == a))
            .map(|(_, b)| b.as_str())
            .collect();
        v.sort_by_cached_key(|s| (s.to_lowercase(), *s));
        v.dedup();
        v.into_iter().map(String::from).collect()
    }
}

pub struct LibraryUi {
    /// Track list state (rows = `filtered`).
    pub list: ListState,
    pub artist_list: ListState,
    pub album_list: ListState,
    pub focus: Focus,
    /// Live search text (case-insensitive substring over title/artist/album/path).
    pub filter: String,
    /// Indices into `App::tracks` matching artist + album + `filter`.
    pub filtered: Vec<usize>,
    pub groups: Groups,
    /// Albums of the selected artist (row 0 of the pane is "All albums").
    pub albums: Vec<String>,
    albums_for: Option<String>,
    last_sel: (Option<usize>, Option<usize>),
    /// Panes the last draw showed (3/2/1); lets key handling skip hidden panes.
    pub shown: Cell<usize>,
}

impl Default for LibraryUi {
    fn default() -> Self {
        Self {
            list: ListState::default(),
            artist_list: ListState::default(),
            album_list: ListState::default(),
            focus: Focus::Tracks,
            filter: String::new(),
            filtered: Vec::new(),
            groups: Groups::default(),
            albums: Vec::new(),
            albums_for: None,
            last_sel: (None, None),
            shown: Cell::new(3),
        }
    }
}

impl LibraryUi {
    pub fn sel_artist(&self) -> Option<&str> {
        let i = self.artist_list.selected().filter(|&i| i > 0)?;
        self.groups.artists.get(i - 1).map(String::as_str)
    }

    pub fn sel_album(&self) -> Option<&str> {
        let i = self.album_list.selected().filter(|&i| i > 0)?;
        self.albums.get(i - 1).map(String::as_str)
    }

    fn panes(&self) -> &'static [Focus] {
        match self.shown.get() {
            3.. => &[Focus::Artists, Focus::Albums, Focus::Tracks],
            2 => &[Focus::Artists, Focus::Tracks],
            _ => &[Focus::Tracks],
        }
    }

    /// (artists, albums, tracks) focus flags.
    pub fn focused(&self) -> (bool, bool, bool) {
        let f = self.eff_focus();
        (f == Focus::Artists, f == Focus::Albums, f == Focus::Tracks)
    }

    /// Focus actually shown (a hidden pane counts as `Tracks`).
    pub fn eff_focus(&self) -> Focus {
        if self.panes().contains(&self.focus) {
            self.focus
        } else {
            Focus::Tracks
        }
    }
}

impl App {
    pub fn reload_library(&mut self) {
        match self.db.all_tracks() {
            Ok(t) => self.tracks = t,
            Err(e) => self.notify_error(format!("Library load failed: {e}")),
        }
        self.refilter();
    }

    /// Recompute groups (only if the tracks changed), albums and the filtered track list.
    pub fn refilter(&mut self) {
        if self.library.groups.n != self.tracks.len() {
            self.library.groups = Groups::build(&self.tracks);
        }
        let ui = &mut self.library;
        clamp(&mut ui.artist_list, ui.groups.artists.len() + 1);
        let artist = ui.sel_artist().map(String::from);
        ui.albums = ui.groups.albums_of(artist.as_deref());
        if ui.albums_for != artist {
            ui.albums_for = artist.clone();
            ui.album_list.select(Some(0));
        }
        clamp(&mut ui.album_list, ui.albums.len() + 1);
        let album = ui.sel_album().map(String::from);
        let q = ui.filter.to_lowercase();
        ui.filtered = self
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                artist.as_deref().is_none_or(|a| artist_of(t) == a)
                    && album.as_deref().is_none_or(|a| album_of(t) == a)
                    && (q.is_empty()
                        || [Some(&t.title), t.artist.as_ref(), t.album.as_ref()]
                            .into_iter()
                            .flatten()
                            .any(|s| s.to_lowercase().contains(&q))
                        || t.path.to_string_lossy().to_lowercase().contains(&q))
            })
            .map(|(i, _)| i)
            .collect();
        let sel = (ui.artist_list.selected(), ui.album_list.selected());
        if ui.last_sel != sel {
            ui.last_sel = sel;
            ui.list.select(Some(0));
        }
        clamp(&mut ui.list, ui.filtered.len());
    }

    #[cfg(test)]
    pub(super) fn library_focused(&mut self) -> (&mut ListState, usize) {
        let ui = &mut self.library;
        match ui.eff_focus() {
            Focus::Artists => (&mut ui.artist_list, ui.groups.artists.len() + 1),
            Focus::Albums => (&mut ui.album_list, ui.albums.len() + 1),
            Focus::Tracks => (&mut ui.list, ui.filtered.len()),
        }
    }

    fn selected_library_track(&self) -> Option<&Track> {
        let i = *self.library.filtered.get(self.library.list.selected()?)?;
        self.tracks.get(i)
    }

    /// What `a` / `A` act on for the focused pane: a track, or every track of the artist/album.
    fn library_selection(&self, focus: Focus) -> Vec<Track> {
        let ui = &self.library;
        let (artist, album) = match focus {
            Focus::Tracks => return self.selected_library_track().cloned().into_iter().collect(),
            Focus::Artists => (ui.sel_artist(), None),
            Focus::Albums => (ui.sel_artist(), ui.sel_album()),
        };
        self.tracks
            .iter()
            .filter(|t| {
                artist.is_none_or(|a| artist_of(t) == a) && album.is_none_or(|a| album_of(t) == a)
            })
            .cloned()
            .collect()
    }

    pub(super) fn library_action(&mut self, a: Action) {
        let focus = self.library.eff_focus();
        let panes = self.library.panes();
        let pos = panes.iter().position(|&p| p == focus).unwrap_or(0);
        match a {
            Action::Left => self.library.focus = panes[pos.saturating_sub(1)],
            Action::Right => self.library.focus = panes[(pos + 1).min(panes.len() - 1)],
            Action::FocusNext => self.library.focus = panes[(pos + 1) % panes.len()],
            Action::Enter if focus != Focus::Tracks => {
                self.library.focus = panes[(pos + 1).min(panes.len() - 1)];
            }
            Action::Enter => {
                let Some(start) = self.library.list.selected() else {
                    return;
                };
                let tracks = self
                    .library
                    .filtered
                    .iter()
                    .map(|&i| self.tracks[i].clone())
                    .collect();
                self.command(Command::PlayTracks { tracks, start });
            }
            Action::Enqueue => {
                let t = self.library_selection(focus);
                self.enqueue(t);
            }
            Action::AddToPlaylist => {
                let t = self.library_selection(focus);
                if t.is_empty() {
                    self.notify_error("Nothing to add");
                } else {
                    self.open_picker(t);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn tr(n: usize, artist: &str, album: &str) -> Track {
        Track {
            id: 0,
            path: PathBuf::from(format!("/x/{n}.mp3")),
            title: format!("t{n}"),
            artist: Some(artist.into()),
            album: Some(album.into()),
            album_artist: None,
            genre: None,
            track_no: None,
            year: None,
            duration: None,
            format: None,
            size: 0,
            width: None,
            height: None,
            is_video: false,
        }
    }

    fn app() -> App {
        use crate::config::keys::KeyMap;
        use crate::library::database::Database;
        use crate::playback::Player;
        use crate::ui::theme::Theme;
        let (tx, _rx) = std::sync::mpsc::channel();
        let db = Database::open_in_memory().unwrap();
        let keys = KeyMap::from_config(&Default::default(), &mut Vec::new());
        let mut a = App::new(tx.clone(), Player::new(tx, 100), db, keys, Theme::default());
        a.tracks = vec![
            tr(1, "beta", "B1"),
            tr(2, "Alpha", "A2"),
            tr(3, "Alpha", "A1"),
            tr(4, "Alpha", "A1"),
        ];
        a.refilter();
        a
    }

    #[test]
    fn grouping_sorted_unique() {
        let g = Groups::build(&app().tracks);
        assert_eq!(g.artists, ["Alpha", "beta"]);
        assert_eq!(g.albums_of(Some("Alpha")), ["A1", "A2"]);
        assert_eq!(g.albums_of(None), ["A1", "A2", "B1"]);
    }

    #[test]
    fn artist_selection_filters_and_resets_tracks() {
        let mut a = app();
        assert_eq!(a.library.filtered.len(), 4);
        a.library.list.select(Some(3));
        a.library.focus = Focus::Artists;
        let (s, n) = a.library_focused();
        super::super::step(s, n, Action::Down);
        a.refilter();
        assert_eq!(a.library.filtered, [1, 2, 3]);
        assert_eq!(a.library.list.selected(), Some(0));
        assert_eq!(a.library.albums, ["A1", "A2"]);
        // album pick narrows further
        a.library.focus = Focus::Albums;
        let (s, n) = a.library_focused();
        super::super::step(s, n, Action::Down);
        a.refilter();
        assert_eq!(a.library.filtered, [2, 3]);
        assert_eq!(a.library_selection(Focus::Artists).len(), 3);
        assert_eq!(a.library_selection(Focus::Albums).len(), 2);
    }

    #[test]
    fn focus_moves_and_hidden_panes_fall_back() {
        let mut a = app();
        a.library_action(Action::Left);
        a.library_action(Action::Left);
        assert_eq!(a.library.focus, Focus::Artists);
        a.library_action(Action::Enter);
        assert_eq!(a.library.focus, Focus::Albums);
        a.library.shown.set(1);
        assert_eq!(a.library.eff_focus(), Focus::Tracks);
    }

    #[test]
    fn search_keeps_selection_clamped() {
        let mut a = app();
        a.library.list.select(Some(3));
        a.library.filter = "t1".into();
        a.refilter();
        assert_eq!(a.library.filtered, [0]);
        assert_eq!(a.library.list.selected(), Some(0));
    }
}
