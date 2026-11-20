/// Repeat mode; cycles Off -> All -> One -> Off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RepeatMode {
    #[default]
    Off,
    All,
    One,
}

impl RepeatMode {
    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::All,
            Self::All => Self::One,
            Self::One => Self::Off,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycles() {
        let m = RepeatMode::Off;
        assert_eq!(m.next(), RepeatMode::All);
        assert_eq!(m.next().next(), RepeatMode::One);
        assert_eq!(m.next().next().next(), RepeatMode::Off);
    }
}
