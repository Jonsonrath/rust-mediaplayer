//! Config loading (TOML), XDG paths, theme overrides and keybindings.
//! Invalid input never aborts: it falls back to defaults and yields a warning.
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::ui::theme::{Theme, parse_hex};

pub mod keys;

const APP_DIR: &str = "rmp";
const VIEW_NAMES: [&str; 5] = ["library", "files", "playlists", "queue", "history"];
const VOLUME_MAX: i64 = 130;
const TOP_KEYS: [&str; 6] = [
    "media_dirs",
    "volume",
    "start_view",
    "rescan_on_start",
    "theme",
    "keys",
];
const THEME_KEYS: [&str; 12] = [
    "bg",
    "surface",
    "raised",
    "text",
    "text_secondary",
    "text_muted",
    "accent",
    "accent_alt",
    "playing",
    "warning",
    "error",
    "border",
];

/// `$XDG_CONFIG_HOME/rmp/config.toml`, falling back to `~/.config`.
pub fn config_path() -> PathBuf {
    config_path_in(env_path("XDG_CONFIG_HOME"), home_dir().as_deref())
}

pub fn config_path_in(xdg: Option<PathBuf>, home: Option<&Path>) -> PathBuf {
    xdg_base(xdg, home, ".config")
        .join(APP_DIR)
        .join("config.toml")
}

/// `$XDG_DATA_HOME/rmp`, falling back to `~/.local/share/rmp`.
pub fn data_dir() -> PathBuf {
    data_dir_in(env_path("XDG_DATA_HOME"), home_dir().as_deref())
}

pub fn data_dir_in(xdg: Option<PathBuf>, home: Option<&Path>) -> PathBuf {
    xdg_base(xdg, home, ".local/share").join(APP_DIR)
}

pub fn db_path() -> PathBuf {
    data_dir().join("library.db")
}

/// Relative XDG values are ignored, as the XDG spec requires.
fn env_path(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}

fn home_dir() -> Option<PathBuf> {
    env_path("HOME")
}

fn xdg_base(xdg: Option<PathBuf>, home: Option<&Path>, fallback: &str) -> PathBuf {
    xdg.or_else(|| home.map(|h| h.join(fallback)))
        .unwrap_or_else(|| PathBuf::from(fallback))
}

/// `~/Music` and `~/Videos`, keeping only those that exist.
fn default_media_dirs(home: Option<&Path>) -> Vec<PathBuf> {
    let Some(home) = home else {
        return Vec::new();
    };
    ["Music", "Videos"]
        .iter()
        .map(|dir| home.join(dir))
        .filter(|p| p.is_dir())
        .collect()
}

/// Replace a leading `~` path component with `home`.
fn expand_tilde(path: &Path, home: Option<&Path>) -> PathBuf {
    match (path.strip_prefix("~"), home) {
        (Ok(rest), Some(home)) => home.join(rest),
        _ => path.to_path_buf(),
    }
}

fn view_index(name: &str) -> Option<usize> {
    VIEW_NAMES.iter().position(|v| v.eq_ignore_ascii_case(name))
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Directories indexed by the library. A leading `~` is expanded.
    pub media_dirs: Vec<PathBuf>,
    /// Startup volume, clamped to 0..=130.
    pub volume: i64,
    /// One of `library`, `files`, `playlists`, `queue`, `history`.
    pub start_view: String,
    pub rescan_on_start: bool,
    pub theme: ThemeConfig,
    /// Action name to key spec, e.g. `view_library = "1"`. See `KeyMap`.
    pub keys: HashMap<String, String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            media_dirs: default_media_dirs(home_dir().as_deref()),
            volume: 80,
            start_view: "library".to_string(),
            rescan_on_start: true,
            theme: ThemeConfig::default(),
            keys: HashMap::new(),
        }
    }
}

/// Optional `#RRGGBB` overrides for the theme tokens. Unset tokens keep the default.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ThemeConfig {
    pub bg: Option<String>,
    pub surface: Option<String>,
    pub raised: Option<String>,
    pub text: Option<String>,
    pub text_secondary: Option<String>,
    pub text_muted: Option<String>,
    pub accent: Option<String>,
    pub accent_alt: Option<String>,
    pub playing: Option<String>,
    pub warning: Option<String>,
    pub error: Option<String>,
    pub border: Option<String>,
}

