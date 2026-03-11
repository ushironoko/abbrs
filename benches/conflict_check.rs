use abbrs::config::Abbreviation;
use abbrs::conflict;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::path::PathBuf;

fn generate_abbreviations(count: usize) -> Vec<Abbreviation> {
    (0..count)
        .map(|i| Abbreviation {
            keyword: format!("abbr{}", i),
            expansion: format!("expanded {}", i),
            ..Default::default()
        })
        .collect()
}

fn generate_path_commands(count: usize) -> Vec<(String, PathBuf)> {
    (0..count)
        .map(|i| {
            (
                format!("cmd{}", i),
                PathBuf::from(format!("/usr/bin/cmd{}", i)),
            )
        })
        .collect()
}

fn bench_conflict_detection(c: &mut Criterion) {
    let abbrs = generate_abbreviations(100);
    let path_cmds = generate_path_commands(2000);

    c.bench_function("conflict_detection_100x2000", |b| {
        b.iter(|| {
            conflict::detect_conflicts(
                black_box(&abbrs),
                black_box(&path_cmds),
            )
        });
    });
}

fn bench_path_scan(c: &mut Criterion) {
    c.bench_function("path_scan", |b| {
        b.iter(|| conflict::scan_path());
    });
}

criterion_group!(benches, bench_conflict_detection, bench_path_scan);
criterion_main!(benches);
