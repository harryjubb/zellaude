use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use zellaude::state::{Activity, SessionInfo, Settings, State};

fn bench_cleanup_stale_sessions(c: &mut Criterion) {
    let mut group = c.benchmark_group("cleanup_stale_sessions");
    for n in [10, 100, 500] {
        group.bench_with_input(BenchmarkId::new("sessions", n), &n, |b, &n| {
            b.iter_batched(
                || {
                    let mut state = State::default();
                    for i in 0..n {
                        state.sessions.insert(
                            i,
                            SessionInfo {
                                session_id: format!("s{i}"),
                                pane_id: i,
                                activity: if i % 3 == 0 {
                                    Activity::Done
                                } else {
                                    Activity::Thinking
                                },
                                tab_name: None,
                                tab_index: None,
                                last_event_ts: 1, // very old
                                cwd: None,
                            },
                        );
                    }
                    state
                },
                |mut state| {
                    black_box(state.cleanup_stale_sessions());
                },
                criterion::BatchSize::SmallInput,
            )
        });
    }
    group.finish();
}

fn bench_cleanup_expired_flashes(c: &mut Criterion) {
    let mut group = c.benchmark_group("cleanup_expired_flashes");
    for n in [10, 100, 500] {
        group.bench_with_input(BenchmarkId::new("flashes", n), &n, |b, &n| {
            b.iter_batched(
                || {
                    let mut state = State::default();
                    for i in 0..n {
                        // Half expired, half active
                        let deadline = if i % 2 == 0 { 1 } else { u64::MAX };
                        state.flash_deadlines.insert(i, deadline);
                    }
                    state
                },
                |mut state| {
                    black_box(state.cleanup_expired_flashes());
                },
                criterion::BatchSize::SmallInput,
            )
        });
    }
    group.finish();
}

fn bench_settings_serde(c: &mut Criterion) {
    let settings = Settings::default();
    let json = serde_json::to_string(&settings).unwrap();

    c.bench_function("settings_serialize", |b| {
        b.iter(|| serde_json::to_string(black_box(&settings)).unwrap())
    });
    c.bench_function("settings_deserialize", |b| {
        b.iter(|| serde_json::from_str::<Settings>(black_box(&json)).unwrap())
    });
}

/// P1: broadcast_sessions serialization cost at various session counts.
/// After "skip empty broadcasts" fix, N=0 is eliminated entirely.
fn bench_session_serialization(c: &mut Criterion) {
    let mut group = c.benchmark_group("session_serialization");
    for n in [0u32, 10, 50] {
        let sessions: std::collections::BTreeMap<u32, SessionInfo> = (0..n)
            .map(|i| {
                (
                    i,
                    SessionInfo {
                        session_id: format!("s{i}"),
                        pane_id: i,
                        activity: Activity::Thinking,
                        tab_name: Some(format!("tab{}", i % 5)),
                        tab_index: Some((i % 5) as usize),
                        last_event_ts: 1700000000 + i as u64,
                        cwd: Some("/tmp".to_string()),
                    },
                )
            })
            .collect();

        group.bench_with_input(
            BenchmarkId::new("sessions", n),
            &sessions,
            |b, sessions| {
                b.iter(|| serde_json::to_string(black_box(sessions)).unwrap())
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_cleanup_stale_sessions,
    bench_cleanup_expired_flashes,
    bench_settings_serde,
    bench_session_serialization
);
criterion_main!(benches);
