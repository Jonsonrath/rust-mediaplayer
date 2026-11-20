//! Play queue: ordered entries, current entry, shuffle order and prev history.
//!
//! Behaviour when the queue changes during playback:
//! - Removing the current entry keeps it as `current()` (it keeps playing); the
//!   following `next` continues from where it used to be (the entry that followed it).
//! - Entries added while shuffle is on join the remaining shuffle order at random places.
//! - Turning shuffle off continues in queue order from the current entry's position.
//! - A user skip (`user_initiated`) in Repeat One moves on and wraps like Repeat All.
use rand::RngExt;
use rand::seq::SliceRandom;
use std::collections::VecDeque;

use super::modes::RepeatMode;
use crate::library::Track;

#[derive(Debug, Clone, PartialEq)]
pub struct QueueEntry {
    /// Unique per queue occurrence; never reused.
    pub id: u64,
    pub track: Track,
}

#[derive(Debug, Default)]
pub struct Queue {
    entries: Vec<QueueEntry>,
    next_id: u64,
    /// Kept as a copy so it survives removal from `entries`.
    current: Option<QueueEntry>,
    /// Id of the entry that followed the current one when the current one was removed.
    resume: Option<u64>,
    shuffle: bool,
    /// Shuffle: entries not yet played this cycle, in play order.
    upcoming: VecDeque<u64>,
    /// Entries played before the current one, most recent last (for `prev`).
    history: Vec<u64>,
}

