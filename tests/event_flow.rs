use std::collections::BTreeMap;
use zellaude::event_handler::handle_hook_event;
use zellaude::state::{Activity, HookPayload, SessionInfo, Settings, State};

fn make_payload(pane_id: u32, event: &str) -> HookPayload {
    HookPayload {
        session_id: Some("test-session".to_string()),
        pane_id,
        hook_event: event.to_string(),
        tool_name: None,
        cwd: Some("/tmp".to_string()),
        zellij_session: Some("test".to_string()),
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
fn full_session_lifecycle() {
    let mut state = State::default();

    // SessionStart
    handle_hook_event(&mut state, make_payload(1, "SessionStart"));
    assert_eq!(state.sessions[&1].activity, Activity::Init);

    // PreToolUse
    handle_hook_event(&mut state, make_tool_payload(1, "Bash"));
    assert_eq!(
        state.sessions[&1].activity,
        Activity::Tool("Bash".to_string())
    );

    // PostToolUse
    handle_hook_event(&mut state, make_payload(1, "PostToolUse"));
    assert_eq!(state.sessions[&1].activity, Activity::Thinking);

    // Another tool
    handle_hook_event(&mut state, make_tool_payload(1, "Read"));
    assert_eq!(
        state.sessions[&1].activity,
        Activity::Tool("Read".to_string())
    );

    // Stop
    handle_hook_event(&mut state, make_payload(1, "Stop"));
    assert_eq!(state.sessions[&1].activity, Activity::Done);

    // After timeout → Idle
    // Simulate old timestamp
    state.sessions.get_mut(&1).unwrap().last_event_ts = 1;
    let changed = state.cleanup_stale_sessions();
    assert!(changed);
    assert_eq!(state.sessions[&1].activity, Activity::Idle);
}

#[test]
fn multiple_concurrent_sessions() {
    let mut state = State::default();

    handle_hook_event(&mut state, make_payload(1, "SessionStart"));
    handle_hook_event(&mut state, make_payload(2, "SessionStart"));
    handle_hook_event(&mut state, make_payload(3, "SessionStart"));
    assert_eq!(state.sessions.len(), 3);

    // Different activities on different panes
    handle_hook_event(&mut state, make_tool_payload(1, "Bash"));
    handle_hook_event(&mut state, make_payload(2, "PermissionRequest"));
    handle_hook_event(&mut state, make_payload(3, "Stop"));

    assert_eq!(
        state.sessions[&1].activity,
        Activity::Tool("Bash".to_string())
    );
    assert_eq!(state.sessions[&2].activity, Activity::Waiting);
    assert_eq!(state.sessions[&3].activity, Activity::Done);

    // End one session
    handle_hook_event(&mut state, make_payload(2, "SessionEnd"));
    assert_eq!(state.sessions.len(), 2);
    assert!(!state.sessions.contains_key(&2));
}

#[test]
fn session_merge_between_instances() {
    let mut state_a = State::default();
    let mut state_b = State::default();

    // Instance A has session 1
    handle_hook_event(&mut state_a, make_payload(1, "SessionStart"));
    handle_hook_event(&mut state_a, make_tool_payload(1, "Edit"));

    // Instance B has session 2
    handle_hook_event(&mut state_b, make_payload(2, "SessionStart"));

    // Simulate sync: B receives A's sessions
    let a_json = serde_json::to_string(&state_a.sessions).unwrap();
    let a_sessions: BTreeMap<u32, SessionInfo> = serde_json::from_str(&a_json).unwrap();
    state_b.merge_sessions(a_sessions);

    // B should now have both sessions
    assert_eq!(state_b.sessions.len(), 2);
    assert!(state_b.sessions.contains_key(&1));
    assert!(state_b.sessions.contains_key(&2));
    assert_eq!(
        state_b.sessions[&1].activity,
        Activity::Tool("Edit".to_string())
    );
}

#[test]
fn settings_json_roundtrip() {
    let settings = Settings {
        notifications: zellaude::state::NotifyMode::Unfocused,
        flash: zellaude::state::FlashMode::Persist,
        elapsed_time: false,
    };
    let json = serde_json::to_string(&settings).unwrap();
    let restored: Settings = serde_json::from_str(&json).unwrap();
    assert_eq!(
        restored.notifications,
        zellaude::state::NotifyMode::Unfocused
    );
    assert_eq!(restored.flash, zellaude::state::FlashMode::Persist);
    assert!(!restored.elapsed_time);
}

#[test]
fn permission_request_flash_lifecycle() {
    let mut state = State::default();
    state.settings.flash = zellaude::state::FlashMode::Once;

    // Permission request triggers flash
    handle_hook_event(&mut state, make_payload(1, "PermissionRequest"));
    assert!(state.flash_deadlines.contains_key(&1));

    // Next event clears flash
    handle_hook_event(&mut state, make_payload(1, "PostToolUse"));
    assert!(!state.flash_deadlines.contains_key(&1));
}

#[test]
fn subagent_lifecycle() {
    let mut state = State::default();

    handle_hook_event(&mut state, make_payload(1, "SessionStart"));
    handle_hook_event(&mut state, make_tool_payload(1, "Bash"));
    handle_hook_event(&mut state, make_payload(1, "SubagentStop"));
    assert_eq!(state.sessions[&1].activity, Activity::AgentDone);

    // AgentDone should also transition to Idle after timeout
    state.sessions.get_mut(&1).unwrap().last_event_ts = 1;
    state.cleanup_stale_sessions();
    assert_eq!(state.sessions[&1].activity, Activity::Idle);
}
