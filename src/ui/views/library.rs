//! Library view: Artists | Albums | Tracks, collapsing with the breakpoint.
use std::collections::HashMap;
use std::path::Path;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::ui::layout::Breakpoint;
use crate::ui::theme::Theme;
use crate::ui::widgets::{
    Col, Mark, empty_state, fit, fmt_thousands, pane_title, track_row, truncate, window,
};

// Same rule as the grouping in app::views::library (that module is not nameable from here).
fn artist_of(t: &crate::library::Track) -> &str {
    t.artist
        .as_deref()
        .or(t.album_artist.as_deref())
        .unwrap_or("Unknown artist")
}

fn album_of(t: &crate::library::Track) -> &str {
    t.album.as_deref().unwrap_or("Unknown album")
}

pub fn draw(f: &mut Frame, app: &App, area: Rect, bp: Breakpoint) {
    if area.is_empty() {
        return;
    }
    let t = &app.theme;
    if app.tracks.is_empty() {
        empty_state(
            f,
            t,
            area,
            "No media indexed",
            "add media_dirs to config.toml, then press u to rescan (or browse files with 2)",
        );
        return;
    }
    let ui = &app.library;
    let panes = bp.library_panes().clamp(1, 3);
    ui.shown.set(panes);
    let (fa, fb, fc) = ui.focused();
    let sep = |f: &mut Frame, r: Rect| {
        let lines = vec![Line::styled("│", t.border()); r.height as usize];
        f.render_widget(Paragraph::new(lines), r);
    };
    let tracks_area = match panes {
        3 => {
            let [a, s1, b, s2, c] = Layout::horizontal([
                Constraint::Fill(22),
                Constraint::Length(1),
                Constraint::Fill(24),
                Constraint::Length(1),
                Constraint::Fill(54),
            ])
            .areas(area);
            sep(f, s1);
            sep(f, s2);
            draw_artists(f, app, a, fa);
            draw_albums(f, app, b, fb);
            c
        }
        2 => {
            let [a, s, c] = Layout::horizontal([
                Constraint::Fill(30),
                Constraint::Length(1),
                Constraint::Fill(70),
            ])
            .areas(area);
            sep(f, s);
            draw_artists(f, app, a, fa);
            c
        }
        _ => area,
    };
    draw_tracks(f, app, tracks_area, fc, panes < 3, bp);
}

/// "artist › album", shrinking so the album stays readable.
fn crumb(artist: Option<&str>, album: Option<&str>, w: usize) -> String {
    let a = artist.unwrap_or("All artists");
    let full = match album {
        Some(b) => format!("{a} › {b}"),
        None => a.to_string(),
    };
    if full.width() <= w {
        return full;
    }
    let (Some(b), true) = (album, w >= 3) else {
        return truncate(&full, w).into_owned();
    };
    let room = w - 3;
    let bw = b.width().min(room * 3 / 5);
    let aw = a.width().min(room - bw);
    let bw = room - aw;
    format!("{} › {}", truncate(a, aw), truncate(b, bw))
}

fn group_row(
    t: &Theme,
    w: usize,
    cursor: bool,
    focused: bool,
    text: &str,
    all: bool,
) -> Line<'static> {
    let cur = match (cursor, focused) {
        (true, true) => Span::styled("›", t.accent()),
        (true, false) => Span::styled("›", t.secondary()),
        _ => Span::raw(" "),
    };
    let style = if all {
        t.secondary()
    } else if cursor && !focused {
        t.strong()
    } else {
        t.text_style()
    };
    Line::from(vec![
        cur,
        Span::raw(" "),
        Span::styled(fit(text, w.saturating_sub(2), false), style),
    ])
}

/// Title row + body; only the visible window of rows is built. The raised highlight is for the
/// focused pane only, so an unfocused pane keeps a quiet `›` on its selection.
fn draw_group<'a>(
    f: &mut Frame,
    t: &Theme,
    area: Rect,
    title: &str,
    (sel, len): (Option<usize>, usize),
    focused: bool,
    row: impl Fn(usize) -> &'a str,
) {
    if area.height == 0 {
        return;
    }
    let [head, body] = Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(area);
    let title = truncate(title, (area.width as usize).saturating_sub(1));
    f.render_widget(Paragraph::new(pane_title(t, &title, focused)), head);
    let w = area.width as usize;
    let (a, b) = window(sel, len, body.height as usize);
    let lines: Vec<Line> = (a..b)
        .map(|i| {
            let l = group_row(t, w, sel == Some(i), focused, row(i), i == 0);
            if focused && sel == Some(i) {
                l.style(t.selected())
            } else {
                l
            }
        })
        .collect();
    f.render_widget(Paragraph::new(lines), body);
}

