//! Playlists view: saved playlists (left) and the highlighted playlist's contents (right).
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use crate::app::App;
use crate::library::Track;
use crate::ui::layout::Breakpoint;
use crate::ui::widgets::{Col, Mark, draw_rows, empty_state, fit, pane_title, track_row};

const NO_PLAYLISTS: &str = "No playlists";
const NO_PLAYLISTS_HINT: &str = "press N to create one, A on any track to add to a playlist";

/// "1h 12m" / "12m" / "45s".
fn fmt_total(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    match (s / 3600, s / 60 % 60) {
        (0, 0) => format!("{s}s"),
        (0, m) => format!("{m}m"),
        (h, m) => format!("{h}h {m:02}m"),
    }
}

fn plural(n: usize) -> String {
    format!("{n} track{}", if n == 1 { "" } else { "s" })
}

pub fn draw(f: &mut Frame, app: &App, area: Rect, bp: Breakpoint) {
    if bp.library_panes() <= 1 {
        if app.pl_ui.focus_items {
            draw_items(f, app, area);
        } else {
            draw_list(f, app, area);
        }
        return;
    }
    let [l, r] = Layout::horizontal([Constraint::Percentage(38), Constraint::Min(10)]).areas(area);
    draw_list(f, app, l);
    draw_items(f, app, r);
}

fn draw_list(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let focused = !app.pl_ui.focus_items;
    let [head, body] = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);
    f.render_widget(Paragraph::new(pane_title(t, "PLAYLISTS", focused)), head);
    if app.playlists.is_empty() {
        empty_state(f, t, body, NO_PLAYLISTS, NO_PLAYLISTS_HINT);
        return;
    }
    let w = body.width as usize;
    draw_rows(
        f,
        t,
        body,
        app.pl_ui.list.selected(),
        app.playlists.len(),
        |i, cur| {
            let p = &app.playlists[i];
            let n = p.track_count.to_string();
            let d = fmt_total(p.duration);
            // Name flexes; count and total duration are right-aligned fixed cells.
            track_row(
                t,
                w,
                cur,
                Mark::None,
                false,
                &[
                    Col::new(&p.name, 0),
                    Col::new(&n, 4).right().style(t.secondary()),
                    Col::new(&d, 7).right().style(t.secondary()),
                ],
                None,
            )
        },
    );
}

fn draw_items(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let focused = app.pl_ui.focus_items;
    let [head, detail, body] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .areas(area);
    let Some(pl) = app.selected_playlist() else {
        f.render_widget(Paragraph::new(pane_title(t, "CONTENTS", focused)), head);
        empty_state(
            f,
            t,
            body,
            "Nothing selected",
            "select a playlist on the left",
        );
        return;
    };
    f.render_widget(Paragraph::new(pane_title(t, "CONTENTS", focused)), head);
    let info = format!(
        " {} · {} · {}",
        pl.name,
        plural(pl.track_count),
        fmt_total(pl.duration)
    );
    f.render_widget(
        Paragraph::new(Line::styled(
            fit(&info, area.width as usize, false),
            t.secondary(),
        )),
        detail,
    );
    let items = &app.pl_ui.items;
    if items.is_empty() {
        empty_state(
            f,
            t,
            body,
            "Empty playlist",
            "press A on any track to add it here",
        );
        return;
    }
    let playing = app.player.snapshot().current.as_ref().map(|c| &c.path);
    let queue = app.player.queue().entries();
    let w = body.width as usize;
    // Selection highlight only on the focused pane's cursor; the unfocused pane keeps a dim cursor row.
    let sel = app.pl_ui.items_list.selected();
    draw_rows(f, t, body, sel, items.len(), |i, cur| {
        let tr: &Track = &items[i].track;
        let mark = if playing == Some(&tr.path) {
            Mark::Playing
        } else if let Some(n) = queue.iter().position(|e| e.track.path == tr.path) {
            Mark::Queued(Some(n + 1))
        } else {
            Mark::None
        };
        let num = (i + 1).to_string();
        let artist = tr.artist.as_deref().unwrap_or("");
        track_row(
            t,
            w,
            cur && focused,
            mark,
            false,
            &[
                Col::new(&num, 3).right().style(t.muted()),
                Col::new(&tr.title, 0),
                Col::new(artist, 0).style(t.secondary()),
            ],
            Some(tr.duration),
        )
    });
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

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
            artist: Some("アーティスト".into()),
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

    fn render(a: &App, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| {
            let area = f.area();
            draw(f, a, area, Breakpoint::of(area));
        })
        .unwrap();
        let buf = term.backend().buffer().clone();
        buf.content().iter().map(|c| c.symbol()).collect()
    }

    #[test]
    fn totals() {
        assert_eq!(fmt_total(4320.0), "1h 12m");
        assert_eq!(fmt_total(90.0), "1m");
        assert_eq!(fmt_total(5.0), "5s");
        assert_eq!(plural(1), "1 track");
    }

    #[test]
    fn renders_at_all_sizes() {
        let mut a = app();
        for (w, h) in [(120, 35), (80, 24), (60, 20), (30, 10)] {
            assert!(render(&a, w, h).contains("No playlists"), "{w}x{h}");
        }
        let pid =
            a.db.create_playlist("日本語のプレイリスト😀 very long name")
                .unwrap();
        a.db.add_to_playlist(
            pid,
            &[track("曲名😀 a very long track title indeed"), track("b")],
        )
        .unwrap();
        a.reload_playlists();
        for (w, h) in [(120, 35), (80, 24), (60, 20), (30, 10)] {
            a.pl_ui.focus_items = false;
            assert!(render(&a, w, h).contains("PLAYLISTS"), "{w}x{h}");
            a.pl_ui.focus_items = true;
            assert!(render(&a, w, h).contains("CONTENTS"), "{w}x{h}");
        }
        a.pl_ui.focus_items = true;
        assert!(render(&a, 120, 35).contains("2 tracks"));
    }
}
