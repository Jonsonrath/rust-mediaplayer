//! History view: newest first, grouped under day separators.
use std::time::{SystemTime, UNIX_EPOCH};

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use crate::app::App;
use crate::ui::layout::Breakpoint;
use crate::ui::widgets::{Col, Mark, empty_state, fmt_time, pane_title, track_row, window};

const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Local day number (days since 1970-01-01) of a unix time.
pub fn day_of(ts: i64, offset: i64) -> i64 {
    (ts + offset).div_euclid(86400)
}

/// "14:32" in local time.
pub fn clock(ts: i64, offset: i64) -> String {
    let s = (ts + offset).rem_euclid(86400);
    format!("{:02}:{:02}", s / 3600, s / 60 % 60)
}

/// "Today", "Yesterday" or "Mon 16 Nov".
pub fn day_label(day: i64, today: i64) -> String {
    match today - day {
        0 => return "Today".into(),
        1 => return "Yesterday".into(),
        _ => {}
    }
    // Howard Hinnant's civil_from_days.
    let z = day + 719_468;
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let wd = (day + 4).rem_euclid(7);
    format!("{} {d} {}", DAYS[wd as usize], MONTHS[(m - 1) as usize])
}

pub fn draw(f: &mut Frame, app: &App, area: Rect, bp: Breakpoint) {
    let t = &app.theme;
    let hist = &app.history;
    if hist.is_empty() {
        return empty_state(
            f,
            t,
            area,
            "No history yet",
            "Tracks you play show up here · D clears",
        );
    }
    let off = app.hist_ui.tz_offset;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let today = day_of(now, off);
    let playing = app.player.snapshot().current.as_ref().map(|c| &c.path);
    let sel = app.hist_ui.list.selected();
    let h = area.height as usize;
    let width = area.width as usize;
    let (a, b) = window(sel, hist.len(), h);
    let mut lines: Vec<Line> = Vec::with_capacity(h + 2);
    let mut cur_line = 0;
    let mut prev_day = None;
    for (i, e) in hist.iter().enumerate().take(b).skip(a) {
        let day = day_of(e.played_at, off);
        if prev_day != Some(day) {
            lines.push(pane_title(t, &day_label(day, today), false));
            prev_day = Some(day);
        }
        let cursor = sel == Some(i);
        let time = clock(e.played_at, off);
        let artist = e.track.artist.as_deref().unwrap_or("");
        let played = match (e.completed, e.track.duration) {
            (true, _) | (false, None) => fmt_time(e.played_secs),
            (false, Some(d)) => format!("{} of {}", fmt_time(e.played_secs), fmt_time(d)),
        };
        let (tick, tick_style) = if e.completed {
            ("✓", t.playing())
        } else {
            ("·", t.muted())
        };
        let mut cols = vec![
            Col::new(&time, 5).style(t.secondary()),
            Col::new(&e.track.title, 0),
        ];
        if bp != Breakpoint::Narrow {
            cols.push(Col::new(artist, 0).style(t.secondary()));
        }
        cols.push(Col::new(&played, 15).right().style(if e.completed {
            t.secondary()
        } else {
            t.muted()
        }));
        cols.push(Col::new(tick, 1).style(tick_style));
        let mark = if playing == Some(&e.track.path) {
            Mark::Playing
        } else {
            Mark::None
        };
        let mut line = track_row(t, width, cursor, mark, false, &cols, None);
        if cursor {
            cur_line = lines.len();
            line = line.style(t.selected());
        }
        lines.push(line);
    }
    // Separators take lines: trim the end, or the top if the cursor would fall off.
    let drop_top = (cur_line + 1)
        .saturating_sub(h)
        .min(lines.len().saturating_sub(h));
    lines.drain(..drop_top);
    lines.truncate(h);
    f.render_widget(Paragraph::new(lines), area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days() {
        // 2025-11-16 is a Sunday; day number 20408.
        let d = 20408;
        assert_eq!(day_label(d, d), "Today");
        assert_eq!(day_label(d, d + 1), "Yesterday");
        assert_eq!(day_label(d, d + 5), "Sun 16 Nov");
        assert_eq!(day_label(0, 99), "Thu 1 Jan");
    }

    #[test]
    fn local_grouping() {
        // 23:30 UTC is the next day at +1h.
        let ts = 86400 * 10 + 23 * 3600 + 30 * 60;
        assert_eq!(day_of(ts, 0), 10);
        assert_eq!(day_of(ts, 3600), 11);
        assert_eq!(clock(ts, 3600), "00:30");
        assert_eq!(clock(-60, 0), "23:59");
    }

    fn app() -> App {
        use crate::config::keys::KeyMap;
        use crate::library::database::Database;
        use crate::playback::Player;
        let (tx, _rx) = std::sync::mpsc::channel();
        let keys = KeyMap::from_config(&Default::default(), &mut Vec::new());
        let db = Database::open_in_memory().unwrap();
        let theme = crate::ui::theme::Theme::default();
        App::new(tx.clone(), Player::new(tx, 100), db, keys, theme)
    }

    fn track(title: &str) -> crate::library::Track {
        crate::library::Track {
            id: 1,
            path: "/nope/x.mp3".into(),
            title: title.into(),
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
        }
    }

    #[test]
    fn renders_at_sizes() {
        use crate::library::database::HistoryEntry;
        use ratatui::{Terminal, backend::TestBackend};
        let mut a = app();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        for (i, n) in [
            "日本語のとても長いタイトル🎵🎵🎵",
            &"long ".repeat(40),
            "Plain",
        ]
        .iter()
        .enumerate()
        {
            a.history.push(HistoryEntry {
                id: i as i64,
                played_at: now - 86400 * i as i64 * 3,
                played_secs: 100.0,
                completed: i % 2 == 0,
                track: track(n),
            });
        }
        a.hist_ui.list.select(Some(2));
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
                assert!(text.contains("Today") && text.contains("Plain"), "{w}x{h}");
            }
        }
    }
}