fn draw_artists(f: &mut Frame, app: &App, area: Rect, focused: bool) {
    let ui = &app.library;
    let artists = &ui.groups.artists;
    draw_group(
        f,
        &app.theme,
        area,
        "Artists",
        (ui.artist_list.selected(), artists.len() + 1),
        focused,
        |i| {
            if i == 0 {
                "All artists"
            } else {
                &artists[i - 1]
            }
        },
    );
}

fn draw_albums(f: &mut Frame, app: &App, area: Rect, focused: bool) {
    let ui = &app.library;
    draw_group(
        f,
        &app.theme,
        area,
        "Albums",
        (ui.album_list.selected(), ui.albums.len() + 1),
        focused,
        |i| {
            if i == 0 {
                "All albums"
            } else {
                &ui.albums[i - 1]
            }
        },
    );
}

fn draw_tracks(
    f: &mut Frame,
    app: &App,
    area: Rect,
    focused: bool,
    show_crumb: bool,
    bp: Breakpoint,
) {
    if area.height == 0 {
        return;
    }
    let t = &app.theme;
    let ui = &app.library;
    let [head, body] = Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(area);
    let w = area.width as usize;
    let count = fmt_thousands(ui.filtered.len());
    let label = if show_crumb {
        let room = w.saturating_sub(count.width() + 4);
        format!("{} · {count}", crumb(ui.sel_artist(), ui.sel_album(), room))
    } else {
        format!("Tracks · {count}")
    };
    f.render_widget(
        Paragraph::new(pane_title(
            t,
            &truncate(&label, w.saturating_sub(1)),
            focused,
        )),
        head,
    );
    if ui.filtered.is_empty() {
        let hint = if ui.filter.is_empty() {
            "pick another artist or album"
        } else {
            "refine or clear the search"
        };
        empty_state(f, t, body, "No matching tracks", hint);
        return;
    }
    let playing = app
        .player
        .snapshot()
        .current
        .as_ref()
        .map(|c| c.path.as_path());
    let queued: HashMap<&Path, usize> = app
        .player
        .queue()
        .entries()
        .iter()
        .enumerate()
        .rev() // first occurrence wins
        .map(|(i, e)| (e.track.path.as_path(), i + 1))
        .collect();
    let wide = w >= 50;
    let show_artist = wide && ui.sel_artist().is_none();
    let show_album = wide && bp.optional_columns() && ui.sel_album().is_none();
    let video = ui.groups.has_video && wide;
    draw_rows_focus(
        f,
        t,
        body,
        ui.list.selected(),
        ui.filtered.len(),
        focused,
        |i, cur| {
            let tr = &app.tracks[ui.filtered[i]];
            let no = tr.track_no.map(|n| format!("{n:02}")).unwrap_or_default();
            let mark = if playing == Some(tr.path.as_path()) {
                Mark::Playing
            } else {
                queued
                    .get(tr.path.as_path())
                    .map_or(Mark::None, |&n| Mark::Queued(Some(n)))
            };
            let sec = t.secondary();
            let mut cols = vec![
                Col::new(&no, 2).right().style(sec),
                Col::new(&tr.title, 0).weight(25),
            ];
            if show_artist {
                cols.push(Col::new(artist_of(tr), 0).weight(14).style(sec));
            }
            if show_album {
                cols.push(Col::new(album_of(tr), 0).weight(11).style(sec));
            }
            if video {
                cols.push(Col::new(if tr.is_video { "VIDEO" } else { "" }, 5).style(t.accent()));
            }
            let mut l = track_row(t, w, cur, mark, false, &cols, Some(tr.duration));
            if mark == Mark::Playing && !cur {
                l = l.patch_style(t.playing());
            }
            l
        },
    );
}