impl Queue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn entries(&self) -> &[QueueEntry] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn current(&self) -> Option<&QueueEntry> {
        self.current.as_ref()
    }

    #[cfg(test)]
    pub fn shuffle(&self) -> bool {
        self.shuffle
    }

    /// Index of the current entry in `entries`, if it is still queued.
    pub fn current_index(&self) -> Option<usize> {
        self.current.as_ref().and_then(|c| self.index_of(c.id))
    }

    fn index_of(&self, id: u64) -> Option<usize> {
        self.entries.iter().position(|e| e.id == id)
    }

    /// Append tracks; returns their new entry ids.
    pub fn push(&mut self, tracks: Vec<Track>) -> Vec<u64> {
        let mut ids = Vec::with_capacity(tracks.len());
        for track in tracks {
            let id = self.next_id;
            self.next_id += 1;
            self.entries.push(QueueEntry { id, track });
            if self.shuffle {
                let at = rand::rng().random_range(0..=self.upcoming.len());
                self.upcoming.insert(at, id);
            }
            ids.push(id);
        }
        ids
    }

    pub fn remove(&mut self, id: u64) {
        let Some(idx) = self.index_of(id) else { return };
        // No successor yet: point at the id the next push will get, so entries
        // appended later still follow the removed current entry.
        let successor = Some(self.entries.get(idx + 1).map_or(self.next_id, |e| e.id));
        if self.current.as_ref().is_some_and(|c| c.id == id) || self.resume == Some(id) {
            self.resume = successor;
        }
        self.entries.remove(idx);
        self.upcoming.retain(|&i| i != id);
        self.history.retain(|&i| i != id);
    }

    /// Returns true if the entry moved.
    pub fn move_up(&mut self, id: u64) -> bool {
        match self.index_of(id) {
            Some(i) if i > 0 => {
                self.entries.swap(i, i - 1);
                true
            }
            _ => false,
        }
    }

    /// Returns true if the entry moved.
    pub fn move_down(&mut self, id: u64) -> bool {
        match self.index_of(id) {
            Some(i) if i + 1 < self.entries.len() => {
                self.entries.swap(i, i + 1);
                true
            }
            _ => false,
        }
    }

    /// Remove everything, including the current entry.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.current = None;
        self.resume = None;
        self.upcoming.clear();
        self.history.clear();
    }

    /// Make `id` current (user jump). Returns false if it is not queued.
    pub fn set_current(&mut self, id: u64) -> bool {
        let Some(idx) = self.index_of(id) else {
            return false;
        };
        if self.current.as_ref().is_some_and(|c| c.id != id) {
            self.remember_current();
        }
        self.upcoming.retain(|&i| i != id);
        self.current = Some(self.entries[idx].clone());
        self.resume = None;
        true
    }

    pub fn set_shuffle(&mut self, on: bool) {
        if on == self.shuffle {
            return;
        }
        self.shuffle = on;
        self.history.clear();
        self.upcoming.clear();
        if on {
            let cur = self.current.as_ref().map(|c| c.id);
            let mut ids: Vec<u64> = self
                .entries
                .iter()
                .map(|e| e.id)
                .filter(|&i| Some(i) != cur)
                .collect();
            ids.shuffle(&mut rand::rng());
            self.upcoming = ids.into();
        }
    }

    /// Advance to the next entry and make it current. `None` means the queue ended
    /// (or is empty); the current entry is then left unchanged.
    pub fn next(&mut self, repeat: RepeatMode, user_initiated: bool) -> Option<&QueueEntry> {
        if repeat == RepeatMode::One && !user_initiated {
            return self.current.as_ref();
        }
        let wrap = repeat != RepeatMode::Off;
        let id = if self.shuffle {
            self.next_shuffled(wrap)?
        } else {
            let len = self.entries.len();
            let idx = match (&self.current, self.current_index()) {
                (_, Some(i)) => i + 1,
                (Some(_), None) => self.resume.and_then(|r| self.index_of(r)).unwrap_or(len),
                (None, None) => 0,
            };
            match idx {
                i if i < len => self.entries[i].id,
                _ if wrap && len > 0 => self.entries[0].id,
                _ => return None,
            }
        };
        self.remember_current();
        self.upcoming.retain(|&i| i != id);
        self.set_current_raw(id)
    }

    /// Push the current entry onto the prev history, unless it was removed from the
    /// queue (`prev` could not return to it).
    fn remember_current(&mut self) {
        if let Some(id) = self.current.as_ref().map(|c| c.id)
            && self.index_of(id).is_some()
        {
            self.history.push(id);
        }
    }

    fn set_current_raw(&mut self, id: u64) -> Option<&QueueEntry> {
        let idx = self.index_of(id)?;
        self.current = Some(self.entries[idx].clone());
        self.resume = None;
        self.current.as_ref()
    }

    fn next_shuffled(&mut self, wrap: bool) -> Option<u64> {
        if self.upcoming.is_empty() {
            if !wrap || self.entries.is_empty() {
                return None;
            }
            // New cycle: reshuffle, but never start it with the track that just played.
            let mut ids: Vec<u64> = self.entries.iter().map(|e| e.id).collect();
            ids.shuffle(&mut rand::rng());
            let cur = self.current.as_ref().map(|c| c.id);
            if ids.len() > 1 && ids[0] == cur.unwrap_or(u64::MAX) {
                ids.swap(0, 1);
            }
            self.upcoming = ids.into();
        }
        self.upcoming.front().copied()
    }

    /// Go back: previous played entry when shuffled, previous index otherwise.
    /// `None` if there is nothing before the current entry.
    pub fn prev(&mut self, repeat: RepeatMode) -> Option<&QueueEntry> {
        let id = if self.shuffle {
            let id = self.history.pop()?;
            if let Some(cur) = &self.current {
                self.upcoming.push_front(cur.id);
            }
            id
        } else {
            let len = self.entries.len();
            let idx = match (self.current_index(), &self.current) {
                (Some(i), _) => i,
                (None, Some(_)) => self.resume.and_then(|r| self.index_of(r)).unwrap_or(len),
                (None, None) => return None,
            };
            match idx {
                0 if repeat != RepeatMode::Off && len > 0 => self.entries[len - 1].id,
                0 => return None,
                i => self.entries[i - 1].id,
            }
        };
        self.set_current_raw(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn track(name: &str) -> Track {
        Track {
            id: 0,
            path: PathBuf::from(name),
            title: name.into(),
            artist: None,
            album: None,
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

    fn queue_of(n: usize) -> (Queue, Vec<u64>) {
        let mut q = Queue::new();
        let ids = q.push((0..n).map(|i| track(&format!("t{i}"))).collect());
        (q, ids)
    }

    fn cur(q: &Queue) -> u64 {
        q.current().unwrap().id
    }

    use RepeatMode::{All, Off, One};

    #[test]
    fn empty_queue() {
        let mut q = Queue::new();
        assert!(q.next(Off, false).is_none());
        assert!(q.next(All, true).is_none());
        assert!(q.prev(Off).is_none());
        assert!(q.current().is_none());
        q.set_shuffle(true);
        assert!(q.next(All, false).is_none());
    }

    #[test]
    fn ids_are_unique_for_repeated_tracks() {
        let mut q = Queue::new();
        let ids = q.push(vec![track("a"), track("a")]);
        assert_ne!(ids[0], ids[1]);
        q.remove(ids[1]);
        let more = q.push(vec![track("a")]);
        assert!(!ids.contains(&more[0]));
    }

    #[test]
    fn next_without_current_starts_at_first() {
        let (mut q, ids) = queue_of(3);
        assert_eq!(q.next(Off, false).unwrap().id, ids[0]);
    }

    #[test]
    fn repeat_off_stops_at_end() {
        let (mut q, ids) = queue_of(2);
        q.set_current(ids[1]);
        assert!(q.next(Off, false).is_none());
        assert_eq!(cur(&q), ids[1]);
    }

    #[test]
    fn repeat_all_wraps() {
        let (mut q, ids) = queue_of(2);
        q.set_current(ids[1]);
        assert_eq!(q.next(All, false).unwrap().id, ids[0]);
        assert_eq!(q.prev(All).unwrap().id, ids[1]);
    }

    #[test]
    fn repeat_one_auto_repeats_but_user_skips() {
        let (mut q, ids) = queue_of(3);
        q.set_current(ids[0]);
        assert_eq!(q.next(One, false).unwrap().id, ids[0]);
        assert_eq!(q.next(One, true).unwrap().id, ids[1]);
        q.set_current(ids[2]);
        assert_eq!(q.next(One, true).unwrap().id, ids[0]);
    }

    #[test]
    fn repeat_one_ignores_shuffle() {
        let (mut q, ids) = queue_of(5);
        q.set_shuffle(true);
        q.set_current(ids[2]);
        for _ in 0..5 {
            assert_eq!(q.next(One, false).unwrap().id, ids[2]);
        }
    }

    #[test]
    fn in_order_prev() {
        let (mut q, ids) = queue_of(3);
        q.set_current(ids[2]);
        assert_eq!(q.prev(Off).unwrap().id, ids[1]);
        assert_eq!(q.prev(Off).unwrap().id, ids[0]);
        assert!(q.prev(Off).is_none());
    }

    #[test]
    fn shuffle_covers_all_before_repeating() {
        let (mut q, ids) = queue_of(6);
        q.set_current(ids[0]);
        q.set_shuffle(true);
        let mut seen = vec![ids[0]];
        for _ in 0..5 {
            seen.push(q.next(Off, false).unwrap().id);
        }
        seen.sort();
        assert_eq!(seen, ids);
        assert!(q.next(Off, false).is_none());
    }

    #[test]
    fn shuffle_all_never_replays_immediately_and_cycles() {
        let (mut q, ids) = queue_of(3);
        q.set_shuffle(true);
        q.set_current(ids[0]);
        let mut last = cur(&q);
        // Finish the first (partial) cycle; full cycles follow.
        for _ in 0..2 {
            last = q.next(All, false).unwrap().id;
        }
        for cycle in 0..20 {
            let mut seen = Vec::new();
            for _ in 0..3 {
                let id = q.next(All, false).unwrap().id;
                assert_ne!(id, last, "immediate replay in cycle {cycle}");
                last = id;
                seen.push(id);
            }
            seen.sort();
            assert_eq!(seen, ids, "cycle {cycle} must cover every entry once");
        }
    }

    #[test]
    fn shuffle_single_entry_all_repeats_it() {
        let (mut q, ids) = queue_of(1);
        q.set_shuffle(true);
        q.set_current(ids[0]);
        assert_eq!(q.next(All, false).unwrap().id, ids[0]);
    }

    #[test]
    fn shuffle_prev_follows_play_order() {
        let (mut q, ids) = queue_of(5);
        q.set_shuffle(true);
        q.set_current(ids[0]);
        let a = q.next(Off, false).unwrap().id;
        let b = q.next(Off, false).unwrap().id;
        assert_eq!(q.prev(Off).unwrap().id, a);
        assert_eq!(q.prev(Off).unwrap().id, ids[0]);
        assert!(q.prev(Off).is_none());
        // Forward again retraces the same order.
        assert_eq!(q.next(Off, false).unwrap().id, a);
        assert_eq!(q.next(Off, false).unwrap().id, b);
    }

    #[test]
    fn shuffle_off_continues_from_current_index() {
        let (mut q, ids) = queue_of(5);
        q.set_shuffle(true);
        q.set_current(ids[3]);
        q.set_shuffle(false);
        assert_eq!(q.next(Off, false).unwrap().id, ids[4]);
        assert_eq!(q.prev(Off).unwrap().id, ids[3]);
        assert_eq!(q.prev(Off).unwrap().id, ids[2]);
    }

    #[test]
    fn added_entries_join_shuffle_order() {
        let (mut q, ids) = queue_of(2);
        q.set_shuffle(true);
        q.set_current(ids[0]);
        let new = q.push(vec![track("x"), track("y")]);
        let mut seen = Vec::new();
        while let Some(e) = q.next(Off, false) {
            seen.push(e.id);
        }
        seen.sort();
        let mut want = vec![ids[1], new[0], new[1]];
        want.sort();
        assert_eq!(seen, want);
    }

    #[test]
    fn reorder() {
        let (mut q, ids) = queue_of(3);
        assert!(!q.move_up(ids[0]));
        assert!(!q.move_down(ids[2]));
        assert!(q.move_up(ids[1]));
        assert!(q.move_down(ids[1]));
        assert!(q.move_down(ids[0]));
        let order: Vec<u64> = q.entries().iter().map(|e| e.id).collect();
        assert_eq!(order, vec![ids[1], ids[0], ids[2]]);
        assert!(!q.move_up(999));
    }

    #[test]
    fn remove_other_entry() {
        let (mut q, ids) = queue_of(3);
        q.set_current(ids[0]);
        q.remove(ids[1]);
        assert_eq!(q.next(Off, false).unwrap().id, ids[2]);
        assert_eq!(q.len(), 2);
    }

    #[test]
    fn remove_current_keeps_it_and_continues_from_its_place() {
        let (mut q, ids) = queue_of(4);
        q.set_current(ids[1]);
        q.remove(ids[1]);
        assert_eq!(cur(&q), ids[1]);
        assert_eq!(q.current_index(), None);
        assert_eq!(q.next(Off, false).unwrap().id, ids[2]);
    }

    #[test]
    fn remove_current_then_following_entry() {
        let (mut q, ids) = queue_of(4);
        q.set_current(ids[1]);
        q.remove(ids[1]);
        q.remove(ids[2]);
        assert_eq!(q.next(Off, false).unwrap().id, ids[3]);
    }

    #[test]
    fn remove_current_prev_goes_before_its_place() {
        let (mut q, ids) = queue_of(4);
        q.set_current(ids[2]);
        q.remove(ids[2]);
        assert_eq!(q.prev(Off).unwrap().id, ids[1]);
    }

    #[test]
    fn remove_last_current_ends_or_wraps() {
        let (mut q, ids) = queue_of(2);
        q.set_current(ids[1]);
        q.remove(ids[1]);
        assert!(q.next(Off, false).is_none());
        assert_eq!(q.next(All, false).unwrap().id, ids[0]);
    }

    #[test]
    fn remove_current_in_shuffle_continues_shuffle() {
        let (mut q, ids) = queue_of(3);
        q.set_shuffle(true);
        q.set_current(ids[0]);
        q.remove(ids[0]);
        let a = q.next(Off, false).unwrap().id;
        let b = q.next(Off, false).unwrap().id;
        assert_ne!(a, b);
        assert!(!q.entries().iter().any(|e| e.id == ids[0]));
        assert!(q.next(Off, false).is_none());
    }

    #[test]
    fn remove_last_current_then_enqueue_continues_with_new_entry() {
        let (mut q, ids) = queue_of(2);
        q.set_current(ids[1]);
        q.remove(ids[1]);
        let new = q.push(vec![track("x")]);
        assert_eq!(q.next(Off, false).unwrap().id, new[0]);
    }

    #[test]
    fn shuffle_prev_after_removed_current_does_not_corrupt_order() {
        let (mut q, ids) = queue_of(3);
        q.set_shuffle(true);
        q.set_current(ids[0]);
        q.remove(ids[0]);
        let a = q.next(Off, false).unwrap().id;
        assert!(q.prev(Off).is_none());
        assert_eq!(cur(&q), a);
        let b = q.next(Off, false).unwrap().id;
        assert_ne!(a, b);
    }

    #[test]
    fn repeat_one_user_prev_wraps() {
        let (mut q, ids) = queue_of(3);
        q.set_current(ids[0]);
        assert_eq!(q.prev(One).unwrap().id, ids[2]);
    }

    #[test]
    fn clear_resets() {
        let (mut q, ids) = queue_of(3);
        q.set_current(ids[1]);
        q.clear();
        assert!(q.len() == 0 && q.current().is_none());
        assert!(q.next(All, false).is_none());
    }

    #[test]
    fn set_current_unknown_id() {
        let (mut q, _) = queue_of(2);
        assert!(!q.set_current(42));
        assert!(q.current().is_none());
    }

    #[test]
    fn repeat_cycles_off_all_one_off() {
        assert_eq!(Off.next(), All);
        assert_eq!(All.next(), One);
        assert_eq!(One.next(), Off);
    }
}
