# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What is Zellaude

A Zellij WASM plugin that replaces the default tab bar with a Claude Code activity-aware status bar. It shows live activity indicators for Claude Code sessions across all Zellij tabs, with clickable tabs, permission flash alerts, desktop notifications, and elapsed time tracking.

## Build & Development

```bash
just build          # Build WASM plugin (cargo build --release --target wasm32-wasip1)
just lint           # Run clippy (cargo clippy --target wasm32-wasip1 -- -D warnings)
just install        # Build and copy to ~/.config/zellij/plugins/
just release <level> # Release via cargo-release (patch/minor/major)
```

The target is always `wasm32-wasip1`. There are no tests — the plugin is verified by running it inside Zellij. The rust toolchain is pinned to stable with the wasm target in `rust-toolchain.toml`.

## Architecture

Two-component system:

1. **WASM plugin** (`src/`) — runs inside Zellij, implements `ZellijPlugin` trait
2. **Hook script** (`scripts/zellaude-hook.sh`) — bash bridge that forwards Claude Code hook events via `zellij pipe`

Data flow: `Claude Code hook → zellaude-hook.sh → zellij pipe → plugin → render`

### Source modules

- `main.rs` — `ZellijPlugin` impl on `State`: event dispatch (`update`), pipe message handling (`pipe`), render delegation, session lifecycle
- `state.rs` — all data types: `State`, `SessionInfo`, `Activity` enum, `Settings`, `HookPayload`, click regions, view modes
- `event_handler.rs` — maps hook events (SessionStart, PreToolUse, PermissionRequest, Stop, etc.) to `Activity` variants, manages flash deadlines
- `render.rs` — builds the ANSI status bar string with powerline arrows, tab rendering, settings menu, flash animation
- `installer.rs` — auto-installs hook script and registers it in `~/.claude/settings.json` on first plugin load (version-tagged, idempotent)
- `tab_pane_map.rs` — maps terminal pane IDs to tab indices using Zellij's `PaneManifest`

### Key design decisions

- All state lives in WASM memory — no temp files or external state
- Multiple plugin instances (one per tab) sync via `pipe_message_to_plugin` inter-plugin messaging (`zellaude:sync`, `zellaude:request`, `zellaude:settings`)
- Sessions are keyed by `pane_id` (u32), not session ID
- Hook script is embedded at compile time via `include_str!` and written to disk by the installer
- Settings persisted to `~/.config/zellij/plugins/zellaude.json`
- Flash animation uses 250ms tick intervals; normal timer is 1s
- `set_selectable(false)` is called after permissions are granted (not in `load`) to keep the plugin visible during fullscreen
