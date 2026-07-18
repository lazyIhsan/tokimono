use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ratatui::style::Color;
use serde::Deserialize;

const DEFAULT_TICK_MS: u64 = 250;
const MIN_TICK_MS: u64 = 50;

const STARTER_TEMPLATE: &str = include_str!("../assets/config.toml.example");

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawConfig {
    general: RawGeneral,
    theme: RawTheme,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawGeneral {
    refresh_rate_ms: Option<u64>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawTheme {
    background: Option<Color>,
    accent: Option<Color>,
    cpu_low: Option<Color>,
    cpu_mid: Option<Color>,
    cpu_high: Option<Color>,
    muted: Option<Color>,
    selection_bg: Option<Color>,
    selection_fg: Option<Color>,
}

/// Resolved color palette. Defaults to a dark theme that never paints an
/// opaque background, so a transparent/blurred terminal background (e.g.
/// kitty) shows through unless `background` is set explicitly.
#[derive(Clone, Copy)]
pub struct Theme {
    pub background: Color,
    pub accent: Color,
    pub cpu_low: Color,
    pub cpu_mid: Color,
    pub cpu_high: Color,
    pub muted: Color,
    pub selection_bg: Color,
    pub selection_fg: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            background: Color::Reset,
            accent: Color::Cyan,
            cpu_low: Color::Green,
            cpu_mid: Color::Yellow,
            cpu_high: Color::Red,
            muted: Color::DarkGray,
            selection_bg: Color::Blue,
            selection_fg: Color::White,
        }
    }
}

pub struct Config {
    pub tick_rate: Duration,
    pub theme: Theme,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            tick_rate: Duration::from_millis(DEFAULT_TICK_MS),
            theme: Theme::default(),
        }
    }
}

fn config_path() -> PathBuf {
    match std::env::var("XDG_CONFIG_HOME") {
        Ok(dir) if !dir.is_empty() => PathBuf::from(dir).join("tokimono/config.toml"),
        _ => {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".config/tokimono/config.toml")
        }
    }
}

/// Loads `~/.config/tokimono/config.toml` (or `$XDG_CONFIG_HOME/tokimono/config.toml`),
/// falling back to defaults for anything missing or unparsable. Drops a
/// commented starter file on first run so there's something to edit.
pub fn load() -> Config {
    let path = config_path();
    ensure_starter_file(&path);

    let raw = match fs::read_to_string(&path) {
        Ok(contents) => toml::from_str::<RawConfig>(&contents).unwrap_or_else(|err| {
            eprintln!("tokimono: ignoring {}: {err}", path.display());
            RawConfig::default()
        }),
        Err(_) => RawConfig::default(),
    };

    let defaults = Theme::default();
    Config {
        tick_rate: raw
            .general
            .refresh_rate_ms
            .map(|ms| Duration::from_millis(ms.max(MIN_TICK_MS)))
            .unwrap_or(Duration::from_millis(DEFAULT_TICK_MS)),
        theme: Theme {
            background: raw.theme.background.unwrap_or(defaults.background),
            accent: raw.theme.accent.unwrap_or(defaults.accent),
            cpu_low: raw.theme.cpu_low.unwrap_or(defaults.cpu_low),
            cpu_mid: raw.theme.cpu_mid.unwrap_or(defaults.cpu_mid),
            cpu_high: raw.theme.cpu_high.unwrap_or(defaults.cpu_high),
            muted: raw.theme.muted.unwrap_or(defaults.muted),
            selection_bg: raw.theme.selection_bg.unwrap_or(defaults.selection_bg),
            selection_fg: raw.theme.selection_fg.unwrap_or(defaults.selection_fg),
        },
    }
}

fn ensure_starter_file(path: &Path) {
    if path.exists() {
        return;
    }
    if let Some(parent) = path.parent()
        && fs::create_dir_all(parent).is_err()
    {
        return;
    }
    let _ = fs::write(path, STARTER_TEMPLATE);
}
