//! Keybindings: action names, key-spec parsing and the default key map.
use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::Deserialize;
use serde::de::{IntoDeserializer, value};

/// Every bindable UI action. Config names are the snake_case forms, e.g. `view_library`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    ViewLibrary,
    ViewFiles,
    ViewPlaylists,
    ViewQueue,
    ViewHistory,
    Down,
    Up,
    Left,
    Right,
    Top,
    Bottom,
    PageDown,
    PageUp,
    Enter,
    TogglePause,
    Stop,
    Next,
    Prev,
    CycleRepeat,
    ToggleShuffle,
    Enqueue,
    Remove,
    MoveDown,
    MoveUp,
    Search,
    SeekBack,
    SeekForward,
    VolumeDown,
    VolumeUp,
    FocusNext,
    Help,
    Quit,
    Rescan,
    AddToPlaylist,
    NewPlaylist,
    Rename,
    Delete,
    ClearQueue,
}

/// Default bindings; an action may appear several times (e.g. `j` and the Down arrow).
const DEFAULTS: &[(Action, &str)] = &[
    (Action::ViewLibrary, "1"),
    (Action::ViewFiles, "2"),
    (Action::ViewPlaylists, "3"),
    (Action::ViewQueue, "4"),
    (Action::ViewHistory, "5"),
    (Action::Down, "j"),
    (Action::Down, "down"),
    (Action::Up, "k"),
    (Action::Up, "up"),
    (Action::Left, "h"),
    (Action::Right, "l"),
    (Action::Top, "g"),
    (Action::Bottom, "G"),
    (Action::PageDown, "ctrl-d"),
    (Action::PageDown, "pgdown"),
    (Action::PageUp, "ctrl-u"),
    (Action::PageUp, "pgup"),
    (Action::Enter, "enter"),
    (Action::TogglePause, "space"),
    (Action::Stop, "x"),
    (Action::Next, "n"),
    (Action::Prev, "p"),
    (Action::CycleRepeat, "r"),
    (Action::ToggleShuffle, "s"),
    (Action::Enqueue, "a"),
    (Action::Remove, "d"),
    (Action::MoveDown, "J"),
    (Action::MoveUp, "K"),
    (Action::Search, "/"),
    (Action::SeekBack, "left"),
    (Action::SeekForward, "right"),
    (Action::VolumeDown, "-"),
    (Action::VolumeUp, "+"),
    (Action::VolumeUp, "="),
    (Action::FocusNext, "tab"),
    (Action::Help, "?"),
    (Action::Quit, "q"),
    (Action::Quit, "ctrl-c"),
    (Action::Rescan, "u"),
    (Action::AddToPlaylist, "A"),
    (Action::NewPlaylist, "N"),
    (Action::Rename, "R"),
    (Action::Delete, "D"),
    (Action::ClearQueue, "C"),
];

/// A physical key. Shift is not part of a binding: shifted letters already arrive as uppercase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binding {
    code: KeyCode,
    ctrl: bool,
}

impl Binding {
    fn from_event(key: &KeyEvent) -> Self {
        Self {
            code: key.code,
            ctrl: key.modifiers.contains(KeyModifiers::CONTROL),
        }
    }

    /// Human-readable form used in the help screen and footer, e.g. `ctrl-d`.
    fn label(&self) -> String {
        let name = match self.code {
            KeyCode::Char(' ') => "space".to_string(),
            KeyCode::Char(c) => c.to_string(),
            KeyCode::PageUp => "pgup".to_string(),
            KeyCode::PageDown => "pgdown".to_string(),
            other => format!("{other:?}").to_lowercase(),
        };
        if self.ctrl {
            format!("ctrl-{name}")
        } else {
            name
        }
    }
}

/// Parse a key spec: a single char (`j`, `J`, `+`), a name (`enter`, `pgdown`, ...),
/// optionally prefixed with `ctrl-`.
pub fn parse_key(spec: &str) -> Option<Binding> {
    let (ctrl, name) = match spec.strip_prefix("ctrl-") {
        Some(rest) => (true, rest),
        None => (false, spec),
    };
    let code = named_key(name).or_else(|| single_char(name))?;
    Some(Binding { code, ctrl })
}

