use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use std::collections::HashMap;
use zellaude::render::{
    activity_priority, activity_style, arrow, bg, fg, format_elapsed, mode_style,
    render_status_bar, render_tabs, BAR_BG, PREFIX_BG,
};
use zellaude::state::{Activity, SessionInfo, State};
use zellij_tile::prelude::*;

/// Build a State with `n_tabs` tabs and `n_sessions` sessions spread across tabs.
fn build_state(n_tabs: usize, n_sessions: usize) -> State {
    let mut state = State::default();
    state.zellij_session_name = Some("bench-session".to_string());

    // Create tabs
    state.tabs = (0..n_tabs)
        .map(|i| TabInfo {
            position: i,
            name: format!("tab-{i}"),
            active: i == 0,
            ..Default::default()
        })
        .collect();

    // Create pane manifest (1 pane per tab + extra for sessions)
    let mut pane_map: HashMap<usize, Vec<PaneInfo>> = HashMap::new();
    let mut pane_id = 0u32;
    for i in 0..n_tabs {
        let n_panes = 1 + (n_sessions / n_tabs.max(1)).max(1);
        let panes: Vec<PaneInfo> = (0..n_panes)
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
    state.pane_manifest = Some(PaneManifest { panes: pane_map });
    state.rebuild_pane_map();

    // Create sessions spread across tabs
    let activities = [
        Activity::Thinking,
        Activity::Tool("Bash".to_string()),
        Activity::Waiting,
        Activity::Done,
        Activity::Init,
    ];
    for i in 0..n_sessions {
        let tab_idx = i % n_tabs;
        // Use pane IDs that exist in the manifest
        let pane_id = (tab_idx * (1 + (n_sessions / n_tabs.max(1)).max(1)) + 1) as u32 + (i / n_tabs) as u32;
        state.sessions.insert(
            pane_id,
            SessionInfo {
                session_id: format!("s{i}"),
                pane_id,
                activity: activities[i % activities.len()].clone(),
                tab_name: Some(format!("tab-{tab_idx}")),
                tab_index: Some(tab_idx),
                last_event_ts: 1, // old enough for elapsed display
                cwd: Some("/tmp".to_string()),
            },
        );
    }

    state
}

fn bench_format_elapsed(c: &mut Criterion) {
    c.bench_function("format_elapsed_seconds", |b| {
        b.iter(|| format_elapsed(black_box(45)))
    });
    c.bench_function("format_elapsed_minutes", |b| {
        b.iter(|| format_elapsed(black_box(300)))
    });
    c.bench_function("format_elapsed_hours", |b| {
        b.iter(|| format_elapsed(black_box(7200)))
    });
}

fn bench_activity_style(c: &mut Criterion) {
    let activities = vec![
        Activity::Init,
        Activity::Thinking,
        Activity::Tool("Bash".to_string()),
        Activity::Prompting,
        Activity::Waiting,
        Activity::Done,
        Activity::Idle,
    ];

    c.bench_function("activity_style_all_variants", |b| {
        b.iter(|| {
            for a in &activities {
                black_box(activity_style(a));
            }
        })
    });

    c.bench_function("activity_priority_all_variants", |b| {
        b.iter(|| {
            for a in &activities {
                black_box(activity_priority(a));
            }
        })
    });
}

fn bench_mode_style(c: &mut Criterion) {
    c.bench_function("mode_style_normal", |b| {
        b.iter(|| mode_style(black_box(InputMode::Normal)))
    });
}

/// P0 baseline: full render_status_bar cost at various scales.
/// Establishes per-render cost; after render caching, amortized cost drops.
fn bench_render_status_bar(c: &mut Criterion) {
    let mut group = c.benchmark_group("render_status_bar");
    for (n_tabs, n_sessions) in [(5, 3), (10, 10), (20, 20)] {
        group.bench_with_input(
            BenchmarkId::new(format!("{n_tabs}t_{n_sessions}s"), n_tabs * 100 + n_sessions),
            &(n_tabs, n_sessions),
            |b, &(t, s)| {
                b.iter_batched(
                    || build_state(t, s),
                    |mut state| {
                        render_status_bar(black_box(&mut state), 1, 120);
                    },
                    criterion::BatchSize::SmallInput,
                )
            },
        );
    }
    group.finish();
}

/// P2: O(T*S) session scan scaling in render_tabs.
/// Exposes the 3x full-scan cost; after pre-grouping by tab, should improve.
fn bench_render_tabs_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("render_tabs_scaling");
    for (n_tabs, n_sessions) in [(5, 5), (10, 20), (20, 60)] {
        group.bench_with_input(
            BenchmarkId::new(
                format!("{n_tabs}t_{n_sessions}s"),
                n_tabs * 100 + n_sessions,
            ),
            &(n_tabs, n_sessions),
            |b, &(t, s)| {
                b.iter_batched(
                    || {
                        let state = build_state(t, s);
                        let buf = String::with_capacity(512);
                        (state, buf)
                    },
                    |(mut state, mut buf)| {
                        let mut col = 15; // simulate prefix width
                        render_tabs(
                            black_box(&mut state),
                            &mut buf,
                            &mut col,
                            120,
                            PREFIX_BG,
                            15,
                        );
                        black_box(&buf);
                    },
                    criterion::BatchSize::SmallInput,
                )
            },
        );
    }
    group.finish();
}

/// P2: fg()/bg() heap allocation cost per call.
/// After write_fg()/write_bg() refactor, allocations drop to zero.
fn bench_fg_bg_allocations(c: &mut Criterion) {
    c.bench_function("fg_100_calls", |b| {
        b.iter(|| {
            for i in 0u8..100 {
                black_box(fg(i, 128, 255 - i));
            }
        })
    });
    c.bench_function("bg_100_calls", |b| {
        b.iter(|| {
            for i in 0u8..100 {
                black_box(bg(i, 128, 255 - i));
            }
        })
    });
    c.bench_function("arrow_100_calls", |b| {
        b.iter(|| {
            let mut buf = String::with_capacity(4096);
            let mut col = 0usize;
            for _ in 0..100 {
                arrow(&mut buf, &mut col, (255, 0, 0), BAR_BG);
            }
            black_box(&buf);
        })
    });
}

criterion_group!(
    benches,
    bench_format_elapsed,
    bench_activity_style,
    bench_mode_style,
    bench_render_status_bar,
    bench_render_tabs_scaling,
    bench_fg_bg_allocations
);
criterion_main!(benches);
