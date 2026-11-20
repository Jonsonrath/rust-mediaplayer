//! Header (identity + tabs) and footer (status / search bar / shortcut hints).
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::layout::Breakpoint;
use super::widgets::{tail, truncate};
use crate::app::{App, VIEW_NAMES};
use crate::config::keys::Action;

const SHORT_NAMES: [&str; 5] = ["LIB", "FILES", "PLAY", "QUEUE", "HIST"];

pub fn header(f: &mut Frame, app: &App, area: Rect, bp: Breakpoint) {
    let t = &app.theme;
    let w = area.width as usize;
    let queued = app.player.queue().len();
    let right = Line::styled(format!("{queued} queued"), t.muted());
    let build = |short: bool, identity: bool| {
        let mut v = Vec::new();
        if identity {
            v.push(Span::styled(" RMP", t.strong()));
            v.push(Span::styled(
                format!(" / {:02}   ", app.view + 1),
                t.muted(),
            ));
        } else {
            v.push(Span::raw(" "));
        }
        for i in 0..VIEW_NAMES.len() {
            let name = if short { SHORT_NAMES[i] } else { VIEW_NAMES[i] };
            let label = if short {
                format!("{} {name}", i + 1)
            } else {
                format!("{:02} {name}", i + 1)
            };
            v.push(Span::styled(label, t.tab(i == app.view)));
            v.push(Span::raw("  "));
        }
        Line::from(v)
    };
    let need_right = right.width() + 2;
    // Full names whenever they fit; the queue count is only drawn if it also fits.
    let mut line = build(false, true);
    if line.width() > w {
        line = build(true, true);
    }
    if line.width() > w {
        line = build(true, false);
    }
    f.render_widget(Paragraph::new(line.clone()), area);
    if bp != Breakpoint::Narrow && line.width() + need_right <= w {
        let rw = right.width() as u16;
        let r = Rect {
            x: area.right().saturating_sub(rw + 1),
            width: rw,
            ..area
        };
        f.render_widget(Paragraph::new(right), r);
    }
}

/// First binding of an action (`key_label` joins alternatives with `/`).
fn key(app: &App, a: Action) -> String {
    let l = app.keys.key_label(a);
    match l.split('/').next() {
        Some(f) if !f.is_empty() => f.to_string(),
        _ => l,
    }
}

fn view_hints(app: &App) -> Vec<(String, &'static str)> {
    let k = |a| key(app, a);
    match app.view {
        0 => vec![
            (k(Action::Enter), "play"),
            (k(Action::Enqueue), "queue"),
            (k(Action::AddToPlaylist), "playlist"),
            (k(Action::Search), "search"),
        ],
        1 => vec![
            (k(Action::Enter), "open/play"),
            (k(Action::Left), "up"),
            (k(Action::Enqueue), "queue"),
        ],
        2 => vec![
            (k(Action::Enter), "play"),
            (k(Action::FocusNext), "pane"),
            (k(Action::NewPlaylist), "new"),
            (k(Action::Rename), "rename"),
            (k(Action::Delete), "delete"),
        ],
        3 => vec![
            (k(Action::Enter), "play"),
            (k(Action::Remove), "remove"),
            (
                format!("{}/{}", k(Action::MoveDown), k(Action::MoveUp)),
                "move",
            ),
            (k(Action::ClearQueue), "clear"),
        ],
        _ => vec![(k(Action::Enter), "replay"), (k(Action::Delete), "clear")],
    }
}

/// `key label` hints that fit in `room` cells, followed by the always-kept help hint.
fn hint_spans(app: &App, room: usize) -> Vec<Span<'static>> {
    let t = &app.theme;
    let help = (key(app, Action::Help), "help");
    let hw = |k: &str, l: &str| k.chars().count() + l.len() + 1;
    let mut used = hw(&help.0, help.1);
    if used > room {
        return Vec::new();
    }
    let mut spans = Vec::new();
    for (k, l) in view_hints(app) {
        let n = hw(&k, l) + 2;
        if used + n > room {
            break;
        }
        used += n;
        spans.push(Span::styled(k, t.key()));
        spans.push(Span::styled(format!(" {l}  "), t.muted()));
    }
    spans.push(Span::styled(help.0, t.key()));
    spans.push(Span::styled(format!(" {}", help.1), t.muted()));
    spans
}

pub fn footer(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let w = area.width as usize;
    let line = if let Some(q) = &app.search {
        let count = format!("{} matches", app.library.filtered.len());
        let room = w.saturating_sub(count.chars().count() + 6);
        let mut spans = vec![
            Span::styled(" / ", t.key()),
            Span::styled(tail(q, room), t.text_style()),
            Span::styled("▏", t.accent()),
        ];
        let used: usize = spans.iter().map(Span::width).sum();
        if w > used + count.chars().count() + 1 {
            spans.push(Span::raw(" ".repeat(w - used - count.chars().count() - 1)));
            spans.push(Span::styled(count, t.muted()));
        }
        Line::from(spans)
    } else if let Some((msg, err)) = &app.status {
        let (mark, ms, ts) = if *err {
            ("! ", t.error(), t.error())
        } else {
            ("● ", t.accent(), t.secondary())
        };
        let more = if *err {
            format!("  {} details", key(app, Action::Help))
        } else {
            String::new()
        };
        let room = w.saturating_sub(3 + more.chars().count());
        let mut spans = vec![Span::raw(" "), Span::styled(mark, ms)];
        spans.push(Span::styled(truncate(msg, room).into_owned(), ts));
        spans.push(Span::styled(more, t.muted()));
        // Keep the shortcut hints, right-aligned, for whatever room the toast leaves.
        let left: usize = spans.iter().map(Span::width).sum();
        let hints = hint_spans(app, w.saturating_sub(left + 2));
        let hw: usize = hints.iter().map(Span::width).sum();
        if hw > 0 {
            spans.push(Span::raw(" ".repeat(w - left - hw - 1)));
            spans.extend(hints);
            spans.push(Span::raw(" "));
        }
        Line::from(spans)
    } else {
        let mut spans = vec![Span::raw(" ")];
        spans.extend(hint_spans(app, w.saturating_sub(1)));
        Line::from(spans)
    };
    f.render_widget(Paragraph::new(line), area);
}
