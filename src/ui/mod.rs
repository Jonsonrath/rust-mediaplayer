//! Rendering. Reads `App`, never mutates it, never touches DB/FS.
pub mod chrome;
pub mod dialogs;
pub mod layout;
pub mod player;
pub mod theme;
pub mod views;
pub mod widgets;

#[cfg(test)]
mod snapshot_tests;

use ratatui::Frame;
use ratatui::layout::Alignment;
use ratatui::widgets::{Block, Paragraph};

use crate::app::App;
use layout::Breakpoint;

pub fn draw(f: &mut Frame, app: &App) {
    let t = &app.theme;
    let area = f.area();
    f.render_widget(Block::new().style(t.base()), area);
    let bp = Breakpoint::of(area);
    if bp == Breakpoint::TooSmall {
        let r = ratatui::layout::Rect {
            y: area.y + area.height.saturating_sub(1) / 2,
            height: area.height.min(2),
            ..area
        };
        f.render_widget(
            Paragraph::new("Terminal too small — resize")
                .style(t.warning())
                .alignment(Alignment::Center)
                .wrap(ratatui::widgets::Wrap { trim: true }),
            r,
        );
        return;
    }
    let r = layout::split(area, bp);
    chrome::header(f, app, r.header, bp);
    f.render_widget(
        Paragraph::new(widgets::separator(t, r.separator.width)),
        r.separator,
    );
    views::draw(f, app, r.body, bp);
    player::draw(f, app, r.player, bp);
    chrome::footer(f, app, r.footer);
    dialogs::draw(f, app, area);
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::app::{Confirm, Dialog, TextPurpose};
    use crate::config::keys::KeyMap;
    use crate::library::database::Database;
    use crate::playback::{PlayState, Player, PlayerSnapshot};
    use crate::ui::theme::Theme;

    fn app() -> App {
        let (tx, _rx) = mpsc::channel();
        let db = Database::open_in_memory().unwrap();
        let mut w = Vec::new();
        let keys = KeyMap::from_config(&Default::default(), &mut w);
        App::new(tx.clone(), Player::new(tx, 100), db, keys, Theme::default())
    }

    const SIZES: [(u16, u16); 14] = [
        (0, 0),
        (1, 1),
        (20, 5),
        (39, 11),
        (40, 12),
        (59, 19),
        (60, 20),
        (80, 24),
        (100, 30),
        (120, 35),
        (300, 80),
        (500, 3),
        (3, 100),
        (1, 40),
    ];

    #[test]
    fn draw_never_panics() {
        let mut a = app();
        let dialogs = [
            None,
            Some(Dialog::Text(TextPurpose::NewPlaylist, "日本語 ".repeat(40))),
            Some(Dialog::Confirm(Confirm::ClearHistory)),
            Some(Dialog::Error {
                msg: "x".repeat(500),
                scroll: 3,
            }),
            Some(Dialog::Help { scroll: 5 }),
            Some(Dialog::Picker {
                tracks: vec![],
                sel: 0,
            }),
        ];
        for d in dialogs {
            a.dialog = d;
            for view in 0..5 {
                a.view = view;
                for (w, h) in SIZES {
                    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
                    term.draw(|f| draw(f, &a)).unwrap();
                }
            }
        }
        a.dialog = None;
        a.search = Some("query".into());
        a.notify_error("boom ".repeat(50));
        a.scanning = true;
        a.scan_count = 1234;
        for (w, h) in SIZES {
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            term.draw(|f| draw(f, &a)).unwrap();
        }
    }

    #[test]
    fn player_panel_sizes() {
        let t = Theme::default();
        let mut track = crate::library::Track {
            id: 1,
            path: "/x.mkv".into(),
            title: "Digital Bath 日本語 ".repeat(5),
            artist: Some("Deftones".into()),
            album: Some("White Pony".into()),
            album_artist: None,
            genre: None,
            track_no: None,
            year: None,
            duration: Some(255.0),
            format: None,
            size: 0,
            width: None,
            height: None,
            is_video: true,
        };
        for (state, dur) in [(PlayState::Playing, Some(255.0)), (PlayState::Paused, None)] {
            track.duration = dur;
            let snap = PlayerSnapshot {
                state,
                current: Some(track.clone()),
                position: 84.0,
                duration: dur,
                volume: 130,
                ..Default::default()
            };
            for (w, h) in SIZES {
                let area = ratatui::layout::Rect::new(0, 0, w, h);
                let bp = Breakpoint::of(area);
                let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
                term.draw(|f| player::draw_panel(f, &t, &snap, Some(99999), f.area(), bp))
                    .unwrap();
            }
        }
        let snap = PlayerSnapshot {
            state: PlayState::Playing,
            current: Some(track),
            position: 84.0,
            duration: Some(255.0),
            ..Default::default()
        };
        let mut term = Terminal::new(TestBackend::new(100, 5)).unwrap();
        term.draw(|f| player::draw_panel(f, &t, &snap, None, f.area(), Breakpoint::Standard))
            .unwrap();
        let text: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            text.contains("NOW PLAYING")
                && text.contains("PLAYING")
                && text.contains("01:24")
                && text.contains("04:15")
                && text.contains("VIDEO")
        );
    }
}
