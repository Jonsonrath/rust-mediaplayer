//! History view: state and key actions.
use ratatui::widgets::ListState;

use super::{Action, App, Command, clamp};
use crate::app::{Confirm, Dialog};
use crate::library::database::HistoryEntry;

pub struct HistoryUi {
    pub list: ListState,
    /// Local UTC offset in seconds, read once at startup.
    pub tz_offset: i64,
}

impl Default for HistoryUi {
    fn default() -> Self {
        Self {
            list: ListState::default(),
            tz_offset: local_offset_secs(),
        }
    }
}

/// shortcut: std has no timezone support, so ask `date +%z` once at startup (falls back to UTC).
/// DST changes while running are not tracked; restart to pick them up.
fn local_offset_secs() -> i64 {
    std::process::Command::new("date")
        .arg("+%z")
        .output()
        .ok()
        .and_then(|o| parse_offset(String::from_utf8_lossy(&o.stdout).trim()))
        .unwrap_or(0)
}

/// "+0530" / "-0800" -> seconds.
pub fn parse_offset(s: &str) -> Option<i64> {
    let (sign, d) = match s.split_at_checked(1)? {
        ("+", d) => (1, d),
        ("-", d) => (-1, d),
        _ => return None,
    };
    if d.len() != 4 || !d.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (h, m): (i64, i64) = (d[..2].parse().ok()?, d[2..].parse().ok()?);
    Some(sign * (h * 3600 + m * 60))
}

impl App {
    pub fn reload_history(&mut self) {
        match self.db.history(500) {
            Ok(h) => self.history = h,
            Err(e) => self.notify_error(format!("History load failed: {e}")),
        }
        clamp(&mut self.hist_ui.list, self.history.len());
    }

    fn selected_history(&self) -> Option<&HistoryEntry> {
        self.history.get(self.hist_ui.list.selected()?)
    }

    pub(super) fn history_action(&mut self, a: Action) {
        match a {
            Action::Enter => {
                if let Some(h) = self.selected_history() {
                    let tracks = vec![h.track.clone()];
                    self.command(Command::PlayTracks { tracks, start: 0 });
                }
            }
            Action::AddToPlaylist => {
                if let Some(h) = self.selected_history() {
                    let tracks = vec![h.track.clone()];
                    self.open_picker(tracks);
                }
            }
            Action::Delete => self.dialog = Some(Dialog::Confirm(Confirm::ClearHistory)),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets() {
        assert_eq!(parse_offset("+0530"), Some(19800));
        assert_eq!(parse_offset("-0800"), Some(-28800));
        assert_eq!(parse_offset("UTC"), None);
        assert_eq!(parse_offset(""), None);
    }
}
