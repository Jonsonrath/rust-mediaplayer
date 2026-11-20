//! Files view: breadcrumb, entry list and (Standard+) a detail pane.
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::library::{MediaKind, is_media_path};
use crate::ui::layout::Breakpoint;
use crate::ui::widgets::{Col, Mark, draw_rows, empty_state, pane_title, track_row, truncate};

const DETAIL_W: u16 = 36;

pub fn draw(f: &mut Frame, app: &App, area: Rect, bp: Breakpoint) {
    if area.is_empty() {
        return;
    }
    let t = &app.theme;
    let [head, rest] = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);
    let home = std::env::var_os("HOME");
    let title = pane_title(t, "Files", true);
    let tw = title.width() + 2;
    let (pre, last) = breadcrumb(
        &app.browser.dir,
        home.as_deref().map(Path::new),
        (head.width as usize).saturating_sub(tw),
    );
    let mut spans = title.spans;
    spans.push(Span::raw("  "));
    spans.push(Span::styled(pre, t.secondary()));
    spans.push(Span::styled(last, t.accent().bold()));
    f.render_widget(Paragraph::new(Line::from(spans)), head);

    let (list_a, detail_a) = if bp >= Breakpoint::Standard && rest.width > DETAIL_W + 20 {
        let [l, d] =
            Layout::horizontal([Constraint::Min(0), Constraint::Length(DETAIL_W)]).areas(rest);
        (l, Some(d))
    } else {
        (rest, None)
    };
    let b = &app.browser;
    if let Some(e) = &b.err {
        empty_state(
            f,
            t,
            list_a,
            "Cannot read this folder",
            &format!("{e} · h: parent"),
        );
    } else if b.entries.is_empty() {
        empty_state(f, t, list_a, "Empty folder", "h: parent folder");
    } else {
        draw_list(f, app, list_a, bp);
    }
    if let Some(d) = detail_a {
        draw_detail(f, app, d);
    }
}

fn draw_list(f: &mut Frame, app: &App, area: Rect, bp: Breakpoint) {
    let t = &app.theme;
    let b = &app.browser;
    let playing = app.player.snapshot().current.as_ref().map(|c| &c.path);
    let queue = app.player.queue().entries();
    let w = area.width as usize;
    let wide = bp.optional_columns();
    draw_rows(f, t, area, b.list.selected(), b.entries.len(), |i, cur| {
        let Some(e) = b.entries.get(i) else {
            return Line::default();
        };
        let is_playing = playing == Some(&e.path);
        let mark = if is_playing {
            Mark::Playing
        } else if e.is_dir {
            Mark::None
        } else {
            queue
                .iter()
                .position(|q| q.track.path == e.path)
                .map_or(Mark::None, |n| Mark::Queued(Some(n + 1)))
        };
        let name = display_name(&e.name, &e.path, e.is_dir);
        let size = if e.is_dir || !e.supported {
            String::new()
        } else {
            human_size(e.size)
        };
        let ext = if wide && !e.is_dir {
            ext_label(&e.path)
        } else {
            String::new()
        };
        let name_style = if is_playing {
            t.playing()
        } else if e.is_dir {
            Style::new().fg(t.accent_alt)
        } else {
            Style::new().fg(t.text)
        };
        let mut cols = vec![Col::new(&name, 0).style(name_style)];
        if w >= 34 {
            cols.push(Col::new(&size, 9).right());
        }
        if wide && w >= 50 {
            cols.push(Col::new(&ext, 5));
        }
        track_row(t, w, cur, mark, !e.is_dir && !e.supported, &cols, None)
    });
}

fn display_name(name: &str, path: &Path, is_dir: bool) -> String {
    if is_dir {
        format!("▸ {name}/")
    } else {
        let k = match is_media_path(path) {
            Some(MediaKind::Video) => "◼",
            Some(MediaKind::Audio) => "♪",
            None => "·",
        };
        format!("{k} {name}")
    }
}

fn ext_label(p: &Path) -> String {
    p.extension()
        .map(|x| x.to_string_lossy().to_uppercase())
        .unwrap_or_default()
}

fn draw_detail(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let block = Block::default()
        .borders(Borders::LEFT)
        .border_style(t.border());
    let inner = block.inner(area);
    f.render_widget(block, area);
    let w = inner.width.saturating_sub(1) as usize;
    let b = &app.browser;
    let Some(e) = b.list.selected().and_then(|i| b.entries.get(i)) else {
        return;
    };
    let kind = if e.is_dir {
        "Folder".to_string()
    } else {
        match is_media_path(&e.path) {
            Some(MediaKind::Audio) => format!("Audio · {}", ext_label(&e.path)),
            Some(MediaKind::Video) => format!("Video · {}", ext_label(&e.path)),
            None => "Unsupported".to_string(),
        }
    };
    let kv = |k: &str, v: String| {
        Line::from(vec![
            Span::styled(format!("{k:<9}"), t.muted()),
            Span::styled(
                truncate(&v, w.saturating_sub(9)).into_owned(),
                t.text_style(),
            ),
        ])
    };
    let mut lines = vec![
        Line::styled(truncate(&e.name, w).into_owned(), t.strong()),
        Line::default(),
        kv("Type", kind),
    ];
    if e.is_dir {
        let n = b.dir_count(&e.path);
        lines.push(kv("Items", n.map_or("--".into(), |n| n.to_string())));
    } else {
        lines.push(kv("Size", human_size(e.size)));
    }
    lines.push(kv("Modified", e.modified.map_or("--".into(), fmt_datetime)));
    if !e.is_dir && !e.supported {
        lines.push(Line::default());
        lines.push(Line::styled("Unsupported file type", t.warning()));
    }
    f.render_widget(
        Paragraph::new(lines),
        Rect::new(
            inner.x + 1,
            inner.y,
            inner.width.saturating_sub(1),
            inner.height,
        ),
    );
}

