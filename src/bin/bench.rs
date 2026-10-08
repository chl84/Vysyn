use anyhow::{Result, ensure};
use std::{path::PathBuf, time::Instant};
use vysyn::{
    cache::Cache,
    decode::{self, Target},
    limits::{Budget, Limits},
    navigation::FileKey,
};

fn main() -> Result<()> {
    let files: Vec<_> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    ensure!(
        !files.is_empty(),
        "usage: vysyn-bench IMAGE... (use a release build)"
    );
    let limits = Limits::from_env()?;
    let target = Target {
        max_dimension: 8192,
        gpu_bytes: limits.gpu_bytes,
    };
    println!(
        "file\tformat\twidth\theight\tframes\tdecode_median_ms\tdecode_p95_ms\tcache_lookup_us\timage_bytes"
    );
    for path in files {
        let mut samples = Vec::new();
        let mut last = None;
        for _ in 0..11 {
            let budget = Budget::new(limits.ram_bytes);
            let start = Instant::now();
            let image = decode::decode(&path, &limits, &budget, target, &|| false)?;
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
            last = Some(image);
        }
        samples.remove(0);
        samples.sort_by(f64::total_cmp);
        let image = std::sync::Arc::new(last.expect("samples are nonempty"));
        let key = FileKey::read(&path)?;
        let mut cache = Cache::new(limits.cache_bytes);
        cache.insert(key.clone(), image.clone(), image.bytes);
        let start = Instant::now();
        for _ in 0..10_000 {
            std::hint::black_box(cache.get(&key));
        }
        let lookup = start.elapsed().as_secs_f64() * 1e6 / 10_000.0;
        println!(
            "{}\t{:?}\t{}\t{}\t{}\t{:.3}\t{:.3}\t{lookup:.3}\t{}",
            path.display(),
            decode::detect_file(&path)?,
            image.original[0],
            image.original[1],
            image.frames.len(),
            (samples[4] + samples[5]) / 2.0,
            samples[9],
            image.bytes
        );
    }
    Ok(())
}
