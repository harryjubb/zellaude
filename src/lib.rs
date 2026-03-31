pub mod event_handler;
pub mod installer;
pub mod render;
pub mod state;
pub mod tab_pane_map;

#[cfg(test)]
pub mod test_helpers;

/// Stub for the zellij WASM host function when compiling for native targets (testing/benchmarks).
/// The real function is provided by the Zellij WASM runtime.
#[cfg(not(target_arch = "wasm32"))]
#[no_mangle]
pub extern "C" fn host_run_plugin_command() {
    // no-op: Zellij API calls are stubs during native testing
}

use state::{HookPayload, MenuAction, SessionInfo, Settings, State, ViewMode};
use std::collections::BTreeMap;
use zellij_tile::prelude::*;

const TIMER_INTERVAL: f64 = 1.0;
const FLASH_TICK: f64 = 0.25;

impl ZellijPlugin for State {
    fn load(&mut self, _configuration: BTreeMap<String, String>) {
        // Try early — may be a no-op before permissions are granted, but ensures
        // non-selectability during the brief init window (prevents fullscreen hiding the bar)
        set_selectable(false);
        request_permission(&[
            PermissionType::ReadApplicationState,
            PermissionType::ChangeApplicationState,
            PermissionType::RunCommands,
            PermissionType::ReadCliPipes,
            PermissionType::MessageAndLaunchOtherPlugins,
        ]);
        subscribe(&[
            EventType::TabUpdate,
            EventType::PaneUpdate,
            EventType::ModeUpdate,
            EventType::Timer,
            EventType::Mouse,
            EventType::RunCommandResult,
            EventType::PermissionRequestResult,
            EventType::Visible, // restart timer chain on resurrection / reconnect
        ]);
        set_timeout(TIMER_INTERVAL);

        // Load persisted settings (may be retried in PermissionRequestResult
        // if this fires before permissions are granted)
        self.load_config();
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::TabUpdate(tabs) => {
                let new_active = tabs.iter().find(|t| t.active).map(|t| t.position);
                if new_active != self.active_tab_index {
                    // Tab focus changed — clear persist flashes on the newly focused tab
                    if let Some(idx) = new_active {
                        self.clear_flashes_on_tab(idx);
                    }
                }
                self.active_tab_index = new_active;
                self.tabs = tabs;
                self.rebuild_pane_map();
                true
            }
            Event::PaneUpdate(manifest) => {
                self.pane_manifest = Some(manifest);
                self.rebuild_pane_map();
                true
            }
            Event::ModeUpdate(mode_info) => {
                self.input_mode = mode_info.mode;
                if let Some(name) = mode_info.session_name {
                    self.zellij_session_name = Some(name);
                }
                true
            }
            Event::Mouse(Mouse::LeftClick(_, col)) => {
                // Check prefix click region first → toggle ViewMode
                if let Some((start, end)) = self.prefix_click_region {
                    if col >= start && col < end {
                        self.view_mode = match self.view_mode {
                            ViewMode::Normal => ViewMode::Settings,
                            ViewMode::Settings => ViewMode::Normal,
                        };
                        return true;
                    }
                }

                match self.view_mode {
                    ViewMode::Normal => {
                        for region in &self.click_regions {
                            if col >= region.start_col && col < region.end_col {
                                if region.is_waiting {
                                    focus_terminal_pane(region.pane_id, false);
                                } else {
                                    switch_tab_to(region.tab_index as u32 + 1);
                                }
                                return false;
                            }
                        }
                        false
                    }
                    ViewMode::Settings => {
                        for region in &self.menu_click_regions {
                            if col >= region.start_col && col < region.end_col {
                                match &region.action {
                                    MenuAction::ToggleSetting(key) => {
                                        match key {
                                            state::SettingKey::Notifications => {
                                                self.settings.notifications =
                                                    self.settings.notifications.cycle();
                                            }
                                            state::SettingKey::Flash => {
                                                self.settings.flash =
                                                    self.settings.flash.cycle();
                                            }
                                            state::SettingKey::ElapsedTime => {
                                                self.settings.elapsed_time =
                                                    !self.settings.elapsed_time;
                                            }
                                        }
                                        self.save_config();
                                    }
                                    MenuAction::CloseMenu => {
                                        self.view_mode = ViewMode::Normal;
                                    }
                                }
                                return true;
                            }
                        }
                        false
                    }
                }
            }
            Event::RunCommandResult(exit_code, stdout, _stderr, context) => {
                match context.get("type").map(|s| s.as_str()) {
                    Some("load_config") if exit_code == Some(0) => {
                        let raw = String::from_utf8_lossy(&stdout);
                        if let Ok(settings) = serde_json::from_str::<Settings>(raw.trim()) {
                            self.settings = settings;
                        }
                        self.config_loaded = true;
                        true
                    }
                    Some("install_hooks") => {
                        self.hooks_installed = true;
                        false
                    }
                    _ => false,
                }
            }
            Event::Timer(_) => {
                let stale_changed = self.cleanup_stale_sessions();
                let flash_changed = self.cleanup_expired_flashes();
                let has_flashes = self.has_active_flashes();
                if has_flashes {
                    set_timeout(FLASH_TICK);
                } else {
                    set_timeout(TIMER_INTERVAL);
                }
                has_flashes || stale_changed || flash_changed || self.has_elapsed_display_changed()
            }
            Event::PermissionRequestResult(_) => {
                // Now that permissions are granted, mark as non-selectable
                // so the plugin stays visible during fullscreen
                set_selectable(false);
                // Ask existing instances for their state — but only if we have none.
                // On reattach Zellij re-fires PermissionRequestResult for all N running
                // instances; if they all request sync, every instance with sessions
                // responds to every request (O(N²) pipe messages). Skip when we already
                // have sessions to avoid this storm.
                if self.sessions.is_empty() {
                    self.request_sync();
                }
                // Retry config load (the one in load() may have been dropped
                // because it ran before permissions were granted)
                if !self.config_loaded {
                    self.load_config();
                }
                // Auto-install hook script and register Claude Code hooks
                if !self.hooks_installed {
                    installer::run_install();
                }
                false
            }
            Event::Visible(visible) => {
                if visible {
                    // Restart timer chain in case it died during session resurrection.
                    // Zellij may restore WASM state without calling load() again, which
                    // would leave the set_timeout chain dead and the bar frozen.
                    set_timeout(TIMER_INTERVAL);
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    fn pipe(&mut self, pipe_message: PipeMessage) -> bool {
        match pipe_message.name.as_str() {
            "zellaude" => {
                // Hook event from CLI
                let payload_str = match pipe_message.payload {
                    Some(ref s) => s,
                    None => return false,
                };
                let payload: HookPayload = match serde_json::from_str(payload_str) {
                    Ok(p) => p,
                    Err(_) => return false,
                };
                event_handler::handle_hook_event(self, payload)
            }
            "zellaude:focus" => {
                // Notification click — focus the requested pane
                if let Some(ref payload) = pipe_message.payload {
                    if let Ok(pane_id) = payload.trim().parse::<u32>() {
                        focus_terminal_pane(pane_id, false);
                    }
                }
                false
            }
            "zellaude:request" => {
                // Another instance asking for state — respond with ours (skip if empty to avoid N^3 storm on resume)
                if !self.sessions.is_empty() {
                    self.broadcast_sessions();
                }
                false
            }
            "zellaude:settings" => {
                // Another instance broadcast new settings
                if let Some(ref payload) = pipe_message.payload {
                    if let Ok(settings) = serde_json::from_str::<Settings>(payload) {
                        self.settings = settings;
                        return true;
                    }
                }
                false
            }
            "zellaude:sync" => {
                // Another instance sharing state — merge it
                if let Some(ref payload) = pipe_message.payload {
                    if let Ok(sessions) =
                        serde_json::from_str::<BTreeMap<u32, SessionInfo>>(payload)
                    {
                        self.merge_sessions(sessions);
                        return true;
                    }
                }
                false
            }
            _ => false,
        }
    }

    fn render(&mut self, rows: usize, cols: usize) {
        render::render_status_bar(self, rows, cols);
    }
}
