# Tokimono

> A fast, modern, Tokio-powered terminal system monitor for Linux.

[![Crates.io](https://img.shields.io/crates/v/tokimono)](https://crates.io/crates/tokimono)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)
[![Rust](https://img.shields.io/badge/rust-1.75+-orange.svg)](https://www.rust-lang.org)

**Tokimono** brings real-time system monitoring to your terminal with a clean, responsive TUI built on top of Tokio and Ratatui. Inspired by the best of `htop`, `btop`, and `glances`, it focuses on clarity, performance, and delightful keyboard-driven interaction.

---

## ✨ Features

- **Real-time metrics** — CPU (per-core + aggregate), memory, swap, load averages
- **Process management** — Sortable, filterable process table with kill/renice support
- **Network & Disk I/O** — Live throughput, errors, and usage
- **Beautiful TUI** — Built with Ratatui for smooth rendering and modern terminal styling
- **Async & efficient** — Powered by Tokio for responsive, low-overhead monitoring
- **Keyboard-first** — Vim-like navigation + intuitive shortcuts
- **Configurable** — Themes, refresh rate, shown metrics, and more
- **Cross-platform ready** — Focused on Linux (easy to extend)

---

## 📦 Installation

### From crates.io (recommended)

```bash
cargo install tokimono
```

---

## ⚙️ Configuration

Tokimono reads `~/.config/tokimono/config.toml` (or `$XDG_CONFIG_HOME/tokimono/config.toml`), writing a commented starter file there the first time it runs. It controls the refresh rate and color theme:

```toml
[general]
refresh_rate_ms = 250

[theme]
background = "reset"   # "reset" preserves terminal transparency (e.g. kitty)
accent = "cyan"
cpu_low = "green"
cpu_mid = "yellow"
cpu_high = "red"
muted = "darkgray"
selection_bg = "blue"
selection_fg = "white"
```

Colors accept named colors, hex values (`"#1a1b26"`), or `"reset"`. `background` defaults to `"reset"`, so a transparent or blurred terminal background shows through tokimono's panels instead of being painted over — set it explicitly if you want a solid panel instead.
