//! Queue view: header summary + rows (position, title, artist, duration).
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};

use crate::app::App;
use crate::ui::layout::Breakpoint;
use crate::ui::widgets::{Col, Mark, draw_rows, empty_state, pane_title, track_row, truncate};

/// "1h 02m" / "12m" / "45s".
pub fn fmt_total(secs: f64) -> String {
    let s = if secs.is_finite() {
        secs.max(0.0) as u64
    } else {
        0
    };
    match (s / 3600, s / 60 % 60) {
        (0, 0) => format!("{s}s"),
        (0, m) => format!("{m}m"),
        (h, m) => format!("{h}h {m:02}m"),
    }
}

/// Header text; `next` is the entry after the playing one.
pub fn header_text(n: usize, total: f64, next: Option<&str>) -> String {
    let mut s = format!(
        "QUEUE · {n} track{} · total {}",
        if n == 1 { "" } else { "s" },
        fmt_total(total)
    );
    if let Some(t) = next {
        s.push_str(" · next: ");
        s.push_str(t);
    }
    s
}

pub fn draw(f: &mut Frame, app: &App, area: Rect, bp: Breakpoint) {
    let t = &app.theme;
    let q = app.player.queue();
    let entries = q.entries();
    if entries.is_empty() {
        return empty_state(
            f,
            t,
            area,
            "Queue is empty",
            "a adds from Library/Files · Enter plays",
        );
    }
    let [head, rows] = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);
    let cur = q.current_index();
    let total: f64 = entries.iter().filter_map(|e| e.track.duration).sum();
    let next = cur
        .and_then(|i| entries.get(i + 1))
        .map(|e| e.track.title.as_str());
    let text = header_text(entries.len(), total, next);
    f.render_widget(
        pane_title(
            t,
            &truncate(&text, head.width.saturating_sub(2) as usize),
            true,
        ),
        head,
    );
    let pos_w = entries.len().to_string().len().max(2);
    let width = rows.width as usize;
    draw_rows(
        f,
        t,
        rows,
        app.queue_ui.list.selected(),
        entries.len(),
        |i, cursor| {
            let e = &entries[i];
            let pos = (i + 1).to_string();
            let ok = app.queue_ui.file_exists(e.id, &e.track.path);
            let artist = e.track.artist.as_deref().unwrap_or("");
            let mut cols = vec![
                Col::new(&pos, pos_w).right().style(t.muted()),
                Col::new(&e.track.title, 0),
            ];
            if bp != Breakpoint::Narrow {
                cols.push(Col::new(artist, 0).style(t.secondary()));
            }
            // Always present so the title/artist columns align across rows.
            cols.push(Col::new(if ok { "" } else { "missing" }, 7));
            let mark = if cur == Some(i) {
                Mark::Playing
            } else {
                Mark::None
            };
            track_row(t, width, cursor, mark, !ok, &cols, Some(e.track.duration))
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn totals() {
        assert_eq!(fmt_total(3720.0), "1h 02m");
        assert_eq!(fmt_total(720.0), "12m");
        assert_eq!(fmt_total(f64::NAN), "0s");
    }

    #[test]
    fn header() {
        assert_eq!(header_text(1, 60.0, None), "QUEUE · 1 track · total 1m");
        assert!(header_text(3, 0.0, Some("x")).ends_with("next: x"));
    }

    #[test]
    fn renders_at_sizes() {
        use crate::config::keys::KeyMap;
        use crate::library::database::Database;
        use crate::playback::Player;
        use ratatui::{Terminal, backend::TestBackend};
        let (tx, _rx) = std::sync::mpsc::channel();
        let keys = KeyMap::from_config(&Default::default(), &mut Vec::new());
        let theme = crate::ui::theme::Theme::default();
        let db = Database::open_in_memory().unwrap();
        let mut a = App::new(tx.clone(), Player::new(tx, 100), db, keys, theme);
        for n in [
            "日本語のとても長いタイトル🎵🎵🎵",
            "long long long long long long long long long long long",
            "Plain",
        ] {
            a.player.queue_mut().push(vec![crate::library::Track {
                id: 1,
                path: "/nope/x.mp3".into(),
                title: n.into(),
                artist: Some("Artist".into()),
                album: None,
                album_artist: None,
                genre: None,
                track_no: None,
                year: None,
                duration: Some(245.0),
                format: None,
                size: 0,
                width: None,
                height: None,
                is_video: false,
            }]);
        }
        a.queue_ui.list.select(Some(1));
        for (w, h) in [(120, 35), (80, 24), (60, 20), (30, 10), (5, 1)] {
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            term.draw(|f| draw(f, &a, f.area(), Breakpoint::of(f.area())))
                .unwrap();
            if w >= 60 {
                let text: String = term
                    .backend()
                    .buffer()
                    .content()
                    .iter()
                    .map(|c| c.symbol())
                    .collect();
                assert!(
                    text.contains("QUEUE · 3 tracks")
                        && text.contains("missing")
                        && text.contains("Plain"),
                    "{w}x{h}"
                );
            }
        }
    }
}
