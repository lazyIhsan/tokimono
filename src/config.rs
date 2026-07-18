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
    preset: Option<String>,
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

fn hex(rgb: u32) -> Color {
    Color::Rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// Named built-in palettes, layered under any explicit `[theme]` overrides.
/// `background` is always `Color::Reset` here too — a preset picks the
/// accent/status colors, not whether tokimono paints an opaque panel.
fn preset_theme(name: &str) -> Option<Theme> {
    match name.to_lowercase().as_str() {
        "catppuccin" | "catppuccin-mocha" => Some(Theme {
            background: Color::Reset,
            accent: hex(0xcba6f7),       // Mauve
            cpu_low: hex(0xa6e3a1),      // Green
            cpu_mid: hex(0xf9e2af),      // Yellow
            cpu_high: hex(0xf38ba8),     // Red
            muted: hex(0x6c7086),        // Overlay0
            selection_bg: hex(0x585b70), // Surface2
            selection_fg: hex(0xcdd6f4), // Text
        }),
        "catppuccin-macchiato" => Some(Theme {
            background: Color::Reset,
            accent: hex(0xc6a0f6),
            cpu_low: hex(0xa6da95),
            cpu_mid: hex(0xeed49f),
            cpu_high: hex(0xed8796),
            muted: hex(0x6e738d),
            selection_bg: hex(0x494d64),
            selection_fg: hex(0xcad3f5),
        }),
        "catppuccin-frappe" | "catppuccin-frappé" => Some(Theme {
            background: Color::Reset,
            accent: hex(0xca9ee6),
            cpu_low: hex(0xa6d189),
            cpu_mid: hex(0xe5c890),
            cpu_high: hex(0xe78284),
            muted: hex(0x737994),
            selection_bg: hex(0x51576d),
            selection_fg: hex(0xc6d0f5),
        }),
        "catppuccin-latte" => Some(Theme {
            background: Color::Reset,
            accent: hex(0x8839ef),
            cpu_low: hex(0x40a02b),
            cpu_mid: hex(0xdf8e1d),
            cpu_high: hex(0xd20f39),
            muted: hex(0x9ca0b0),
            selection_bg: hex(0xbcc0cc),
            selection_fg: hex(0x4c4f69),
        }),
        "gruvbox" | "gruvbox-dark" => Some(Theme {
            background: Color::Reset,
            accent: hex(0xfe8019),       // bright orange
            cpu_low: hex(0xb8bb26),      // bright green
            cpu_mid: hex(0xfabd2f),      // bright yellow
            cpu_high: hex(0xfb4934),     // bright red
            muted: hex(0x928374),        // gray
            selection_bg: hex(0x504945), // bg2
            selection_fg: hex(0xebdbb2), // fg1
        }),
        "gruvbox-light" => Some(Theme {
            background: Color::Reset,
            accent: hex(0xaf3a03),       // faded orange
            cpu_low: hex(0x79740e),      // faded green
            cpu_mid: hex(0xb57614),      // faded yellow
            cpu_high: hex(0x9d0006),     // faded red
            muted: hex(0x928374),        // gray
            selection_bg: hex(0xd5c4a1), // bg2
            selection_fg: hex(0x3c3836), // fg1
        }),
        _ => None,
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

    let defaults = match raw.theme.preset.as_deref() {
        Some(name) => preset_theme(name).unwrap_or_else(|| {
            eprintln!("tokimono: unknown theme preset \"{name}\", using default");
            Theme::default()
        }),
        None => Theme::default(),
    };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_presets_resolve() {
        for name in [
            "catppuccin-mocha",
            "catppuccin-macchiato",
            "catppuccin-frappe",
            "catppuccin-latte",
            "gruvbox-dark",
            "gruvbox-light",
        ] {
            assert!(preset_theme(name).is_some(), "{name} should resolve");
        }
    }

    #[test]
    fn bare_family_names_alias_to_a_default_flavor() {
        assert!(preset_theme("catppuccin").is_some());
        assert!(preset_theme("gruvbox").is_some());
    }

    #[test]
    fn preset_lookup_is_case_insensitive() {
        assert!(preset_theme("CATPPUCCIN-MOCHA").is_some());
        assert!(preset_theme("Gruvbox-Dark").is_some());
    }

    #[test]
    fn unknown_preset_returns_none() {
        assert!(preset_theme("not-a-real-theme").is_none());
    }

    #[test]
    fn presets_never_override_the_transparent_background_default() {
        for name in ["catppuccin-mocha", "gruvbox-dark", "gruvbox-light"] {
            let theme = preset_theme(name).unwrap();
            assert_eq!(theme.background, Color::Reset);
        }
    }

    #[test]
    fn catppuccin_mocha_uses_documented_accent_color() {
        let theme = preset_theme("catppuccin-mocha").unwrap();
        assert_eq!(theme.accent, hex(0xcba6f7));
    }
}