/// `draw_rows`, but the raised highlight only when the pane is focused.
fn draw_rows_focus(
    f: &mut Frame,
    t: &Theme,
    area: Rect,
    sel: Option<usize>,
    len: usize,
    focused: bool,
    row: impl Fn(usize, bool) -> Line<'static>,
) {
    let (a, b) = window(sel, len, area.height as usize);
    let lines: Vec<Line> = (a..b)
        .map(|i| {
            let cur = sel == Some(i);
            let l = row(i, cur);
            if cur && focused {
                l.style(t.selected())
            } else {
                l
            }
        })
        .collect();
    f.render_widget(Paragraph::new(lines), area);
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::config::keys::KeyMap;
    use crate::library::Track;
    use crate::library::database::Database;
    use crate::playback::Player;

    fn tr(n: usize, title: &str, artist: &str, album: &str) -> Track {
        Track {
            id: 0,
            path: PathBuf::from(format!("/x/{n}.mp3")),
            title: title.into(),
            artist: Some(artist.into()),
            album: Some(album.into()),
            album_artist: None,
            genre: None,
            track_no: Some(n as u32),
            year: None,
            duration: Some(185.0),
            format: None,
            size: 0,
            width: None,
            height: None,
            is_video: n.is_multiple_of(2),
        }
    }

    fn app(tracks: Vec<Track>) -> App {
        let (tx, _rx) = std::sync::mpsc::channel();
        let db = Database::open_in_memory().unwrap();
        let keys = KeyMap::from_config(&Default::default(), &mut Vec::new());
        let mut a = App::new(tx.clone(), Player::new(tx, 100), db, keys, Theme::default());
        a.tracks = tracks;
        a.refilter();
        a
    }

    fn render(a: &App, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| {
            let r = f.area();
            draw(f, a, r, Breakpoint::of(r));
        })
        .unwrap();
        term.backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    #[test]
    fn renders_at_all_sizes() {
        let long = "とても長いタイトル🎵".repeat(8);
        let a = app(vec![
            tr(1, "Alpha", "Zed", "First"),
            tr(2, &long, &long, &long),
            tr(3, "Gamma", "Zed", "First"),
        ]);
        for (w, h) in [
            (120, 35),
            (100, 30),
            (80, 24),
            (60, 20),
            (30, 10),
            (1, 1),
            (0, 0),
        ] {
            let s = render(&a, w, h);
            if w >= 30 {
                assert!(s.contains("Alpha"), "{w}x{h}");
            }
            if w >= 60 {
                assert!(s.contains("Tracks") || s.contains("All artists"), "{w}x{h}");
            }
        }
        assert!(render(&a, 120, 35).contains("VIDEO"));
    }

    #[test]
    fn standard_has_two_panes_and_playing_row_is_marked() {
        let mut a = app(vec![
            tr(1, "Alpha", "Zed", "First"),
            tr(3, "Gamma", "Zed", "First"),
        ]);
        // unindexed (id 0) queue entry matched by path
        let mut t = a.tracks[1].clone();
        t.path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        a.tracks[1].path = t.path.clone();
        a.command(crate::event::Command::PlayTracks {
            tracks: vec![t],
            start: 0,
        });
        let s = render(&a, 100, 30);
        assert!(
            !s.contains("Albums"),
            "Standard folds albums into the crumb"
        );
        assert!(s.contains('▶'));
    }

    #[test]
    fn empty_library_hint() {
        let s = render(&app(vec![]), 120, 35);
        assert!(s.contains("No media indexed") && s.contains("press u to rescan"));
    }

    #[test]
    fn crumb_elides_artist_first() {
        assert_eq!(crumb(None, None, 20), "All artists");
        assert_eq!(crumb(Some("A"), Some("B"), 20), "A › B");
        let c = crumb(Some("Very Long Artist Name"), Some("Short Album"), 24);
        assert!(c.width() <= 24 && c.contains("Short Album"), "{c}");
        assert!(
            crumb(
                Some("x".repeat(30).as_str()),
                Some("y".repeat(30).as_str()),
                2
            )
            .width()
                <= 2
        );
        assert!(crumb(Some("日本語の名前"), Some("アルバム名前"), 9).width() <= 9);
    }
}
