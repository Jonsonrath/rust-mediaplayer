//! Shared drawing helpers: cell-width text, pane titles, track rows, progress bar, lists.
//! All widths are terminal cells (unicode-width); strings are never byte-sliced.
use std::borrow::Cow;

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::theme::Theme;

/// Cursor (1) + status (3) cells at the start of every track row.
pub const MARKER_W: usize = 5;
/// Width of a right-aligned duration cell ("1:02:05").
pub const DUR_W: usize = 7;

/// Truncate to at most `max_width` terminal cells, appending `…` when cut. Never splits a char.
pub fn truncate(s: &str, max_width: usize) -> Cow<'_, str> {
    if s.width() <= max_width {
        return Cow::Borrowed(s);
    }
    if max_width == 0 {
        return Cow::Borrowed("");
    }
    let mut out = String::new();
    let mut w = 0;
    for ch in s.chars() {
        let cw = ch.width().unwrap_or(0);
        if w + cw > max_width - 1 {
            break;
        }
        out.push(ch);
        w += cw;
    }
    out.push('…');
    Cow::Owned(out)
}

/// Truncate then pad with spaces to exactly `w` cells (left- or right-aligned).
pub fn fit(s: &str, w: usize, right: bool) -> String {
    let t = truncate(s, w);
    let pad = " ".repeat(w.saturating_sub(t.width()));
    if right {
        pad + &t
    } else {
        t.into_owned() + &pad
    }
}

/// The last `max_width` cells of `s` (prefixed with `…` when cut): the visible end of an input.
pub fn tail(s: &str, max_width: usize) -> String {
    if s.width() <= max_width {
        return s.to_string();
    }
    let mut w = 1; // the ellipsis
    let mut rev: Vec<char> = Vec::new();
    for ch in s.chars().rev() {
        let cw = ch.width().unwrap_or(0);
        if w + cw > max_width {
            break;
        }
        rev.push(ch);
        w += cw;
    }
    let mut out = String::from("…");
    out.extend(rev.into_iter().rev());
    if max_width == 0 { String::new() } else { out }
}

/// `mm:ss`, or `h:mm:ss` from one hour up. Negative/NaN clamp to 0.
pub fn fmt_time(secs: f64) -> String {
    let t = if secs.is_finite() && secs > 0.0 {
        secs as u64
    } else {
        0
    };
    let (h, m, s) = (t / 3600, t / 60 % 60, t % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// `1234567` -> `1,234,567`.
pub fn fmt_thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Pane heading. Focused: accent bar + accent bold title; unfocused: secondary. Same width either way.
pub fn pane_title(t: &Theme, title: &str, focused: bool) -> Line<'static> {
    if focused {
        Line::from(vec![
            Span::styled("▍", t.accent()),
            Span::styled(title.to_string(), t.focused_title()),
        ])
    } else {
        Line::from(vec![
            Span::raw(" "),
            Span::styled(title.to_string(), t.secondary()),
        ])
    }
}

/// Thin horizontal rule in the border color.
pub fn separator(t: &Theme, width: u16) -> Line<'static> {
    Line::styled("─".repeat(width as usize), t.border())
}

/// Status cell of a track row. Independent of the cursor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mark {
    None,
    /// Currently playing: `▶` in the playing color.
    Playing,
    /// In the queue; `Some(n)` shows the 1-based position.
    Queued(Option<usize>),
}

/// One data column of a track row. `width == 0` is a flexible column (shares leftover space).
pub struct Col<'a> {
    pub text: &'a str,
    pub width: usize,
    pub right: bool,
    pub style: Option<Style>,
    /// Share of the leftover space for flexible columns.
    pub weight: usize,
}

impl<'a> Col<'a> {
    pub fn new(text: &'a str, width: usize) -> Self {
        Self {
            text,
            width,
            right: false,
            style: None,
            weight: 1,
        }
    }
    pub fn weight(mut self, w: usize) -> Self {
        self.weight = w.max(1);
        self
    }
    pub fn right(mut self) -> Self {
        self.right = true;
        self
    }
    pub fn style(mut self, s: Style) -> Self {
        self.style = Some(s);
        self
    }
}

