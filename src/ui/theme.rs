use ratatui::style::{Color, Modifier, Style};

/// Semantic color tokens (SPEC palette). Widgets must use these, never literal colors.
#[derive(Debug, Clone)]
pub struct Theme {
    pub bg: Color,
    pub surface: Color,
    pub raised: Color,
    pub text: Color,
    pub text_secondary: Color,
    pub text_muted: Color,
    pub accent: Color,
    pub accent_alt: Color,
    pub playing: Color,
    pub warning: Color,
    pub error: Color,
    pub border: Color,
}

impl Default for Theme {
    fn default() -> Self {
        let c = Color::Rgb;
        Self {
            bg: c(0x0B, 0x0F, 0x14),
            surface: c(0x12, 0x19, 0x22),
            raised: c(0x19, 0x23, 0x2D),
            text: c(0xE6, 0xED, 0xF3),
            text_secondary: c(0x81, 0x90, 0x9F),
            text_muted: c(0x52, 0x60, 0x6E),
            accent: c(0x51, 0xD6, 0xD0),
            accent_alt: c(0x60, 0x96, 0xE8),
            playing: c(0x79, 0xD8, 0xA0),
            warning: c(0xE4, 0xB4, 0x6A),
            error: c(0xE4, 0x77, 0x7C),
            border: c(0x29, 0x35, 0x40),
        }
    }
}

impl Theme {
    pub fn base(&self) -> Style {
        Style::new().fg(self.text).bg(self.bg)
    }
    pub fn selected(&self) -> Style {
        Style::new().fg(self.text).bg(self.raised)
    }
    pub fn focused_title(&self) -> Style {
        Style::new().fg(self.accent).add_modifier(Modifier::BOLD)
    }
    pub fn muted(&self) -> Style {
        Style::new().fg(self.text_muted)
    }
    pub fn secondary(&self) -> Style {
        Style::new().fg(self.text_secondary)
    }
    pub fn border(&self) -> Style {
        Style::new().fg(self.border)
    }
    pub fn playing(&self) -> Style {
        Style::new().fg(self.playing)
    }
    pub fn error(&self) -> Style {
        Style::new().fg(self.error)
    }
    pub fn warning(&self) -> Style {
        Style::new().fg(self.warning)
    }
    pub fn accent(&self) -> Style {
        Style::new().fg(self.accent)
    }
    /// Queued marker / position.
    pub fn queued(&self) -> Style {
        Style::new().fg(self.accent_alt)
    }
    /// Bottom player panel and other panel surfaces.
    pub fn panel(&self) -> Style {
        Style::new().fg(self.text).bg(self.surface)
    }
    /// Modal dialog body.
    pub fn modal(&self) -> Style {
        Style::new().fg(self.text).bg(self.raised)
    }
    /// Header tab: active = accent bold, inactive = secondary.
    pub fn tab(&self, active: bool) -> Style {
        if active {
            self.focused_title()
        } else {
            self.secondary()
        }
    }
    /// Shortcut key in a hint line.
    pub fn key(&self) -> Style {
        self.accent()
    }
    pub fn text_style(&self) -> Style {
        Style::new().fg(self.text)
    }
    /// Prominent text (titles).
    pub fn strong(&self) -> Style {
        Style::new().fg(self.text).add_modifier(Modifier::BOLD)
    }
}

/// Parse `#RRGGBB` (leading `#` optional) into an RGB color.
pub fn parse_hex(s: &str) -> Option<Color> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let n = u32::from_str_radix(h, 16).ok()?;
    Some(Color::Rgb((n >> 16) as u8, (n >> 8) as u8, n as u8))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex() {
        assert_eq!(parse_hex("#51D6D0"), Some(Color::Rgb(0x51, 0xD6, 0xD0)));
        assert_eq!(parse_hex("0b0f14"), Some(Color::Rgb(0x0B, 0x0F, 0x14)));
        assert_eq!(parse_hex("#12345"), None);
        assert_eq!(parse_hex("#GGGGGG"), None);
        assert_eq!(parse_hex("+12345"), None);
    }
}
