//! Queue view: state and key actions.
use std::cell::RefCell;
use std::collections::HashMap;

use ratatui::widgets::ListState;

use super::{Action, App, Command, clamp};
use crate::app::{Confirm, Dialog};

#[derive(Default)]
pub struct QueueUi {
    pub list: ListState,
    /// Entry id -> file exists. Checked once per entry id (lazily, on first draw), never per frame.
    /// shortcut: a file that appears/disappears later isn't noticed until the entry is re-added.
    exists: RefCell<HashMap<u64, bool>>,
}

impl QueueUi {
    pub fn file_exists(&self, id: u64, path: &std::path::Path) -> bool {
        *self
            .exists
            .borrow_mut()
            .entry(id)
            .or_insert_with(|| path.exists())
    }
}

impl App {
    pub(crate) fn clear_queue(&mut self) {
        self.command(Command::Stop);
        self.player.queue_mut().clear();
        clamp(&mut self.queue_ui.list, 0);
        self.queue_ui.exists.borrow_mut().clear();
    }

    pub(super) fn queue_action(&mut self, a: Action) {
        if a == Action::ClearQueue {
            if !self.player.queue().entries().is_empty() {
                self.dialog = Some(Dialog::Confirm(Confirm::ClearQueue));
            }
            return;
        }
        let Some(i) = self.queue_ui.list.selected() else {
            return;
        };
        let Some(e) = self.player.queue().entries().get(i) else {
            return;
        };
        let (id, track) = (e.id, e.track.clone());
        match a {
            Action::Enter => self.command(Command::PlayQueueEntry(id)),
            Action::AddToPlaylist => self.open_picker(vec![track]),
            Action::Remove => {
                self.player.queue_mut().remove(id);
                self.queue_ui.exists.borrow_mut().remove(&id);
                clamp(&mut self.queue_ui.list, self.player.queue().len());
                self.notify("Removed from queue");
            }
            Action::MoveUp if self.player.queue_mut().move_up(id) => {
                self.queue_ui.list.select(Some(i - 1));
                self.notify(format!("Moved to #{i}"));
            }
            Action::MoveDown if self.player.queue_mut().move_down(id) => {
                self.queue_ui.list.select(Some(i + 1));
                self.notify(format!("Moved to #{}", i + 2));
            }
            _ => {}
        }
    }
}
