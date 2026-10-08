#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
use anyhow::{Result, bail};
use std::{path::PathBuf, time::Duration};
use vysyn::{
    app::{self, Options},
    limits::Limits,
};

fn main() {
    if let Err(error) = start() {
        app::report(&format!("{error:#}"));
        std::process::exit(1);
    }
}

fn start() -> Result<()> {
    let mut path = None;
    let mut trace = std::env::var_os("VYSYN_TRACE").is_some();
    let mut exit_after = None;
    let mut args = std::env::args_os().skip(1);
    let mut literal = false;
    while let Some(arg) = args.next() {
        if !literal {
            match arg.to_str() {
                Some("--") => {
                    literal = true;
                    continue;
                }
                Some("--help" | "-h") => {
                    println!(
                        "Usage: vysyn [--trace] [--smoke-ms MILLISECONDS] [--] [IMAGE|DIRECTORY]\n\nWheel: zoom; left drag: pan; arrows: navigate; +/-: zoom; 0: fit; F11: fullscreen; Esc: close."
                    );
                    return Ok(());
                }
                Some("--version") => {
                    println!("vysyn {}", env!("CARGO_PKG_VERSION"));
                    return Ok(());
                }
                Some("--trace") => {
                    trace = true;
                    continue;
                }
                Some("--smoke-ms") => {
                    let ms: u64 = args
                        .next()
                        .and_then(|s| s.to_str().map(str::to_owned))
                        .ok_or_else(|| anyhow::anyhow!("--smoke-ms needs a duration"))?
                        .parse()?;
                    anyhow::ensure!(
                        (1..=600_000).contains(&ms),
                        "smoke duration must be 1–600000 ms"
                    );
                    exit_after = Some(Duration::from_millis(ms));
                    continue;
                }
                Some(s) if s.starts_with('-') => {
                    bail!("unknown option {s}; use -- before filenames starting with '-'")
                }
                _ => {}
            }
        }
        if path.is_some() {
            bail!("open one image or directory at a time");
        }
        let p = PathBuf::from(arg);
        path = Some(if p.is_absolute() {
            p
        } else {
            std::env::current_dir()?.join(p)
        });
    }
    app::run(
        Options {
            path,
            trace,
            exit_after,
        },
        Limits::from_env()?,
    )
}
