use kort::cache;
use kort::config;
use kort::matcher;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use tempfile::TempDir;

fn generate_toml(count: usize) -> String {
    let mut toml = String::from("[settings]\n\n");
    for i in 0..count {
        toml.push_str(&format!(
            "[[abbr]]\nkeyword = \"abbr{}\"\nexpansion = \"expanded command {} with args\"\n\n",
            i, i
        ));
    }
    toml
}

fn bench_toml_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("config_parse");

    for size in [10, 100, 500] {
        let toml = generate_toml(size);
        group.bench_with_input(BenchmarkId::new("toml", size), &toml, |b, toml| {
            b.iter(|| config::parse(black_box(toml)).unwrap());
        });
    }

    group.finish();
}

fn bench_cache_read(c: &mut Criterion) {
    let mut group = c.benchmark_group("cache_read");

    for size in [10, 100, 500] {
        let dir = TempDir::new().unwrap();
        let toml = generate_toml(size);
        let config_path = dir.path().join("kort.toml");
        std::fs::write(&config_path, &toml).unwrap();

        let cfg = config::parse(&toml).unwrap();
        let m = matcher::build(&cfg.abbr);
        let cache_path = dir.path().join("kort.cache");
        let settings = cache::CachedSettings::default();
        cache::write(&cache_path, &m, &settings, &config_path).unwrap();

        group.bench_with_input(BenchmarkId::new("bitcode", size), &cache_path, |b, path| {
            b.iter(|| cache::read(black_box(path)).unwrap());
        });
    }

    group.finish();
}

criterion_group!(benches, bench_toml_parse, bench_cache_read);
criterion_main!(benches);
