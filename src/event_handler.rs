use crate::state::{Activity, FlashMode, HookPayload, SessionInfo, State};

/// Returns `true` if the event changed visible state and a render is needed.
pub fn handle_hook_event(state: &mut State, payload: HookPayload) -> bool {
    // Capture env info for use in notifications
    if let Some(ref name) = payload.zellij_session {
        state.zellij_session_name = Some(name.clone());
    }
    if let Some(ref tp) = payload.term_program {
        state.term_program = Some(tp.clone());
    }

    let event = payload.hook_event.as_str();

    // SessionEnd → remove session (only triggers render if session existed)
    if event == "SessionEnd" {
        return state.sessions.remove(&payload.pane_id).is_some();
    }

    let activity = match event {
        "SessionStart" => Activity::Init,
        "PreToolUse" => {
            Activity::Tool(payload.tool_name.clone().unwrap_or_default())
        }
        "PostToolUse" | "PostToolUseFailure" => Activity::Thinking,
        "UserPromptSubmit" => Activity::Thinking,
        "PermissionRequest" => Activity::Waiting,
        // Notification is informational — just refresh the timestamp, keep current activity.
        // No render needed: the timer will pick up the updated timestamp.
        "Notification" => {
            if let Some(session) = state.sessions.get_mut(&payload.pane_id) {
                session.last_event_ts = crate::state::unix_now();
            }
            return false;
        }
        "Stop" => Activity::Done,
        "SubagentStop" => Activity::AgentDone,
        _ => Activity::Idle,
    };

    // Skip render if activity hasn't changed
    if let Some(existing) = state.sessions.get_mut(&payload.pane_id) {
        if existing.activity == activity {
            // Still update timestamp and metadata even if we skip the render
            existing.last_event_ts = crate::state::unix_now();
            if let Some(sid) = &payload.session_id {
                existing.session_id = sid.clone();
            }
            if let Some(cwd) = payload.cwd {
                existing.cwd = Some(cwd);
            }
            return false;
        }
    }

    let (tab_index, tab_name) = state
        .pane_to_tab
        .get(&payload.pane_id)
        .cloned()
        .unzip();

    let session = state
        .sessions
        .entry(payload.pane_id)
        .or_insert_with(|| SessionInfo {
            session_id: payload.session_id.clone().unwrap_or_default(),
            pane_id: payload.pane_id,
            activity: Activity::Init,
            tab_name: None,
            tab_index: None,
            last_event_ts: 0,
            cwd: None,
        });

    if matches!(activity, Activity::Waiting) {
        match state.settings.flash {
            FlashMode::Once => {
                state.flash_deadlines.insert(
                    payload.pane_id,
                    crate::state::unix_now_ms() + crate::state::FLASH_DURATION_MS,
                );
            }
            FlashMode::Persist => {
                state.flash_deadlines.insert(payload.pane_id, u64::MAX);
            }
            FlashMode::Off => {}
        }
        // Desktop notification is handled by the hook script to avoid
        // duplicates from multiple plugin instances.
    } else {
        state.flash_deadlines.remove(&payload.pane_id);
    }

    session.activity = activity;
    session.last_event_ts = crate::state::unix_now();
    if let Some(sid) = &payload.session_id {
        session.session_id = sid.clone();
    }
    if let Some(cwd) = payload.cwd {
        session.cwd = Some(cwd);
    }
    if let Some((idx, name)) = tab_index.zip(tab_name) {
        session.tab_index = Some(idx);
        session.tab_name = Some(name);
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::unix_now;

    fn make_payload(pane_id: u32, event: &str) -> HookPayload {
        HookPayload {
            session_id: Some("test-session".to_string()),
            pane_id,
            hook_event: event.to_string(),
            tool_name: None,
            cwd: Some("/tmp".to_string()),
            zellij_session: Some("zellij-test".to_string()),
            term_program: Some("xterm".to_string()),
        }
    }

    fn make_tool_payload(pane_id: u32, tool: &str) -> HookPayload {
        HookPayload {
            tool_name: Some(tool.to_string()),
            ..make_payload(pane_id, "PreToolUse")
        }
    }

    #[test]
    fn session_start_creates_init_session() {
        let mut state = State::default();
        handle_hook_event(&mut state, make_payload(1, "SessionStart"));
        assert_eq!(state.sessions.len(), 1);
        let session = state.sessions.get(&1).unwrap();
        assert_eq!(session.activity, Activity::Init);
        assert_eq!(session.session_id, "test-session");
    }

    #[test]
    fn session_end_removes_session() {
        let mut state = State::default();
        handle_hook_event(&mut state, make_payload(1, "SessionStart"));
        assert_eq!(state.sessions.len(), 1);
        handle_hook_event(&mut state, make_payload(1, "SessionEnd"));
        assert_eq!(state.sessions.len(), 0);
    }

    #[test]
    fn pre_tool_use_sets_tool_activity() {
        let mut state = State::default();
        handle_hook_event(&mut state, make_tool_payload(1, "Bash"));
        let session = state.sessions.get(&1).unwrap();
        assert_eq!(session.activity, Activity::Tool("Bash".to_string()));
    }

    #[test]
    fn post_tool_use_sets_thinking() {
        let mut state = State::default();
        handle_hook_event(&mut state, make_payload(1, "PostToolUse"));
        assert_eq!(state.sessions.get(&1).unwrap().activity, Activity::Thinking);
    }

    #[test]
    fn post_tool_use_failure_sets_thinking() {
        let mut state = State::default();
        handle_hook_event(&mut state, make_payload(1, "PostToolUseFailure"));
        assert_eq!(state.sessions.get(&1).unwrap().activity, Activity::Thinking);
    }

    #[test]
    fn user_prompt_submit_sets_thinking() {
        let mut state = State::default();
        handle_hook_event(&mut state, make_payload(1, "UserPromptSubmit"));
        assert_eq!(state.sessions.get(&1).unwrap().activity, Activity::Thinking);
    }

    #[test]
    fn permission_request_sets_waiting() {
        let mut state = State::default();
        handle_hook_event(&mut state, make_payload(1, "PermissionRequest"));
        assert_eq!(state.sessions.get(&1).unwrap().activity, Activity::Waiting);
    }

    #[test]
    fn stop_sets_done() {
        let mut state = State::default();
        handle_hook_event(&mut state, make_payload(1, "Stop"));
        assert_eq!(state.sessions.get(&1).unwrap().activity, Activity::Done);
    }

    #[test]
    fn subagent_stop_sets_agent_done() {
        let mut state = State::default();
        handle_hook_event(&mut state, make_payload(1, "SubagentStop"));
        assert_eq!(
            state.sessions.get(&1).unwrap().activity,
            Activity::AgentDone
        );
    }

    #[test]
    fn unknown_event_sets_idle() {
        let mut state = State::default();
        handle_hook_event(&mut state, make_payload(1, "SomeUnknownEvent"));
        assert_eq!(state.sessions.get(&1).unwrap().activity, Activity::Idle);
    }

    #[test]
    fn notification_updates_timestamp_keeps_activity() {
        let mut state = State::default();
        handle_hook_event(&mut state, make_tool_payload(1, "Bash"));
        let ts_before = state.sessions.get(&1).unwrap().last_event_ts;

        // Notification should keep the Tool activity
        handle_hook_event(&mut state, make_payload(1, "Notification"));
        let session = state.sessions.get(&1).unwrap();
        assert_eq!(session.activity, Activity::Tool("Bash".to_string()));
        assert!(session.last_event_ts >= ts_before);
    }

    #[test]
    fn notification_on_nonexistent_session_does_nothing() {
        let mut state = State::default();
        handle_hook_event(&mut state, make_payload(99, "Notification"));
        assert!(state.sessions.is_empty());
    }

    #[test]
    fn flash_deadline_set_for_waiting_flash_once() {
        let mut state = State::default();
        state.settings.flash = FlashMode::Once;
        handle_hook_event(&mut state, make_payload(1, "PermissionRequest"));
        assert!(state.flash_deadlines.contains_key(&1));
        let deadline = state.flash_deadlines[&1];
        assert!(deadline < u64::MAX);
        assert!(deadline > 0);
    }

    #[test]
    fn flash_deadline_max_for_waiting_flash_persist() {
        let mut state = State::default();
        state.settings.flash = FlashMode::Persist;
        handle_hook_event(&mut state, make_payload(1, "PermissionRequest"));
        assert_eq!(state.flash_deadlines[&1], u64::MAX);
    }

    #[test]
    fn no_flash_deadline_for_waiting_flash_off() {
        let mut state = State::default();
        state.settings.flash = FlashMode::Off;
        handle_hook_event(&mut state, make_payload(1, "PermissionRequest"));
        assert!(!state.flash_deadlines.contains_key(&1));
    }

    #[test]
    fn non_waiting_event_clears_flash_deadline() {
        let mut state = State::default();
        state.flash_deadlines.insert(1, u64::MAX);
        handle_hook_event(&mut state, make_payload(1, "PostToolUse"));
        assert!(!state.flash_deadlines.contains_key(&1));
    }

    #[test]
    fn zellij_session_and_term_program_captured() {
        let mut state = State::default();
        handle_hook_event(&mut state, make_payload(1, "SessionStart"));
        assert_eq!(state.zellij_session_name.as_deref(), Some("zellij-test"));
        assert_eq!(state.term_program.as_deref(), Some("xterm"));
    }

    #[test]
    fn session_updated_on_subsequent_events() {
        let mut state = State::default();
        handle_hook_event(&mut state, make_payload(1, "SessionStart"));
        assert_eq!(state.sessions.get(&1).unwrap().activity, Activity::Init);

        handle_hook_event(&mut state, make_tool_payload(1, "Read"));
        assert_eq!(
            state.sessions.get(&1).unwrap().activity,
            Activity::Tool("Read".to_string())
        );
        assert_eq!(state.sessions.len(), 1); // same session, not a new one
    }

    #[test]
    fn cwd_and_session_id_updated() {
        let mut state = State::default();
        let mut payload = make_payload(1, "SessionStart");
        payload.cwd = Some("/home/user".to_string());
        payload.session_id = Some("sid-1".to_string());
        handle_hook_event(&mut state, payload);

        let session = state.sessions.get(&1).unwrap();
        assert_eq!(session.cwd.as_deref(), Some("/home/user"));
        assert_eq!(session.session_id, "sid-1");
    }

    #[test]
    fn pane_to_tab_mapping_used_for_new_session() {
        let mut state = State::default();
        state
            .pane_to_tab
            .insert(1, (0, "my-tab".to_string()));
        handle_hook_event(&mut state, make_payload(1, "SessionStart"));
        let session = state.sessions.get(&1).unwrap();
        assert_eq!(session.tab_index, Some(0));
        assert_eq!(session.tab_name.as_deref(), Some("my-tab"));
    }

    #[test]
    fn last_event_ts_is_recent() {
        let mut state = State::default();
        let before = unix_now();
        handle_hook_event(&mut state, make_payload(1, "SessionStart"));
        let after = unix_now();
        let ts = state.sessions.get(&1).unwrap().last_event_ts;
        assert!(ts >= before && ts <= after);
    }

    #[test]
    fn multiple_panes_tracked_independently() {
        let mut state = State::default();
        handle_hook_event(&mut state, make_payload(1, "SessionStart"));
        handle_hook_event(&mut state, make_payload(2, "PreToolUse"));
        assert_eq!(state.sessions.len(), 2);
        assert_eq!(state.sessions.get(&1).unwrap().activity, Activity::Init);
        assert_eq!(
            state.sessions.get(&2).unwrap().activity,
            Activity::Tool(String::new())
        );
    }
}
