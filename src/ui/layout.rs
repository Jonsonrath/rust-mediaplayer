//! Responsive breakpoints and the main screen split.
use ratatui::layout::{Constraint, Layout, Rect};

/// Terminal size class. Views pick columns/panes from this instead of raw sizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Breakpoint {
    /// Below ~40x12: only a "resize" message.
    TooSmall,
    /// >= 60x20 (down to 40x12): one pane.
    Narrow,
    /// >= 80x24: library 2 panes, optional columns dropped.
    Compact,
    /// >= 100x30
    Standard,
    /// >= 120x35
    Full,
}

impl Breakpoint {
    pub fn of(a: Rect) -> Self {
        let (w, h) = (a.width, a.height);
        if w >= 120 && h >= 35 {
            Self::Full
        } else if w >= 100 && h >= 30 {
            Self::Standard
        } else if w >= 80 && h >= 24 {
            Self::Compact
        } else if w >= 40 && h >= 12 {
            Self::Narrow
        } else {
            Self::TooSmall
        }
    }

    /// Library panes shown: Full: artists | albums | tracks; Standard/Compact: artists | tracks (album in the crumb); Narrow: tracks only.
    pub fn library_panes(self) -> usize {
        match self {
            Self::Full => 3,
            Self::Standard | Self::Compact => 2,
            _ => 1,
        }
    }

    /// Whether optional columns (album, format, year, ...) should be drawn.
    pub fn optional_columns(self) -> bool {
        self >= Self::Standard
    }

    /// Player panel height in lines (including its top rule when 5).
    pub fn player_height(self) -> u16 {
        match self {
            Self::Full | Self::Standard => 5,
            Self::Compact | Self::Narrow => 3,
            _ => 2,
        }
    }
}

/// The fixed screen regions.
pub struct Regions {
    pub header: Rect,
    pub separator: Rect,
    pub body: Rect,
    pub player: Rect,
    pub footer: Rect,
}

pub fn split(area: Rect, bp: Breakpoint) -> Regions {
    let [header, separator, body, player, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(bp.player_height()),
        Constraint::Length(1),
    ])
    .areas(area);
    Regions {
        header,
        separator,
        body,
        player,
        footer,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breakpoints() {
        let b = |w, h| Breakpoint::of(Rect::new(0, 0, w, h));
        assert_eq!(b(120, 35), Breakpoint::Full);
        assert_eq!(b(200, 34), Breakpoint::Standard);
        assert_eq!(b(100, 30), Breakpoint::Standard);
        assert_eq!(b(80, 24), Breakpoint::Compact);
        assert_eq!(b(60, 20), Breakpoint::Narrow);
        assert_eq!(b(40, 12), Breakpoint::Narrow);
        assert_eq!(b(39, 40), Breakpoint::TooSmall);
        assert_eq!(b(0, 0), Breakpoint::TooSmall);
    }

    #[test]
    fn split_fits() {
        let r = split(Rect::new(0, 0, 80, 24), Breakpoint::Compact);
        assert_eq!(r.body.height, 24 - 2 - 3 - 1);
        assert_eq!(r.footer.y, 23);
    }
}