/// Build one list row: `[cursor][status 3] col  col  …  duration`.
/// `muted` greys the whole row (unavailable/unsupported). Row highlight is added by `draw_rows`.
pub fn track_row(
    t: &Theme,
    width: usize,
    cursor: bool,
    mark: Mark,
    muted: bool,
    cols: &[Col],
    duration: Option<Option<f64>>,
) -> Line<'static> {
    let mut spans = Vec::with_capacity(cols.len() * 2 + 3);
    spans.push(if cursor {
        Span::styled("›", t.accent())
    } else {
        Span::raw(" ")
    });
    spans.push(match mark {
        Mark::Playing => Span::styled(" ▶  ", t.playing()),
        Mark::Queued(n) => {
            let s = n.map_or("·".to_string(), |n| format!("·{}", n.min(99)));
            Span::styled(format!("{} ", fit(&s, 3, false)), t.queued())
        }
        Mark::None => Span::raw("    "),
    });
    let dur_w = if duration.is_some() { DUR_W + 2 } else { 0 };
    let avail = width.saturating_sub(MARKER_W + dur_w);
    let gaps = cols.len().saturating_sub(1) * 2;
    let fixed: usize = cols.iter().map(|c| c.width).sum();
    let mut weight_left: usize = cols.iter().filter(|c| c.width == 0).map(|c| c.weight).sum();
    let flex_total = avail.saturating_sub(fixed + gaps);
    let mut flex_rest = flex_total;
    let base = if muted {
        t.muted()
    } else {
        Style::new().fg(t.text)
    };
    for (i, c) in cols.iter().enumerate() {
        let w = if c.width == 0 {
            let w = flex_rest * c.weight / weight_left.max(1);
            flex_rest -= w;
            weight_left -= c.weight;
            w
        } else {
            c.width
        };
        if i > 0 {
            spans.push(Span::raw("  "));
        }
        let style = if muted {
            t.muted()
        } else {
            c.style.unwrap_or(base)
        };
        spans.push(Span::styled(fit(c.text, w, c.right), style));
    }
    if let Some(d) = duration {
        let s = d.map_or("--:--".to_string(), fmt_time);
        spans.push(Span::raw("  "));
        spans.push(Span::styled(fit(&s, DUR_W, true), t.secondary()));
    }
    Line::from(spans)
}

/// Visible `[start, end)` of `len` rows in `height` lines, keeping `selected` in view.
/// Stateless (centers the selection), so draw fns need no mutable scroll state.
pub fn window(selected: Option<usize>, len: usize, height: usize) -> (usize, usize) {
    let max_start = len.saturating_sub(height);
    let start = selected
        .map_or(0, |s| s.saturating_sub(height / 2))
        .min(max_start);
    (start, (start + height).min(len))
}

/// Render only the visible rows of a list. `row(index, is_cursor)` builds each line; the cursor row
/// gets the raised highlight across the full width.
pub fn draw_rows(
    f: &mut Frame,
    t: &Theme,
    area: Rect,
    selected: Option<usize>,
    len: usize,
    mut row: impl FnMut(usize, bool) -> Line<'static>,
) {
    let (a, b) = window(selected, len, area.height as usize);
    let lines: Vec<Line> = (a..b)
        .map(|i| {
            let cur = selected == Some(i);
            let l = row(i, cur);
            if cur { l.style(t.selected()) } else { l }
        })
        .collect();
    f.render_widget(Paragraph::new(lines), area);
}

/// Centered helpful message for an empty pane.
pub fn empty_state(f: &mut Frame, t: &Theme, area: Rect, title: &str, hint: &str) {
    if area.is_empty() {
        return;
    }
    let top = area.y + area.height.saturating_sub(2) / 2;
    let r = Rect::new(
        area.x,
        top,
        area.width,
        area.bottom().saturating_sub(top).min(4),
    );
    let lines = vec![
        Line::styled(title.to_string(), t.secondary()),
        Line::styled(hint.to_string(), t.muted()),
    ];
    f.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true }),
        r,
    );
}

