use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::time::{SystemTime, UNIX_EPOCH};
use zellij_tile::prelude::*;

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub const FLASH_DURATION_MS: u64 = 2000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Activity {
    Init,
    Thinking,
    Tool(String),
    Prompting,
    Waiting,
    Notification,
    Done,
    AgentDone,
    Idle,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub session_id: String,
    pub pane_id: u32,
    pub activity: Activity,
    pub tab_name: Option<String>,
    pub tab_index: Option<usize>,
    pub last_event_ts: u64,
    pub cwd: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct HookPayload {
    pub session_id: Option<String>,
    pub pane_id: u32,
    pub hook_event: String,
    pub tool_name: Option<String>,
    pub cwd: Option<String>,
    pub zellij_session: Option<String>,
    pub term_program: Option<String>,
}

pub struct ClickRegion {
    pub start_col: usize,
    pub end_col: usize,
    pub tab_index: usize,
    pub pane_id: u32,
    pub is_waiting: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub enum NotifyMode {
    Never,
    Unfocused,
    #[default]
    Always,
}

impl NotifyMode {
    pub fn cycle(self) -> Self {
        match self {
            Self::Always => Self::Unfocused,
            Self::Unfocused => Self::Never,
            Self::Never => Self::Always,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub enum FlashMode {
    Off,
    #[default]
    Once,
    Persist,
}

impl FlashMode {
    pub fn cycle(self) -> Self {
        match self {
            Self::Once => Self::Persist,
            Self::Persist => Self::Off,
            Self::Off => Self::Once,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub notifications: NotifyMode,
    pub flash: FlashMode,
    pub elapsed_time: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            notifications: NotifyMode::Always,
            flash: FlashMode::Once,
            elapsed_time: true,
        }
    }
}

#[derive(Default, PartialEq)]
pub enum ViewMode {
    #[default]
    Normal,
    Settings,
}

#[derive(Clone, Copy)]
pub enum SettingKey {
    Notifications,
    Flash,
    ElapsedTime,
}

pub enum MenuAction {
    ToggleSetting(SettingKey),
    CloseMenu,
}

pub struct MenuClickRegion {
    pub start_col: usize,
    pub end_col: usize,
    pub action: MenuAction,
}

pub const DONE_TIMEOUT: u64 = 30;

#[derive(Default)]
pub struct State {
    pub sessions: BTreeMap<u32, SessionInfo>,
    pub pane_to_tab: HashMap<u32, (usize, String)>,
    pub tabs: Vec<TabInfo>,
    pub pane_manifest: Option<PaneManifest>,
    pub active_tab_index: Option<usize>,
    pub click_regions: Vec<ClickRegion>,
    /// pane_id -> flash deadline in ms (for waiting animation)
    pub flash_deadlines: HashMap<u32, u64>,
    pub zellij_session_name: Option<String>,
    pub term_program: Option<String>,
    pub input_mode: InputMode,
    pub settings: Settings,
    pub view_mode: ViewMode,
    pub prefix_click_region: Option<(usize, usize)>,
    pub menu_click_regions: Vec<MenuClickRegion>,
    pub config_loaded: bool,
    pub hooks_installed: bool,
    /// Cached render output — skip I/O when unchanged
    pub last_rendered: String,
}

impl State {
    pub fn rebuild_pane_map(&mut self) {
        if let Some(ref manifest) = self.pane_manifest {
            self.pane_to_tab = crate::tab_pane_map::build_pane_to_tab_map(&self.tabs, manifest);
            self.refresh_session_tab_names();
            self.remove_dead_panes();
        }
    }

    pub fn refresh_session_tab_names(&mut self) {
        for session in self.sessions.values_mut() {
            if let Some((idx, name)) = self.pane_to_tab.get(&session.pane_id) {
                session.tab_index = Some(*idx);
                session.tab_name = Some(name.clone());
            }
        }
    }

    pub fn remove_dead_panes(&mut self) {
        self.sessions
            .retain(|pane_id, _| self.pane_to_tab.contains_key(pane_id));
    }

    pub fn cleanup_stale_sessions(&mut self) -> bool {
        let now = unix_now();
        let mut changed = false;
        for session in self.sessions.values_mut() {
            match session.activity {
                Activity::Done | Activity::AgentDone => {
                    if now.saturating_sub(session.last_event_ts) >= DONE_TIMEOUT {
                        session.activity = Activity::Idle;
                        changed = true;
                    }
                }
                _ => {}
            }
        }
        changed
    }

    pub fn clear_flashes_on_tab(&mut self, tab_idx: usize) {
        let pane_ids: Vec<u32> = self
            .sessions
            .values()
            .filter(|s| s.tab_index == Some(tab_idx))
            .map(|s| s.pane_id)
            .collect();
        for pane_id in pane_ids {
            self.flash_deadlines.remove(&pane_id);
        }
    }

    pub fn has_active_flashes(&self) -> bool {
        let now = unix_now_ms();
        self.flash_deadlines.values().any(|&deadline| now < deadline)
    }

    pub fn cleanup_expired_flashes(&mut self) -> bool {
        let before = self.flash_deadlines.len();
        let now = unix_now_ms();
        self.flash_deadlines.retain(|_, deadline| now < *deadline);
        self.flash_deadlines.len() != before
    }

    pub fn has_elapsed_display(&self) -> bool {
        if !self.settings.elapsed_time {
            return false;
        }
        let now = unix_now();
        self.sessions.values().any(|s| {
            !matches!(s.activity, Activity::Idle)
                && now.saturating_sub(s.last_event_ts) >= DONE_TIMEOUT
        })
    }

    pub fn merge_sessions(&mut self, incoming: BTreeMap<u32, SessionInfo>) {
        for (pane_id, mut session) in incoming {
            let dominated = self
                .sessions
                .get(&pane_id)
                .map(|existing| session.last_event_ts > existing.last_event_ts)
                .unwrap_or(true);
            if dominated {
                if let Some((idx, name)) = self.pane_to_tab.get(&pane_id) {
                    session.tab_index = Some(*idx);
                    session.tab_name = Some(name.clone());
                }
                self.sessions.insert(pane_id, session);
            }
        }
    }

    pub fn request_sync(&self) {
        pipe_message_to_plugin(MessageToPlugin::new("zellaude:request"));
    }

    pub fn broadcast_sessions(&self) {
        let mut msg = MessageToPlugin::new("zellaude:sync");
        msg.message_payload =
            Some(serde_json::to_string(&self.sessions).unwrap_or_default());
        pipe_message_to_plugin(msg);
    }

    pub fn broadcast_settings(&self) {
        let mut msg = MessageToPlugin::new("zellaude:settings");
        msg.message_payload =
            Some(serde_json::to_string(&self.settings).unwrap_or_default());
        pipe_message_to_plugin(msg);
    }

    pub fn load_config(&self) {
        let mut ctx = BTreeMap::new();
        ctx.insert("type".into(), "load_config".into());
        run_command(
            &[
                "sh",
                "-c",
                "cat \"$HOME/.config/zellij/plugins/zellaude.json\" 2>/dev/null || echo '{}'",
            ],
            ctx,
        );
    }

    pub fn save_config(&self) {
        if !self.config_loaded {
            return;
        }
        self.broadcast_settings();
        let json = serde_json::to_string(&self.settings).unwrap_or_default();
        let json_esc = json.replace('\'', "'\\''");
        let cmd = format!(
            "mkdir -p \"$HOME/.config/zellij/plugins\" && printf '%s' '{json_esc}' > \"$HOME/.config/zellij/plugins/zellaude.json\""
        );
        let mut ctx = BTreeMap::new();
        ctx.insert("type".into(), "save_config".into());
        run_command(&["sh", "-c", &cmd], ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_now_returns_nonzero() {
        let now = unix_now();
        assert!(now > 1_700_000_000, "timestamp should be recent: {now}");
    }

    #[test]
    fn unix_now_ms_is_milliseconds() {
        let secs = unix_now();
        let ms = unix_now_ms();
        // ms should be roughly secs * 1000 (within 2 seconds tolerance)
        assert!(ms >= secs * 1000);
        assert!(ms < (secs + 2) * 1000);
    }

    #[test]
    fn notify_mode_cycles_completely() {
        let start = NotifyMode::Always;
        let step1 = start.cycle();
        assert_eq!(step1, NotifyMode::Unfocused);
        let step2 = step1.cycle();
        assert_eq!(step2, NotifyMode::Never);
        let step3 = step2.cycle();
        assert_eq!(step3, NotifyMode::Always);
    }

    #[test]
    fn flash_mode_cycles_completely() {
        let start = FlashMode::Once;
        let step1 = start.cycle();
        assert_eq!(step1, FlashMode::Persist);
        let step2 = step1.cycle();
        assert_eq!(step2, FlashMode::Off);
        let step3 = step2.cycle();
        assert_eq!(step3, FlashMode::Once);
    }

    #[test]
    fn settings_default_values() {
        let s = Settings::default();
        assert_eq!(s.notifications, NotifyMode::Always);
        assert_eq!(s.flash, FlashMode::Once);
        assert!(s.elapsed_time);
    }

    #[test]
    fn settings_serde_roundtrip() {
        let original = Settings {
            notifications: NotifyMode::Unfocused,
            flash: FlashMode::Persist,
            elapsed_time: false,
        };
        let json = serde_json::to_string(&original).unwrap();
        let deserialized: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.notifications, NotifyMode::Unfocused);
        assert_eq!(deserialized.flash, FlashMode::Persist);
        assert!(!deserialized.elapsed_time);
    }

    #[test]
    fn settings_deserializes_with_defaults() {
        let json = "{}";
        let s: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(s.notifications, NotifyMode::Always);
        assert_eq!(s.flash, FlashMode::Once);
        assert!(s.elapsed_time);
    }

    #[test]
    fn hook_payload_deserialization() {
        let json = r#"{
            "session_id": "abc123",
            "pane_id": 42,
            "hook_event": "PreToolUse",
            "tool_name": "Bash",
            "cwd": "/home/user",
            "zellij_session": "my-session",
            "term_program": "xterm"
        }"#;
        let payload: HookPayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.session_id.as_deref(), Some("abc123"));
        assert_eq!(payload.pane_id, 42);
        assert_eq!(payload.hook_event, "PreToolUse");
        assert_eq!(payload.tool_name.as_deref(), Some("Bash"));
        assert_eq!(payload.cwd.as_deref(), Some("/home/user"));
    }

    #[test]
    fn hook_payload_minimal_json() {
        let json = r#"{"pane_id": 1, "hook_event": "Stop"}"#;
        let payload: HookPayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.pane_id, 1);
        assert_eq!(payload.hook_event, "Stop");
        assert!(payload.session_id.is_none());
        assert!(payload.tool_name.is_none());
    }

    #[test]
    fn activity_serde_roundtrip_all_variants() {
        let variants = vec![
            Activity::Init,
            Activity::Thinking,
            Activity::Tool("Bash".to_string()),
            Activity::Tool("Read".to_string()),
            Activity::Prompting,
            Activity::Waiting,
            Activity::Notification,
            Activity::Done,
            Activity::AgentDone,
            Activity::Idle,
        ];
        for activity in variants {
            let json = serde_json::to_string(&activity).unwrap();
            let deserialized: Activity = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, activity, "roundtrip failed for {json}");
        }
    }

    #[test]
    fn session_info_serde_roundtrip() {
        let session = SessionInfo {
            session_id: "test".to_string(),
            pane_id: 5,
            activity: Activity::Tool("Edit".to_string()),
            tab_name: Some("dev".to_string()),
            tab_index: Some(0),
            last_event_ts: 1700000000,
            cwd: Some("/tmp".to_string()),
        };
        let json = serde_json::to_string(&session).unwrap();
        let deserialized: SessionInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.session_id, "test");
        assert_eq!(deserialized.pane_id, 5);
        assert_eq!(deserialized.activity, Activity::Tool("Edit".to_string()));
        assert_eq!(deserialized.tab_name.as_deref(), Some("dev"));
    }

    // --- State method tests ---

    fn make_session(pane_id: u32, activity: Activity, ts: u64) -> SessionInfo {
        SessionInfo {
            session_id: format!("s{pane_id}"),
            pane_id,
            activity,
            tab_name: None,
            tab_index: None,
            last_event_ts: ts,
            cwd: None,
        }
    }

    fn make_session_with_tab(
        pane_id: u32,
        activity: Activity,
        ts: u64,
        tab_idx: usize,
    ) -> SessionInfo {
        SessionInfo {
            tab_index: Some(tab_idx),
            tab_name: Some(format!("tab{tab_idx}")),
            ..make_session(pane_id, activity, ts)
        }
    }

    #[test]
    fn merge_sessions_newer_timestamp_wins() {
        let mut state = State::default();
        state
            .sessions
            .insert(1, make_session(1, Activity::Init, 100));

        let mut incoming = BTreeMap::new();
        incoming.insert(1, make_session(1, Activity::Thinking, 200));
        state.merge_sessions(incoming);

        assert_eq!(state.sessions[&1].activity, Activity::Thinking);
        assert_eq!(state.sessions[&1].last_event_ts, 200);
    }

    #[test]
    fn merge_sessions_older_timestamp_ignored() {
        let mut state = State::default();
        state
            .sessions
            .insert(1, make_session(1, Activity::Thinking, 200));

        let mut incoming = BTreeMap::new();
        incoming.insert(1, make_session(1, Activity::Init, 100));
        state.merge_sessions(incoming);

        assert_eq!(state.sessions[&1].activity, Activity::Thinking);
        assert_eq!(state.sessions[&1].last_event_ts, 200);
    }

    #[test]
    fn merge_sessions_new_pane_added() {
        let mut state = State::default();
        state
            .sessions
            .insert(1, make_session(1, Activity::Init, 100));

        let mut incoming = BTreeMap::new();
        incoming.insert(2, make_session(2, Activity::Thinking, 150));
        state.merge_sessions(incoming);

        assert_eq!(state.sessions.len(), 2);
        assert!(state.sessions.contains_key(&2));
    }

    #[test]
    fn merge_sessions_refreshes_tab_names() {
        let mut state = State::default();
        state
            .pane_to_tab
            .insert(1, (0, "local-tab".to_string()));

        let mut incoming = BTreeMap::new();
        let mut session = make_session(1, Activity::Thinking, 200);
        session.tab_name = Some("remote-tab".to_string());
        incoming.insert(1, session);
        state.merge_sessions(incoming);

        // Should use local pane map, not remote tab name
        assert_eq!(state.sessions[&1].tab_name.as_deref(), Some("local-tab"));
    }

    #[test]
    fn cleanup_stale_sessions_done_to_idle() {
        let mut state = State::default();
        state
            .sessions
            .insert(1, make_session(1, Activity::Done, 1)); // very old timestamp

        let changed = state.cleanup_stale_sessions();
        assert!(changed);
        assert_eq!(state.sessions[&1].activity, Activity::Idle);
    }

    #[test]
    fn cleanup_stale_sessions_agent_done_to_idle() {
        let mut state = State::default();
        state
            .sessions
            .insert(1, make_session(1, Activity::AgentDone, 1));

        let changed = state.cleanup_stale_sessions();
        assert!(changed);
        assert_eq!(state.sessions[&1].activity, Activity::Idle);
    }

    #[test]
    fn cleanup_stale_sessions_recent_done_unchanged() {
        let mut state = State::default();
        let now = unix_now();
        state
            .sessions
            .insert(1, make_session(1, Activity::Done, now));

        let changed = state.cleanup_stale_sessions();
        assert!(!changed);
        assert_eq!(state.sessions[&1].activity, Activity::Done);
    }

    #[test]
    fn cleanup_stale_sessions_thinking_untouched() {
        let mut state = State::default();
        state
            .sessions
            .insert(1, make_session(1, Activity::Thinking, 1));

        let changed = state.cleanup_stale_sessions();
        assert!(!changed);
        assert_eq!(state.sessions[&1].activity, Activity::Thinking);
    }

    #[test]
    fn cleanup_expired_flashes_removes_expired() {
        let mut state = State::default();
        state.flash_deadlines.insert(1, 1); // expired (timestamp 1ms)
        state.flash_deadlines.insert(2, u64::MAX); // not expired

        let changed = state.cleanup_expired_flashes();
        assert!(changed);
        assert_eq!(state.flash_deadlines.len(), 1);
        assert!(state.flash_deadlines.contains_key(&2));
    }

    #[test]
    fn cleanup_expired_flashes_no_change_when_all_active() {
        let mut state = State::default();
        state.flash_deadlines.insert(1, u64::MAX);

        let changed = state.cleanup_expired_flashes();
        assert!(!changed);
        assert_eq!(state.flash_deadlines.len(), 1);
    }

    #[test]
    fn clear_flashes_on_tab_removes_correct_flashes() {
        let mut state = State::default();
        state
            .sessions
            .insert(1, make_session_with_tab(1, Activity::Waiting, 100, 0));
        state
            .sessions
            .insert(2, make_session_with_tab(2, Activity::Waiting, 100, 0));
        state
            .sessions
            .insert(3, make_session_with_tab(3, Activity::Waiting, 100, 1));

        state.flash_deadlines.insert(1, u64::MAX);
        state.flash_deadlines.insert(2, u64::MAX);
        state.flash_deadlines.insert(3, u64::MAX);

        state.clear_flashes_on_tab(0);

        assert!(!state.flash_deadlines.contains_key(&1));
        assert!(!state.flash_deadlines.contains_key(&2));
        assert!(state.flash_deadlines.contains_key(&3)); // tab 1, not cleared
    }

    #[test]
    fn has_active_flashes_true_when_active() {
        let mut state = State::default();
        state.flash_deadlines.insert(1, u64::MAX);
        assert!(state.has_active_flashes());
    }

    #[test]
    fn has_active_flashes_false_when_expired() {
        let mut state = State::default();
        state.flash_deadlines.insert(1, 1); // expired
        assert!(!state.has_active_flashes());
    }

    #[test]
    fn has_active_flashes_false_when_empty() {
        let state = State::default();
        assert!(!state.has_active_flashes());
    }

    #[test]
    fn has_elapsed_display_false_when_disabled() {
        let mut state = State::default();
        state.settings.elapsed_time = false;
        state
            .sessions
            .insert(1, make_session(1, Activity::Thinking, 1));
        assert!(!state.has_elapsed_display());
    }

    #[test]
    fn has_elapsed_display_true_for_old_non_idle() {
        let mut state = State::default();
        state.settings.elapsed_time = true;
        state
            .sessions
            .insert(1, make_session(1, Activity::Thinking, 1)); // very old
        assert!(state.has_elapsed_display());
    }

    #[test]
    fn has_elapsed_display_false_for_idle() {
        let mut state = State::default();
        state.settings.elapsed_time = true;
        state
            .sessions
            .insert(1, make_session(1, Activity::Idle, 1));
        assert!(!state.has_elapsed_display());
    }

    #[test]
    fn has_elapsed_display_false_for_recent() {
        let mut state = State::default();
        state.settings.elapsed_time = true;
        let now = unix_now();
        state
            .sessions
            .insert(1, make_session(1, Activity::Thinking, now));
        assert!(!state.has_elapsed_display());
    }

    #[test]
    fn remove_dead_panes_removes_orphaned_sessions() {
        let mut state = State::default();
        state.sessions.insert(1, make_session(1, Activity::Init, 100));
        state.sessions.insert(2, make_session(2, Activity::Init, 100));
        state.pane_to_tab.insert(1, (0, "tab0".to_string()));
        // pane 2 not in pane_to_tab → dead

        state.remove_dead_panes();
        assert_eq!(state.sessions.len(), 1);
        assert!(state.sessions.contains_key(&1));
    }

    #[test]
    fn refresh_session_tab_names_updates_from_map() {
        let mut state = State::default();
        state.sessions.insert(
            1,
            SessionInfo {
                tab_index: None,
                tab_name: None,
                ..make_session(1, Activity::Init, 100)
            },
        );
        state.pane_to_tab.insert(1, (2, "updated-tab".to_string()));

        state.refresh_session_tab_names();
        assert_eq!(state.sessions[&1].tab_index, Some(2));
        assert_eq!(
            state.sessions[&1].tab_name.as_deref(),
            Some("updated-tab")
        );
    }

    #[test]
    fn rebuild_pane_map_updates_sessions() {
        let mut state = State::default();
        state.tabs = vec![TabInfo {
            position: 0,
            name: "main".to_string(),
            active: true,
            ..Default::default()
        }];

        let mut panes = HashMap::new();
        panes.insert(
            0,
            vec![PaneInfo {
                id: 10,
                title: "term".to_string(),
                ..Default::default()
            }],
        );
        state.pane_manifest = Some(PaneManifest { panes });

        // Add a session for pane 10
        state
            .sessions
            .insert(10, make_session(10, Activity::Init, 100));

        state.rebuild_pane_map();

        assert_eq!(state.sessions[&10].tab_index, Some(0));
        assert_eq!(state.sessions[&10].tab_name.as_deref(), Some("main"));
    }
}