fn named_key(name: &str) -> Option<KeyCode> {
    Some(match name.to_ascii_lowercase().as_str() {
        "enter" => KeyCode::Enter,
        "esc" => KeyCode::Esc,
        "tab" => KeyCode::Tab,
        "space" => KeyCode::Char(' '),
        "backspace" => KeyCode::Backspace,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "pgup" => KeyCode::PageUp,
        "pgdown" => KeyCode::PageDown,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        _ => return None,
    })
}

fn single_char(name: &str) -> Option<KeyCode> {
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Some(KeyCode::Char(c)),
        _ => None,
    }
}

fn parse_action(name: &str) -> Option<Action> {
    let de: value::StrDeserializer<'_, value::Error> = name.into_deserializer();
    Action::deserialize(de).ok()
}

/// Maps key presses to actions. Earlier entries win; `from_config` keeps each key unique.
#[derive(Debug, Clone)]
pub struct KeyMap {
    bindings: Vec<(Binding, Action)>,
}

impl Default for KeyMap {
    fn default() -> Self {
        let bindings = DEFAULTS
            .iter()
            .filter_map(|(action, spec)| parse_key(spec).map(|key| (key, *action)))
            .collect();
        Self { bindings }
    }
}

impl KeyMap {
    /// Defaults with `[keys]` overrides applied. A user key replaces all default keys of
    /// its action. Bad names or keys produce a warning and keep the default.
    pub fn from_config(overrides: &HashMap<String, String>, warnings: &mut Vec<String>) -> Self {
        let mut map = Self::default();
        let mut entries: Vec<_> = overrides.iter().collect();
        entries.sort();
        for (name, spec) in entries {
            let Some(action) = parse_action(name) else {
                warnings.push(format!("keys: unknown action \"{name}\""));
                continue;
            };
            let Some(key) = parse_key(spec) else {
                warnings.push(format!("keys: invalid key \"{spec}\" for {name}"));
                continue;
            };
            map.rebind(action, key, spec, warnings);
        }
        map
    }

    fn rebind(&mut self, action: Action, key: Binding, spec: &str, warnings: &mut Vec<String>) {
        self.bindings.retain(|(_, a)| *a != action);
        if let Some((_, other)) = self.bindings.iter().find(|(b, _)| *b == key) {
            warnings.push(format!(
                "keys: \"{spec}\" was also bound to {other:?}; it now runs {action:?}"
            ));
        }
        self.bindings.retain(|(b, _)| *b != key);
        self.bindings.push((key, action));
    }

    /// The action bound to this key press, if any. Shift is ignored.
    pub fn action(&self, key: &KeyEvent) -> Option<Action> {
        let pressed = Binding::from_event(key);
        self.bindings
            .iter()
            .find(|(b, _)| *b == pressed)
            .map(|(_, action)| *action)
    }