impl Config {
    /// Load the config file. A missing file gives defaults with no warning. Any other
    /// problem gives defaults plus a warning naming the file.
    pub fn load() -> (Config, Vec<String>) {
        load_from(&config_path(), home_dir().as_deref())
    }

    fn parse(text: &str, home: Option<&Path>) -> Result<(Config, Vec<String>), toml::de::Error> {
        let table: toml::Table = toml::from_str(text)?;
        let mut warnings = unknown_key_warnings(&table);
        let mut cfg: Config = table.try_into()?;
        cfg.media_dirs = cfg
            .media_dirs
            .iter()
            .map(|d| expand_tilde(d, home))
            .collect();
        cfg.volume = cfg.volume.clamp(0, VOLUME_MAX);
        if view_index(&cfg.start_view).is_none() {
            warnings.push(format!(
                "start_view \"{}\" is not a view; using library",
                cfg.start_view
            ));
            cfg.start_view = "library".to_string();
        }
        Ok((cfg, warnings))
    }

    /// Index into `App::VIEW_NAMES` for the configured start view.
    pub fn start_view_index(&self) -> usize {
        view_index(&self.start_view).unwrap_or(0)
    }

    /// Defaults with valid `[theme]` hex overrides applied. Invalid values add a warning.
    pub fn theme(&self, warnings: &mut Vec<String>) -> Theme {
        let mut theme = Theme::default();
        let slots = [
            ("bg", &self.theme.bg, &mut theme.bg),
            ("surface", &self.theme.surface, &mut theme.surface),
            ("raised", &self.theme.raised, &mut theme.raised),
            ("text", &self.theme.text, &mut theme.text),
            (
                "text_secondary",
                &self.theme.text_secondary,
                &mut theme.text_secondary,
            ),
            ("text_muted", &self.theme.text_muted, &mut theme.text_muted),
            ("accent", &self.theme.accent, &mut theme.accent),
            ("accent_alt", &self.theme.accent_alt, &mut theme.accent_alt),
            ("playing", &self.theme.playing, &mut theme.playing),
            ("warning", &self.theme.warning, &mut theme.warning),
            ("error", &self.theme.error, &mut theme.error),
            ("border", &self.theme.border, &mut theme.border),
        ];
        for (name, value, slot) in slots {
            let Some(value) = value else {
                continue;
            };
            match parse_hex(value) {
                Some(color) => *slot = color,
                None => warnings.push(format!(
                    "theme.{name}: invalid hex color \"{value}\"; using default"
                )),
            }
        }
        theme
    }
}

/// Serde silently ignores unknown keys; report typos instead.
fn unknown_key_warnings(table: &toml::Table) -> Vec<String> {
    let mut w = Vec::new();
    for k in table.keys().filter(|k| !TOP_KEYS.contains(&k.as_str())) {
        w.push(format!("config: unknown key \"{k}\" ignored"));
    }
    if let Some(theme) = table.get("theme").and_then(|t| t.as_table()) {
        for k in theme.keys().filter(|k| !THEME_KEYS.contains(&k.as_str())) {
            w.push(format!("config: unknown key \"theme.{k}\" ignored"));
        }
    }
    w
}

fn load_from(path: &Path, home: Option<&Path>) -> (Config, Vec<String>) {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return (Config::default(), Vec::new()),
        Err(e) => {
            return (
                Config::default(),
                vec![format!("config {}: {e}; using defaults", path.display())],
            );
        }
    };
    match Config::parse(&text, home) {
        Ok((cfg, warnings)) => (cfg, warnings),
        Err(e) => (
            Config::default(),
            vec![format!("config {}: {e}; using defaults", path.display())],
        ),
    }
}

#[cfg(test)]
mod tests {
    use ratatui::style::Color;

    use super::*;

    #[test]
    fn defaults() {
        let cfg = Config::default();
        assert_eq!(cfg.volume, 80);
        assert_eq!(cfg.start_view_index(), 0);
        assert!(cfg.rescan_on_start);
        assert!(cfg.keys.is_empty());
    }

    #[test]
    fn default_media_dirs_keep_existing_only() {
        let home = tempfile::tempdir().unwrap();
        assert!(default_media_dirs(Some(home.path())).is_empty());
        fs::create_dir(home.path().join("Videos")).unwrap();
        assert_eq!(
            default_media_dirs(Some(home.path())),
            vec![home.path().join("Videos")]
        );
        assert!(default_media_dirs(None).is_empty());
    }

