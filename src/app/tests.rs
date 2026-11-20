use std::path::PathBuf;
use std::sync::mpsc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::*;
use crate::playback::modes::RepeatMode;

fn track(n: usize) -> Track {
    Track {
        id: 0,
        path: PathBuf::from(format!("/nonexistent/rmp-test/{n}.mp3")),
        title: format!("t{n}"),
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

fn app() -> App {
    let (tx, _rx) = mpsc::channel();
    let db = Database::open_in_memory().unwrap();
    let mut w = Vec::new();
    let keys = KeyMap::from_config(&Default::default(), &mut w);
    App::new(tx.clone(), Player::new(tx, 100), db, keys, Theme::default())
}

fn press(a: &mut App, c: char) {
    a.handle(AppEvent::Key(KeyEvent::new(
        KeyCode::Char(c),
        KeyModifiers::NONE,
    )));
}

#[test]
fn repeat_and_shuffle_keys() {
    let mut a = app();
    let seen: Vec<_> = (0..3)
        .map(|_| {
            press(&mut a, 'r');
            a.player.snapshot().repeat
        })
        .collect();
    assert_eq!(seen, [RepeatMode::All, RepeatMode::One, RepeatMode::Off]);
    press(&mut a, 's');
    assert!(a.player.snapshot().shuffle);
    press(&mut a, 's');
    assert!(!a.player.snapshot().shuffle);
}

#[test]
fn play_tracks_sets_queue_and_mpris_command_path() {
    let mut a = app();
    a.handle(AppEvent::Command(Command::PlayTracks {
        tracks: (0..3).map(track).collect(),
        start: 1,
    }));
    assert_eq!(a.player.queue().len(), 3);
    a.handle(AppEvent::Command(Command::Enqueue(vec![track(9)])));
    assert_eq!(a.player.queue().len(), 4);
    assert!(a.status.is_some());
}

#[test]
fn typing_does_not_trigger_shortcuts() {
    let mut a = app();
    press(&mut a, '/');
    press(&mut a, 'q');
    press(&mut a, 'r');
    assert!(!a.quit);
    assert_eq!(a.player.snapshot().repeat, RepeatMode::Off);
    assert_eq!(a.search.as_deref(), Some("qr"));
    a.handle(AppEvent::Key(KeyEvent::new(
        KeyCode::Esc,
        KeyModifiers::NONE,
    )));
    assert!(a.search.is_none());
    press(&mut a, 'q');
    assert!(a.quit);
}

#[test]
fn playlist_dialog_creates_playlist() {
    let mut a = app();
    press(&mut a, '3');
    press(&mut a, 'N');
    press(&mut a, 'x');
    a.handle(AppEvent::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    assert_eq!(a.playlists.len(), 1);
    assert_eq!(a.playlists[0].name, "x");
}

#[test]
fn help_blocks_shortcuts_and_closes() {
    let mut a = app();
    press(&mut a, '?');
    assert!(matches!(a.dialog, Some(Dialog::Help { .. })));
    press(&mut a, 'r');
    assert_eq!(a.player.snapshot().repeat, RepeatMode::Off);
    press(&mut a, 'q');
    assert!(a.dialog.is_none() && !a.quit);
}

#[test]
fn picker_adds_to_playlist() {
    let mut a = app();
    a.open_picker(vec![track(1)]);
    assert!(a.status.as_ref().is_some_and(|s| s.1), "no playlist yet");
    a.db.create_playlist("Mix").unwrap();
    a.reload_playlists();
    a.open_picker(vec![track(1)]);
    assert!(matches!(a.dialog, Some(Dialog::Picker { .. })));
    a.handle(AppEvent::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    assert!(a.dialog.is_none());
    assert_eq!(
        a.status.as_ref().map(|s| s.0.as_str()),
        Some("Added 1 to Mix")
    );
}

fn key(a: &mut App, k: KeyCode) {
    a.handle(AppEvent::Key(KeyEvent::new(k, KeyModifiers::NONE)));
}

#[test]
fn clear_queue_confirm_flow() {
    let mut a = app();
    a.handle(AppEvent::Command(Command::Enqueue(vec![
        track(1),
        track(2),
    ])));
    press(&mut a, '4');
    for cancel in [KeyCode::Esc, KeyCode::Char('n')] {
        press(&mut a, 'C');
        assert!(matches!(
            a.dialog,
            Some(Dialog::Confirm(Confirm::ClearQueue))
        ));
        key(&mut a, cancel);
        assert!(a.dialog.is_none());
        assert_eq!(a.player.queue().len(), 2);
    }
    press(&mut a, 'C');
    key(&mut a, KeyCode::Char('y'));
    assert!(a.dialog.is_none());
    assert_eq!(a.player.queue().len(), 0);
}
