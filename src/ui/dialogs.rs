//! Modal dialogs: one consistent style (raised surface, accent border, padded, hint line).
use ratatui::Frame;
use ratatui::layout::{Margin, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};

use super::theme::Theme;
use super::widgets::{Col, DUR_W, MARKER_W, draw_rows, fit, tail, truncate};
use crate::app::{App, Confirm, Dialog, TextPurpose};
use crate::config::keys::Action;

/// (group, [(action, description)]) for the help overlay; keys come from the live KeyMap.
const HELP: &[(&str, &[(Action, &str)])] = &[
    (
        "Playback",
        &[
            (Action::TogglePause, "play / pause"),
            (Action::Stop, "stop"),
            (Action::Next, "next track"),
            (Action::Prev, "previous track"),
            (Action::SeekBack, "seek back 5s"),
            (Action::SeekForward, "seek forward 5s"),
            (Action::VolumeDown, "volume down"),
            (Action::VolumeUp, "volume up"),
            (Action::CycleRepeat, "repeat off / all / one"),
            (Action::ToggleShuffle, "shuffle"),
        ],
    ),
    (
        "Navigation",
        &[
            (Action::ViewLibrary, "library"),
            (Action::ViewFiles, "files"),
            (Action::ViewPlaylists, "playlists"),
            (Action::ViewQueue, "queue"),
            (Action::ViewHistory, "history"),
            (Action::Down, "down"),
            (Action::Up, "up"),
            (Action::Left, "left pane / parent dir"),
            (Action::Right, "right pane / open"),
            (Action::Top, "top"),
            (Action::Bottom, "bottom"),
            (Action::PageDown, "page down"),
            (Action::PageUp, "page up"),
            (Action::FocusNext, "next pane"),
            (Action::Enter, "open / play"),
            (Action::Search, "search library"),
            (Action::Help, "this help"),
            (Action::Quit, "quit"),
        ],
    ),
    (
        "Queue & Playlists",
        &[
            (Action::Enqueue, "add to queue"),
            (Action::AddToPlaylist, "add to playlist"),
            (Action::Remove, "remove item"),
            (Action::MoveDown, "move item down"),
            (Action::MoveUp, "move item up"),
            (Action::ClearQueue, "clear queue"),
            (Action::NewPlaylist, "new playlist"),
            (Action::Rename, "rename playlist"),
            (Action::Delete, "delete playlist / clear history"),
            (Action::Rescan, "rescan library"),
        ],
    ),
];

/// Centered, clamped modal frame; returns the padded content area.
fn modal(f: &mut Frame, area: Rect, title: &str, border: Style, t: &Theme, w: u16, h: u16) -> Rect {
    let w = w.min(area.width.saturating_sub(2));
    let h = h.min(area.height.saturating_sub(2));
    let r = Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    };
    f.render_widget(Clear, r);
    let block = Block::bordered()
        .style(t.modal())
        .border_style(border.bg(t.raised))
        .title(Span::styled(format!(" {title} "), border.bg(t.raised)));
    let inner = block.inner(r).inner(Margin::new(1, 0));
    f.render_widget(block, r);
    inner
}

fn hint(t: &Theme, keys: &str) -> Line<'static> {
    Line::styled(keys.to_string(), t.muted())
}

pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    let Some(d) = &app.dialog else { return };
    let t = &app.theme;
    match d {
        Dialog::Text(p, buf) => {
            let title = match p {
                TextPurpose::NewPlaylist => "New playlist",
                TextPurpose::RenamePlaylist(_) => "Rename playlist",
            };
            let inner = modal(f, area, title, t.accent(), t, 48, 7);
            let room = (inner.width as usize).saturating_sub(1);
            let input = if buf.is_empty() {
                Line::from(vec![
                    Span::styled("▏", t.accent()),
                    Span::styled("name", t.muted()),
                ])
            } else {
                Line::from(vec![
                    Span::styled(tail(buf, room), t.text_style()),
                    Span::styled("▏", t.accent()),
                ])
            };
            let lines = vec![
                Line::raw(""),
                input,
                Line::raw(""),
                hint(t, "enter confirm · esc cancel"),
            ];
            f.render_widget(Paragraph::new(lines).style(t.modal()), inner);
        }
        Dialog::Confirm(c) => {
            let (title, msg) = match c {
                Confirm::ClearQueue => (
                    "Clear queue",
                    "Remove all tracks from the queue?".to_string(),
                ),
                Confirm::ClearHistory => ("Clear history", "Clear all play history?".to_string()),
                Confirm::DeletePlaylist(id) => {
                    let name = app
                        .playlists
                        .iter()
                        .find(|p| p.id == *id)
                        .map_or("", |p| p.name.as_str());
                    ("Delete playlist", format!("Delete playlist \"{name}\"?"))
                }
            };
            let inner = modal(f, area, title, t.warning(), t, 48, 7);
            let lines = vec![
                Line::raw(""),
                Line::styled(msg, t.warning()),
                Line::styled("This cannot be undone.", t.secondary()),
                hint(t, "enter/y confirm · esc cancel"),
            ];
            f.render_widget(
                Paragraph::new(lines)
                    .style(t.modal())
                    .wrap(Wrap { trim: false }),
                inner,
            );
        }
        Dialog::Error { msg, scroll } => {
            let inner = modal(f, area, "Error", t.error(), t, 64, 12);
            let body = Rect {
                height: inner.height.saturating_sub(2),
                ..inner
            };
            f.render_widget(
                Paragraph::new(msg.as_str())
                    .style(t.modal().fg(t.error))
                    .wrap(Wrap { trim: false })
                    .scroll((*scroll, 0)),
                body,
            );
            let foot = Rect {
                y: inner.bottom().saturating_sub(1),
                height: 1.min(inner.height),
                ..inner
            };
            f.render_widget(
                Paragraph::new(hint(t, "j/k scroll · esc close")).style(t.modal()),
                foot,
            );
        }
        Dialog::Help { scroll } => {
            let mut lines: Vec<Line> = Vec::new();
            for (g, items) in HELP {
                if !lines.is_empty() {
                    lines.push(Line::raw(""));
                }
                lines.push(Line::styled(*g, t.focused_title()));
                for (a, desc) in *items {
                    lines.push(Line::from(vec![
                        Span::styled(fit(&app.keys.key_label(*a), 16, false), t.key()),
                        Span::styled(*desc, t.secondary()),
                    ]));
                }
            }
            let inner = modal(f, area, "Help", t.accent(), t, 60, lines.len() as u16 + 4);
            let body = Rect {
                height: inner.height.saturating_sub(2),
                ..inner
            };
            f.render_widget(
                Paragraph::new(lines).style(t.modal()).scroll((*scroll, 0)),
                body,
            );
            let foot = Rect {
                y: inner.bottom().saturating_sub(1),
                height: 1.min(inner.height),
                ..inner
            };
            f.render_widget(
                Paragraph::new(hint(t, "j/k scroll · esc/q close")).style(t.modal()),
                foot,
            );
        }
        Dialog::Picker { tracks, sel } => {
            let h = app.playlists.len().min(10) as u16 + 5;
            let inner = modal(f, area, "Add to playlist", t.accent(), t, 48, h);
            let n = tracks.len();
            let sub = format!("{n} track{}", if n == 1 { "" } else { "s" });
            f.render_widget(
                Paragraph::new(Line::styled(sub, t.secondary())).style(t.modal()),
                Rect {
                    height: 1.min(inner.height),
                    ..inner
                },
            );
            let list = Rect {
                y: inner.y + 1,
                height: inner.height.saturating_sub(3),
                ..inner
            };
            let w = list.width as usize;
            draw_rows(f, t, list, Some(*sel), app.playlists.len(), |i, cur| {
                let p = &app.playlists[i];
                let count = format!("{}", p.track_count);
                let cols = [
                    Col::new(&p.name, 0),
                    Col::new(&count, DUR_W.min(w.saturating_sub(MARKER_W + 4))).right(),
                ];
                super::widgets::track_row(t, w, cur, super::widgets::Mark::None, false, &cols, None)
            });
            let foot = Rect {
                y: inner.bottom().saturating_sub(1),
                height: 1.min(inner.height),
                ..inner
            };
            let hl =
                truncate("j/k select · enter add · esc cancel", inner.width as usize).into_owned();
            f.render_widget(Paragraph::new(hint(t, &hl)).style(t.modal()), foot);
        }
    }
}
