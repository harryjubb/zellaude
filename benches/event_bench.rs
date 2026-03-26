use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use std::collections::{BTreeMap, HashMap};
use zellaude::event_handler::handle_hook_event;
use zellaude::state::{Activity, HookPayload, SessionInfo, State};
use zellaude::tab_pane_map::build_pane_to_tab_map;
use zellij_tile::prelude::*;

fn make_payload(pane_id: u32, event: &str, tool: Option<&str>) -> HookPayload {
    HookPayload {
        session_id: Some("test-session".to_string()),
        pane_id,
        hook_event: event.to_string(),
        tool_name: tool.map(|s| s.to_string()),
        cwd: Some("/tmp".to_string()),
        zellij_session: Some("test".to_string()),
        term_program: Some("xterm".to_string()),
    }
}

fn bench_handle_hook_event(c: &mut Criterion) {
    let events = [
        ("SessionStart", None),
        ("PreToolUse", Some("Bash")),
        ("PostToolUse", None),
        ("UserPromptSubmit", None),
        ("PermissionRequest", None),
        ("Stop", None),
    ];

    let mut group = c.benchmark_group("handle_hook_event");
    for (event, tool) in &events {
        group.bench_with_input(BenchmarkId::new("event", event), event, |b, event| {
            b.iter(|| {
                let mut state = State::default();
                let payload = make_payload(1, event, *tool);
                handle_hook_event(&mut state, black_box(payload));
            })
        });
    }
    group.finish();
}

fn bench_session_merge(c: &mut Criterion) {
    let mut group = c.benchmark_group("session_merge");
    for n in [10, 100, 1000] {
        group.bench_with_input(BenchmarkId::new("sessions", n), &n, |b, &n| {
            let incoming: BTreeMap<u32, SessionInfo> = (0..n)
                .map(|i| {
                    (
                        i,
                        SessionInfo {
                            session_id: format!("s{i}"),
                            pane_id: i,
                            activity: Activity::Thinking,
                            tab_name: Some(format!("tab{i}")),
                            tab_index: Some(i as usize),
                            last_event_ts: 1000 + i as u64,
                            cwd: Some("/tmp".to_string()),
                        },
                    )
                })
                .collect();

            b.iter(|| {
                let mut state = State::default();
                state.merge_sessions(black_box(incoming.clone()));
            })
        });
    }
    group.finish();
}

fn bench_build_pane_to_tab_map(c: &mut Criterion) {
    let mut group = c.benchmark_group("build_pane_to_tab_map");
    for n_tabs in [5, 20, 50] {
        let panes_per_tab = 3u32;
        let tabs: Vec<TabInfo> = (0..n_tabs)
            .map(|i| TabInfo {
                position: i,
                name: format!("tab{i}"),
                active: i == 0,
                ..Default::default()
            })
            .collect();

        let mut pane_map: HashMap<usize, Vec<PaneInfo>> = HashMap::new();
        let mut pane_id = 0u32;
        for i in 0..n_tabs {
            let panes: Vec<PaneInfo> = (0..panes_per_tab)
                .map(|_| {
                    pane_id += 1;
                    PaneInfo {
                        id: pane_id,
                        title: format!("pane{pane_id}"),
                        ..Default::default()
                    }
                })
                .collect();
            pane_map.insert(i, panes);
        }
        let manifest = PaneManifest { panes: pane_map };

        group.bench_with_input(
            BenchmarkId::new("tabs", n_tabs),
            &(tabs, manifest),
            |b, (tabs, manifest)| {
                b.iter(|| build_pane_to_tab_map(black_box(tabs), black_box(manifest)))
            },
        );
    }
    group.finish();
}

/// P1: Repeated same-activity event (no visible state change).
/// After conditional render fix, handler detects no-change.
/// This confirms the detection cost is negligible.
fn bench_repeated_same_activity(c: &mut Criterion) {
    c.bench_function("repeated_same_activity", |b| {
        b.iter_batched(
            || {
                let mut state = State::default();
                // Pre-populate with a Bash tool session
                handle_hook_event(&mut state, make_payload(1, "PreToolUse", Some("Bash")));
                state
            },
            |mut state| {
                // Send the same event again — activity doesn't change
                let payload = make_payload(1, "PreToolUse", Some("Bash"));
                handle_hook_event(black_box(&mut state), payload);
            },
            criterion::BatchSize::SmallInput,
        )
    });
}

criterion_group!(
    benches,
    bench_handle_hook_event,
    bench_session_merge,
    bench_build_pane_to_tab_map,
    bench_repeated_same_activity
);
criterion_main!(benches);
