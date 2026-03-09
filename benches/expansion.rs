use kort::config::Abbreviation;
use kort::context::RegexCache;
use kort::expand::{expand, ExpandInput};
use kort::matcher;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};

fn generate_abbreviations(count: usize) -> Vec<Abbreviation> {
    (0..count)
        .map(|i| Abbreviation {
            keyword: format!("abbr{}", i),
            expansion: format!("expanded command {} with args", i),
            ..Default::default()
        })
        .collect()
}

fn bench_expansion(c: &mut Criterion) {
    let mut group = c.benchmark_group("expansion");

    for size in [10, 100, 500, 1000] {
        let abbrs = generate_abbreviations(size);
        let m = matcher::build(&abbrs);
        let rc = RegexCache::from_matcher(&m);

        group.bench_with_input(BenchmarkId::new("lookup", size), &(&m, &rc), |b, (m, rc)| {
            let input = ExpandInput {
                lbuffer: format!("abbr{}", size / 2),
                rbuffer: String::new(),
            };
            b.iter(|| expand(black_box(&input), black_box(m), black_box(&[]), black_box(rc)));
        });
    }

    group.finish();
}

fn bench_global_expansion(c: &mut Criterion) {
    let abbrs: Vec<_> = (0..100)
        .map(|i| Abbreviation {
            keyword: format!("G{}", i),
            expansion: format!("global expansion {}", i),
            global: true,
            ..Default::default()
        })
        .collect();

    let m = matcher::build(&abbrs);
    let rc = RegexCache::from_matcher(&m);

    c.bench_function("global_lookup_100", |b| {
        let input = ExpandInput {
            lbuffer: "echo hello G50".to_string(),
            rbuffer: String::new(),
        };
        b.iter(|| expand(black_box(&input), black_box(&m), black_box(&[]), black_box(&rc)));
    });
}

fn bench_placeholder(c: &mut Criterion) {
    let abbrs = vec![Abbreviation {
        keyword: "gc".to_string(),
        expansion: "git commit -m '{{message}}'".to_string(),
        ..Default::default()
    }];

    let m = matcher::build(&abbrs);
    let rc = RegexCache::from_matcher(&m);

    c.bench_function("placeholder_expansion", |b| {
        let input = ExpandInput {
            lbuffer: "gc".to_string(),
            rbuffer: String::new(),
        };
        b.iter(|| expand(black_box(&input), black_box(&m), black_box(&[]), black_box(&rc)));
    });
}

criterion_group!(benches, bench_expansion, bench_global_expansion, bench_placeholder);
criterion_main!(benches);