/// Progress line exactly `width` cells: `━━━●━━━`. `None` (unknown duration) is a dim dashed line.
pub fn progress_bar(t: &Theme, width: usize, ratio: Option<f64>) -> Line<'static> {
    let Some(r) = ratio else {
        return Line::styled("╌".repeat(width), t.muted());
    };
    if width == 0 {
        return Line::default();
    }
    let r = if r.is_finite() {
        r.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let knob = ((width - 1) as f64 * r).round() as usize;
    Line::from(vec![
        Span::styled("━".repeat(knob), t.accent()),
        Span::styled("●", Style::new().fg(t.accent)),
        Span::styled("━".repeat(width - 1 - knob), t.border()),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates() {
        assert_eq!(truncate("hello", 5), "hello");
        assert_eq!(truncate("hello world", 6), "hello…");
        assert_eq!(truncate("hello", 0), "");
        // CJK chars are 2 cells: 5 cells fit "日本" (4) + "…" (1)
        assert_eq!(truncate("日本語テキスト", 5), "日本…");
        // a wide char that would straddle the limit is dropped
        assert_eq!(truncate("a日本", 3), "a…");
        assert_eq!(truncate("🎵🎵🎵", 4), "🎵…");
        assert_eq!(truncate("日本", 4), "日本");
    }

    #[test]
    fn fits_and_tails() {
        assert_eq!(fit("ab", 4, false), "ab  ");
        assert_eq!(fit("ab", 4, true), "  ab");
        assert_eq!(fit("日本語", 5, false).width(), 5);
        assert_eq!(tail("hello world", 6), "…world");
        assert_eq!(tail("abc", 6), "abc");
        assert_eq!(tail("日本語", 5), "…本語");
        assert_eq!(tail("abc", 0), "");
    }

    #[test]
    fn times() {
        assert_eq!(fmt_time(84.9), "01:24");
        assert_eq!(fmt_time(3725.0), "1:02:05");
        assert_eq!(fmt_time(-1.0), "00:00");
        assert_eq!(fmt_time(f64::NAN), "00:00");
        assert_eq!(fmt_thousands(1234567), "1,234,567");
        assert_eq!(fmt_thousands(12), "12");
    }

    #[test]
    fn bar() {
        let t = Theme::default();
        let w = |l: &Line| l.width();
        assert_eq!(w(&progress_bar(&t, 20, Some(0.5))), 20);
        assert_eq!(w(&progress_bar(&t, 20, Some(7.0))), 20);
        assert_eq!(w(&progress_bar(&t, 20, Some(f64::NAN))), 20);
        assert_eq!(w(&progress_bar(&t, 20, None)), 20);
        assert_eq!(w(&progress_bar(&t, 0, Some(0.5))), 0);
        assert_eq!(w(&progress_bar(&t, 1, Some(1.0))), 1);
        assert_eq!(progress_bar(&t, 5, None).spans[0].content, "╌╌╌╌╌");
    }

    #[test]
    fn windows() {
        assert_eq!(window(Some(0), 100, 10), (0, 10));
        assert_eq!(window(Some(50), 100, 10), (45, 55));
        assert_eq!(window(Some(99), 100, 10), (90, 100));
        assert_eq!(window(None, 3, 10), (0, 3));
        assert_eq!(window(Some(2), 3, 0), (2, 2));
    }

    #[test]
    fn rows_have_exact_width() {
        let t = Theme::default();
        let cols = [Col::new("Title 日本語", 0), Col::new("Artist", 8)];
        for w in [30usize, 40, 80] {
            let l = track_row(
                &t,
                w,
                true,
                Mark::Queued(Some(3)),
                false,
                &cols,
                Some(Some(61.0)),
            );
            assert_eq!(l.width(), w, "width {w}");
        }
    }

    #[test]
    fn columns_start_at_same_offset_on_every_row() {
        let t = Theme::default();
        let titles = [
            "Untitled",
            "Track number 03",
            "日本語のタイトルがとても長い曲",
            "x",
        ];
        let nums = ["04", "10", "4", "100"];
        let marks = [
            Mark::None,
            Mark::Playing,
            Mark::Queued(None),
            Mark::Queued(Some(12)),
        ];
        for w in [60usize, 79, 100, 121] {
            let mut starts = Vec::new();
            for (i, title) in titles.iter().enumerate() {
                for (j, mark) in marks.iter().enumerate() {
                    let cols = [
                        Col::new(nums[(i + j) % 4], 4).right(),
                        Col::new(title, 0).weight(3),
                        Col::new("Björk", 0).weight(2),
                        Col::new("Album 3", 0),
                    ];
                    let l = track_row(&t, w, j % 2 == 0, *mark, false, &cols, Some(Some(141.0)));
                    assert_eq!(l.width(), w);
                    // spans: cursor, mark, then col/gap pairs; collect each column's start cell
                    let mut x = l.spans[0].width() + l.spans[1].width();
                    let mut row = Vec::new();
                    for sp in &l.spans[2..] {
                        if sp.content.as_ref() != "  " {
                            row.push(x);
                        }
                        x += sp.width();
                    }
                    starts.push(row);
                }
            }
            assert!(
                starts.windows(2).all(|p| p[0] == p[1]),
                "width {w}: {starts:?}"
            );
        }
    }
}