fn human_size(n: u64) -> String {
    const U: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if n < 1024 {
        return format!("{n} B");
    }
    let mut v = n as f64 / 1024.0;
    let mut u = 0;
    while v >= 1024.0 && u < U.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    format!("{v:.1} {}", U[u])
}

/// `YYYY-MM-DD HH:MM UTC` (no timezone database available, so UTC is stated).
fn fmt_datetime(t: SystemTime) -> String {
    let Ok(d) = t.duration_since(UNIX_EPOCH) else {
        return "--".into();
    };
    let secs = d.as_secs();
    let days = (secs / 86_400) as i64;
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let rem = secs % 86_400;
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        rem / 3600,
        rem % 3600 / 60
    )
}

/// Split a path into (prefix, last segment) fitting `w` cells: home becomes `~`, middle
/// segments collapse to `…`, the last segment is shown in full while it fits.
pub fn breadcrumb(path: &Path, home: Option<&Path>, w: usize) -> (String, String) {
    let (first, rest) = match home.and_then(|h| path.strip_prefix(h).ok()) {
        Some(r) => ("~".to_string(), r),
        None => (String::new(), path),
    };
    let mut segs: Vec<String> = rest
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    let Some(last) = segs.pop() else {
        return (
            String::new(),
            if first.is_empty() { "/".into() } else { first },
        );
    };
    let mut items = vec![first];
    items.extend(segs);
    let render = |items: &[String]| items.iter().map(|s| format!("{s}/")).collect::<String>();
    let mut elided = false;
    while render(&items).width() + last.width() > w && items.len() > 1 + usize::from(elided) {
        if elided {
            items.remove(2);
        } else {
            items[1] = "…".into();
            elided = true;
            // Drop the extra middle items collapsed into the ellipsis.
            continue;
        }
    }
    let head = render(&items);
    if head.width() + last.width() > w {
        // Still too long: keep only the tail of the last segment.
        let tail = crate::ui::widgets::tail(&last, w);
        return (String::new(), tail);
    }
    (head, last)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::mpsc;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use crate::config::keys::KeyMap;
    use crate::library::database::Database;
    use crate::playback::Player;
    use crate::ui::theme::Theme;

    fn app() -> App {
        let (tx, _rx) = mpsc::channel();
        let db = Database::open_in_memory().unwrap();
        let mut w = Vec::new();
        let keys = KeyMap::from_config(&Default::default(), &mut w);
        App::new(tx.clone(), Player::new(tx, 100), db, keys, Theme::default())
    }

    #[test]
    fn breadcrumb_home_and_elision() {
        let h = Path::new("/home/u");
        let p = Path::new("/home/u/Music/Artists/Album/CD1");
        assert_eq!(
            breadcrumb(p, Some(h), 80),
            ("~/Music/Artists/Album/".into(), "CD1".into())
        );
        let (pre, last) = breadcrumb(p, Some(h), 14);
        assert_eq!((pre.as_str(), last.as_str()), ("~/…/Album/", "CD1"));
        let (pre, last) = breadcrumb(p, Some(h), 6);
        assert!(pre.width() + last.width() <= 6);
        assert_eq!(
            breadcrumb(Path::new("/"), None, 10),
            (String::new(), "/".into())
        );
        assert_eq!(breadcrumb(h, Some(h), 10), (String::new(), "~".into()));
    }

    #[test]
    fn datetime_and_size() {
        let t = UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        assert_eq!(fmt_datetime(t), "2023-11-14 22:13 UTC");
        assert_eq!(human_size(1536), "1.5 KB");
        assert_eq!(human_size(12), "12 B");
    }

    #[test]
    fn renders_at_all_sizes() {
        let d = tempfile::tempdir().unwrap();
        let long = "very long name ".repeat(12);
        for n in [
            format!("{long}.mp3"),
            "日本語のとても長いファイル名.flac".into(),
            "🎵emoji🎶.mkv".into(),
            "notes.txt".into(),
        ] {
            std::fs::write(d.path().join(n), b"abc").unwrap();
        }
        std::fs::create_dir(d.path().join("Субдиректория")).unwrap();
        let mut a = app();
        a.load_dir(d.path().to_path_buf());
        a.browser.list.select(Some(2));
        for (w, h) in [(120u16, 35u16), (80, 24), (60, 20), (30, 10)] {
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            term.draw(|f| {
                let r = f.area();
                draw(f, &a, r, Breakpoint::of(r));
            })
            .unwrap();
            let text: String = term
                .backend()
                .buffer()
                .content()
                .iter()
                .map(|c| c.symbol())
                .collect();
            assert!(text.contains("Files"), "{w}x{h}");
            if w >= 60 {
                assert!(text.contains("▸"), "{w}x{h}");
            }
            if w >= 120 {
                assert!(text.contains("Unsupported") || text.contains("Type"));
            }
        }
        a.load_dir(PathBuf::from("/nonexistent/rmp"));
        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        term.draw(|f| draw(f, &a, f.area(), Breakpoint::Compact))
            .unwrap();
    }
}