    #[test]
    fn empty_text_gives_defaults() {
        let (cfg, warnings) = Config::parse("", None).unwrap();
        assert_eq!(cfg.volume, 80);
        assert!(warnings.is_empty());
    }

    #[test]
    fn invalid_toml_warns_and_uses_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "volume = = 3").unwrap();
        let (cfg, warnings) = load_from(&path, None);
        assert_eq!(cfg.volume, 80);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains(&path.display().to_string()));
    }

    #[test]
    fn missing_file_is_silent() {
        let dir = tempfile::tempdir().unwrap();
        let (cfg, warnings) = load_from(&dir.path().join("nope.toml"), None);
        assert_eq!(cfg.start_view, "library");
        assert!(warnings.is_empty());
    }

    #[test]
    fn values_are_normalized() {
        let (cfg, warnings) = Config::parse("volume = 500\nstart_view = \"Queue\"", None).unwrap();
        assert_eq!(cfg.volume, 130);
        assert_eq!(cfg.start_view_index(), 3);
        assert!(warnings.is_empty());

        let (cfg, _) = Config::parse("volume = -5", None).unwrap();
        assert_eq!(cfg.volume, 0);
    }

    #[test]
    fn bad_start_view_warns() {
        let (cfg, warnings) = Config::parse("start_view = \"nowhere\"", None).unwrap();
        assert_eq!(cfg.start_view_index(), 0);
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn tilde_expands_in_media_dirs() {
        let home = Path::new("/home/someone");
        let (cfg, _) =
            Config::parse("media_dirs = [\"~/Music\", \"/srv/media\"]", Some(home)).unwrap();
        assert_eq!(
            cfg.media_dirs,
            vec![
                PathBuf::from("/home/someone/Music"),
                PathBuf::from("/srv/media")
            ]
        );
        assert_eq!(
            expand_tilde(Path::new("~user/x"), Some(home)),
            PathBuf::from("~user/x")
        );
    }

    #[test]
    fn theme_overrides_and_invalid_hex() {
        let (cfg, _) = Config::parse(
            "[theme]\nbg = \"#000000\"\naccent = \"nope\"\nerror = \"#FF0000\"",
            None,
        )
        .unwrap();
        let mut warnings = Vec::new();
        let theme = cfg.theme(&mut warnings);
        assert_eq!(theme.bg, Color::Rgb(0, 0, 0));
        assert_eq!(theme.error, Color::Rgb(0xFF, 0, 0));
        assert_eq!(theme.accent, Theme::default().accent);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("accent"));
    }

    #[test]
    fn unknown_keys_warn() {
        let (_, w) = Config::parse("volum = 3\n[theme]\nbgg = \"#000000\"", None).unwrap();
        assert_eq!(w.len(), 2);
        assert!(w[0].contains("volum") && w[1].contains("theme.bgg"));
    }

    #[test]
    fn wrong_type_is_an_error_not_a_panic() {
        assert!(Config::parse("volume = \"loud\"", None).is_err());
        assert!(Config::parse("theme = 3", None).is_err());
    }

    #[test]
    fn keys_table_parses() {
        let (cfg, _) = Config::parse("[keys]\nquit = \"Q\"", None).unwrap();
        assert_eq!(cfg.keys.get("quit").map(String::as_str), Some("Q"));
    }

    #[test]
    fn example_config_is_valid() {
        let (cfg, warnings) = Config::parse(include_str!("../config.example.toml"), None).unwrap();
        assert!(warnings.is_empty());
        let mut theme_warnings = Vec::new();
        cfg.theme(&mut theme_warnings);
        assert!(theme_warnings.is_empty());
    }

    #[test]
    fn paths_use_xdg_then_home() {
        let home = Path::new("/home/someone");
        assert_eq!(
            config_path_in(None, Some(home)),
            PathBuf::from("/home/someone/.config/rmp/config.toml")
        );
        assert_eq!(
            config_path_in(Some(PathBuf::from("/xdg/cfg")), Some(home)),
            PathBuf::from("/xdg/cfg/rmp/config.toml")
        );
        assert_eq!(
            data_dir_in(None, Some(home)),
            PathBuf::from("/home/someone/.local/share/rmp")
        );
        assert_eq!(
            data_dir_in(Some(PathBuf::from("/xdg/data")), Some(home)),
            PathBuf::from("/xdg/data/rmp")
        );
        assert_eq!(data_dir_in(None, None), PathBuf::from(".local/share/rmp"));
    }
}
