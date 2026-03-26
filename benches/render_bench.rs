use criterion::{black_box, criterion_group, criterion_main, Criterion};
use zellaude::render::{activity_priority, activity_style, format_elapsed, mode_style};
use zellaude::state::Activity;
use zellij_tile::prelude::InputMode;

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

criterion_group!(benches, bench_format_elapsed, bench_activity_style, bench_mode_style);
criterion_main!(benches);