    /// All keys bound to `action`, joined with `/` (e.g. `j/down`). Empty if unbound.
    pub fn key_label(&self, action: Action) -> String {
        self.bindings
            .iter()
            .filter(|(_, a)| *a == action)
            .map(|(b, _)| b.label())
            .collect::<Vec<_>>()
            .join("/")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    #[test]
    fn parses_key_specs() {
        let b = |code, ctrl| Some(Binding { code, ctrl });
        assert_eq!(parse_key("j"), b(KeyCode::Char('j'), false));
        assert_eq!(parse_key("J"), b(KeyCode::Char('J'), false));
        assert_eq!(parse_key("+"), b(KeyCode::Char('+'), false));
        assert_eq!(parse_key("-"), b(KeyCode::Char('-'), false));
        assert_eq!(parse_key("ENTER"), b(KeyCode::Enter, false));
        assert_eq!(parse_key("space"), b(KeyCode::Char(' '), false));
        assert_eq!(parse_key("pgdown"), b(KeyCode::PageDown, false));
        assert_eq!(parse_key("ctrl-x"), b(KeyCode::Char('x'), true));
        assert_eq!(parse_key("ctrl-left"), b(KeyCode::Left, true));
        assert_eq!(parse_key(""), None);
        assert_eq!(parse_key("ctrl-"), None);
        assert_eq!(parse_key("jk"), None);
        assert_eq!(parse_key("bogus"), None);
    }

    #[test]
    fn every_default_parses() {
        assert_eq!(KeyMap::default().bindings.len(), DEFAULTS.len());
    }

    #[test]
    fn default_bindings() {
        let km = KeyMap::default();
        assert_eq!(
            km.action(&press(KeyCode::Char('1'))),
            Some(Action::ViewLibrary)
        );
        assert_eq!(km.action(&press(KeyCode::Char('j'))), Some(Action::Down));
        assert_eq!(km.action(&press(KeyCode::Down)), Some(Action::Down));
        assert_eq!(km.action(&press(KeyCode::Left)), Some(Action::SeekBack));
        assert_eq!(km.action(&press(KeyCode::Char('h'))), Some(Action::Left));
        assert_eq!(
            km.action(&press(KeyCode::Char(' '))),
            Some(Action::TogglePause)
        );
        assert_eq!(km.action(&ctrl(KeyCode::Char('d'))), Some(Action::PageDown));
        assert_eq!(km.action(&ctrl(KeyCode::Char('c'))), Some(Action::Quit));
        assert_eq!(km.action(&press(KeyCode::Char('d'))), Some(Action::Remove));
        assert_eq!(km.action(&press(KeyCode::Char('z'))), None);
        assert_eq!(km.action(&ctrl(KeyCode::Char('j'))), None);
    }

    #[test]
    fn shift_letters_ignore_shift_modifier() {
        let km = KeyMap::default();
        let shifted_j = KeyEvent::new(KeyCode::Char('J'), KeyModifiers::SHIFT);
        assert_eq!(km.action(&shifted_j), Some(Action::MoveDown));
        let plain_j = press(KeyCode::Char('j'));
        assert_eq!(km.action(&plain_j), Some(Action::Down));
    }

    #[test]
    fn overrides_replace_action_keys() {
        let mut warnings = Vec::new();
        let overrides = HashMap::from([("view_library".to_string(), "y".to_string())]);
        let km = KeyMap::from_config(&overrides, &mut warnings);
        assert!(warnings.is_empty());
        assert_eq!(
            km.action(&press(KeyCode::Char('y'))),
            Some(Action::ViewLibrary)
        );
        assert_eq!(km.action(&press(KeyCode::Char('1'))), None);
        assert_eq!(
            km.action(&press(KeyCode::Char('2'))),
            Some(Action::ViewFiles)
        );
    }

    #[test]
    fn bad_overrides_warn_and_keep_defaults() {
        let mut warnings = Vec::new();
        let overrides = HashMap::from([
            ("no_such_action".to_string(), "y".to_string()),
            ("down".to_string(), "nope-key".to_string()),
        ]);
        let km = KeyMap::from_config(&overrides, &mut warnings);
        assert_eq!(warnings.len(), 2);
        assert!(warnings.iter().any(|w| w.contains("no_such_action")));
        assert!(warnings.iter().any(|w| w.contains("nope-key")));
        assert_eq!(km.action(&press(KeyCode::Char('j'))), Some(Action::Down));
    }

    #[test]
    fn conflicting_override_warns_and_takes_key() {
        let mut warnings = Vec::new();
        let overrides = HashMap::from([("quit".to_string(), "j".to_string())]);
        let km = KeyMap::from_config(&overrides, &mut warnings);
        assert_eq!(warnings.len(), 1);
        assert_eq!(km.action(&press(KeyCode::Char('j'))), Some(Action::Quit));
    }

    #[test]
    fn key_labels() {
        let km = KeyMap::default();
        assert_eq!(km.key_label(Action::ViewLibrary), "1");
        assert_eq!(km.key_label(Action::Down), "j/down");
        assert_eq!(km.key_label(Action::TogglePause), "space");
        assert_eq!(km.key_label(Action::PageDown), "ctrl-d/pgdown");
        assert_eq!(km.key_label(Action::Quit), "q/ctrl-c");
    }

    #[test]
    fn action_names_deserialize() {
        assert_eq!(parse_action("view_history"), Some(Action::ViewHistory));
        assert_eq!(parse_action("clear_queue"), Some(Action::ClearQueue));
        assert_eq!(parse_action("ViewHistory"), None);
    }
}
