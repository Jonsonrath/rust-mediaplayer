//! Persistent bottom player panel: title, state, progress, volume, repeat, shuffle, scan activity.
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use super::layout::Breakpoint;
use super::theme::Theme;
use super::widgets::{fit, fmt_thousands, fmt_time, progress_bar, separator, truncate};
use crate::app::App;
use crate::playback::modes::RepeatMode;
use crate::playback::{PlayState, PlayerSnapshot};

pub fn draw(f: &mut Frame, app: &App, area: Rect, bp: Breakpoint) {
    let scan = app.scanning.then_some(app.scan_count);
    draw_panel(f, &app.theme, app.player.snapshot(), scan, area, bp);
}

pub fn draw_panel(
    f: &mut Frame,
    t: &Theme,
    s: &PlayerSnapshot,
    scan: Option<usize>,
    area: Rect,
    bp: Breakpoint,
) {
    f.render_widget(Block::new().style(t.panel()), area);
    if area.is_empty() {
        return;
    }
    let mut body = area;
    let mut lines = Vec::with_capacity(4);
    if bp.player_height() >= 5 {
        f.render_widget(
            Paragraph::new(separator(t, area.width)),
            Rect { height: 1, ..area },
        );
        body.y += 1;
        body.height -= 1;
        lines.push(Line::styled("NOW PLAYING", t.muted()));
    }
    let inner = Rect {
        x: body.x + 1.min(body.width),
        width: body.width.saturating_sub(2),
        ..body
    };
    let w = inner.width as usize;
    let narrow = bp == Breakpoint::Narrow;
    lines.push(title_line(t, s, w, narrow));
    lines.push(bar_line(t, s, w));
    lines.push(status_line(t, s, scan, w));
    f.render_widget(Paragraph::new(lines).style(t.panel()), inner);
}

fn spans_w(v: &[Span]) -> usize {
    v.iter().map(Span::width).sum()
}

fn mode_spans(
    t: &Theme,
    s: &PlayerSnapshot,
    compact: bool,
) -> (Vec<Span<'static>>, Vec<Span<'static>>) {
    let (rt, rs) = match (s.repeat, compact) {
        (RepeatMode::Off, false) => ("↻ REPEAT OFF", t.muted()),
        (RepeatMode::All, false) => ("↻ REPEAT ALL", t.accent()),
        (RepeatMode::One, false) => ("↻¹ REPEAT ONE", t.accent().add_modifier(Modifier::BOLD)),
        (RepeatMode::Off, true) => ("↻ OFF", t.muted()),
        (RepeatMode::All, true) => ("↻ ALL", t.accent()),
        (RepeatMode::One, true) => ("↻¹ ONE", t.accent().add_modifier(Modifier::BOLD)),
    };
    let (st, ss) = match (s.shuffle, compact) {
        (true, false) => ("⤮ SHUFFLE ON", t.accent()),
        (false, false) => ("⤮ SHUFFLE OFF", t.muted()),
        (true, true) => ("⤮ ON", t.accent()),
        (false, true) => ("⤮ OFF", t.muted()),
    };
    let (rw, sw) = if compact { (6, 5) } else { (13, 13) };
    (
        vec![Span::styled(fit(rt, rw, false), rs)],
        vec![Span::styled(fit(st, sw, false), ss)],
    )
}

fn title_line(t: &Theme, s: &PlayerSnapshot, w: usize, narrow: bool) -> Line<'static> {
    let (glyph, label, style) = match s.state {
        PlayState::Playing => ("▶", "PLAYING", t.playing()),
        PlayState::Paused => ("⏸", "PAUSED", t.warning()),
        PlayState::Stopped => ("■", "STOPPED", t.muted()),
    };
    let mut right = Vec::new();
    if s.current.as_ref().is_some_and(|c| c.is_video) {
        right.push(Span::styled("VIDEO", t.queued()));
        right.push(Span::raw("  "));
    }
    right.push(Span::styled(
        if narrow {
            glyph.to_string()
        } else {
            format!("{glyph} {label}")
        },
        style,
    ));
    let rw = spans_w(&right);
    let budget = w.saturating_sub(rw + 2);
    let mut spans = match &s.current {
        None => vec![Span::styled(
            truncate("Nothing playing — Enter to play, a to queue", budget).into_owned(),
            t.muted(),
        )],
        Some(c) => {
            let title = truncate(&c.title, budget).into_owned();
            let mut left = budget.saturating_sub(spans_w(&[Span::raw(title.clone())]));
            let mut v = vec![Span::styled(title, t.strong())];
            for (sep, text) in [(" — ", &c.artist), (" · ", &c.album)] {
                let Some(text) = text else { continue };
                if left <= sep.chars().count() + 1 {
                    break;
                }
                let part = truncate(text, left - sep.chars().count()).into_owned();
                left -= sep.chars().count() + Span::raw(part.clone()).width();
                v.push(Span::styled(format!("{sep}{part}"), t.secondary()));
            }
            v
        }
    };
    let used = spans_w(&spans);
    spans.push(Span::raw(" ".repeat(w.saturating_sub(used + rw))));
    spans.extend(right);
    Line::from(spans)
}

fn bar_line(t: &Theme, s: &PlayerSnapshot, w: usize) -> Line<'static> {
    let dur = s.duration.filter(|d| d.is_finite() && *d > 0.0);
    let tw = if s.position >= 3600.0 || dur.is_some_and(|d| d >= 3600.0) {
        7
    } else {
        5
    };
    let ratio = dur.map(|d| s.position / d);
    let bar_w = w.saturating_sub(2 * tw + 4);
    if bar_w < 4 {
        return progress_bar(t, w, ratio);
    }
    let idle = s.current.is_none();
    let el = if idle {
        "--:--".into()
    } else {
        fmt_time(s.position)
    };
    let tot = dur.map_or("--:--".to_string(), fmt_time);
    let mut spans = vec![
        Span::styled(fit(&el, tw, true), t.secondary()),
        Span::raw("  "),
    ];
    spans.extend(progress_bar(t, bar_w, if idle { None } else { ratio }).spans);
    spans.push(Span::raw("  "));
    spans.push(Span::styled(fit(&tot, tw, true), t.secondary()));
    Line::from(spans)
}

fn status_line(t: &Theme, s: &PlayerSnapshot, scan: Option<usize>, w: usize) -> Line<'static> {
    let vol = |compact: bool| {
        let n = format!("{:>3}%", s.volume);
        vec![
            Span::styled(if compact { "VOL " } else { "VOL  " }, t.muted()),
            Span::styled(n, t.text_style()),
        ]
    };
    let build = |compact: bool| {
        let (r, sh) = mode_spans(t, s, compact);
        vec![vol(compact), r, sh]
    };
    let join = |items: &[Vec<Span<'static>>]| -> Vec<Span<'static>> {
        let mut v = Vec::new();
        for (i, it) in items.iter().enumerate() {
            if i > 0 {
                v.push(Span::raw("   "));
            }
            v.extend(it.iter().cloned());
        }
        v
    };
    let mut items = build(false);
    if spans_w(&join(&items)) > w {
        items = build(true);
    }
    while items.len() > 1 && spans_w(&join(&items)) > w {
        items.pop();
    }
    let mut spans = join(&items);
    if let Some(n) = scan {
        let text = format!("SCANNING {}", fmt_thousands(n));
        let free = w.saturating_sub(spans_w(&spans) + 2);
        if free >= text.chars().count() {
            spans.push(Span::raw(" ".repeat(free - text.chars().count() + 2)));
            spans.push(Span::styled(text, t.accent()));
        }
    }
    Line::from(spans)
}
