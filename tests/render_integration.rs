use zellaude::render;
use zellaude::state::{Activity, SessionInfo, State, ViewMode};
use zellij_tile::prelude::*;

fn make_tab(position: usize, name: &str, active: bool) -> TabInfo {
    TabInfo {
        position,
        name: name.to_string(),
        active,
        ..Default::default()
    }
}

fn make_session(pane_id: u32, activity: Activity, tab_idx: usize) -> SessionInfo {
    SessionInfo {
        session_id: format!("s{pane_id}"),
        pane_id,
        activity,
        tab_name: Some(format!("tab{tab_idx}")),
        tab_index: Some(tab_idx),
        last_event_ts: 1_700_000_000,
        cwd: None,
    }
}

/// Run render and verify it doesn't panic. We can't easily capture print! output
/// but we can verify side effects (click regions, etc.)
fn run_render(state: &mut State, rows: usize, cols: usize) {
    render::render_status_bar(state, rows, cols);
}

#[test]
fn render_no_sessions_no_panic() {
    let mut state = State::default();
    state.tabs = vec![make_tab(0, "main", true)];
    run_render(&mut state, 1, 80);
}

#[test]
fn render_with_sessions_no_panic() {
    let mut state = State::default();
    state.tabs = vec![make_tab(0, "dev", true), make_tab(1, "test", false)];
    state
        .sessions
        .insert(1, make_session(1, Activity::Thinking, 0));
    state.pane_to_tab.insert(1, (0, "dev".to_string()));

    run_render(&mut state, 1, 120);
}

#[test]
fn render_narrow_terminal_no_panic() {
    let mut state = State::default();
    state.tabs = vec![make_tab(0, "main", true)];

    run_render(&mut state, 1, 3);
    run_render(&mut state, 1, 1);
    run_render(&mut state, 1, 0);
}

#[test]
fn render_creates_click_regions_for_tabs() {
    let mut state = State::default();
    state.tabs = vec![
        make_tab(0, "dev", true),
        make_tab(1, "test", false),
        make_tab(2, "docs", false),
    ];

    run_render(&mut state, 1, 120);

    assert!(
        !state.click_regions.is_empty(),
        "should have click regions for tabs"
    );
}

#[test]
fn render_click_regions_non_overlapping() {
    let mut state = State::default();
    state.tabs = vec![
        make_tab(0, "alpha", true),
        make_tab(1, "beta", false),
        make_tab(2, "gamma", false),
    ];

    run_render(&mut state, 1, 120);

    for i in 0..state.click_regions.len() {
        for j in (i + 1)..state.click_regions.len() {
            let a = &state.click_regions[i];
            let b = &state.click_regions[j];
            assert!(
                a.end_col <= b.start_col || b.end_col <= a.start_col,
                "click regions {i} ({}-{}) and {j} ({}-{}) overlap",
                a.start_col,
                a.end_col,
                b.start_col,
                b.end_col,
            );
        }
    }
}

#[test]
fn render_waiting_session_has_waiting_click_region() {
    let mut state = State::default();
    state.tabs = vec![make_tab(0, "dev", true)];
    state
        .sessions
        .insert(1, make_session(1, Activity::Waiting, 0));
    state.pane_to_tab.insert(1, (0, "dev".to_string()));

    run_render(&mut state, 1, 120);

    let waiting_regions: Vec<_> = state.click_regions.iter().filter(|r| r.is_waiting).collect();
    assert!(
        !waiting_regions.is_empty(),
        "waiting session should produce a waiting click region"
    );
    assert_eq!(waiting_regions[0].pane_id, 1);
}

#[test]
fn render_settings_menu_produces_menu_regions() {
    let mut state = State::default();
    state.tabs = vec![make_tab(0, "main", true)];
    state.view_mode = ViewMode::Settings;

    run_render(&mut state, 1, 120);

    assert!(
        !state.menu_click_regions.is_empty(),
        "settings menu should produce menu click regions"
    );
    // Should have at least: notifications, flash, elapsed time, close button
    assert!(
        state.menu_click_regions.len() >= 4,
        "expected at least 4 menu regions, got {}",
        state.menu_click_regions.len()
    );
}

#[test]
fn render_prefix_click_region_set() {
    let mut state = State::default();
    state.tabs = vec![make_tab(0, "main", true)];

    run_render(&mut state, 1, 80);

    assert!(
        state.prefix_click_region.is_some(),
        "prefix click region should be set"
    );
    let (start, end) = state.prefix_click_region.unwrap();
    assert_eq!(start, 0);
    assert!(end > 0, "prefix should have positive width");
}

#[test]
fn render_all_activity_types() {
    let activities = vec![
        Activity::Init,
        Activity::Thinking,
        Activity::Tool("Bash".to_string()),
        Activity::Prompting,
        Activity::Waiting,
        Activity::Notification,
        Activity::Done,
        Activity::AgentDone,
        Activity::Idle,
    ];

    for (i, activity) in activities.into_iter().enumerate() {
        let mut state = State::default();
        let pane_id = (i + 1) as u32;
        state.tabs = vec![make_tab(0, "test", true)];
        state
            .sessions
            .insert(pane_id, make_session(pane_id, activity, 0));
        state.pane_to_tab.insert(pane_id, (0, "test".to_string()));

        run_render(&mut state, 1, 80);
    }
}

#[test]
fn render_many_tabs_doesnt_overflow() {
    let mut state = State::default();
    for i in 0..20 {
        state
            .tabs
            .push(make_tab(i, &format!("tab-{i}"), i == 0));
    }

    // Narrow terminal — should gracefully truncate
    run_render(&mut state, 1, 40);
    // Wide terminal
    run_render(&mut state, 1, 200);
}

#[test]
fn render_with_elapsed_time() {
    let mut state = State::default();
    state.settings.elapsed_time = true;
    state.tabs = vec![make_tab(0, "dev", true)];
    state.sessions.insert(
        1,
        SessionInfo {
            last_event_ts: 1, // very old — will show elapsed
            ..make_session(1, Activity::Thinking, 0)
        },
    );
    state.pane_to_tab.insert(1, (0, "dev".to_string()));

    run_render(&mut state, 1, 120);
}
