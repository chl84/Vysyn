# Release performance measurements

Measured 2026-10-09 on an AMD Ryzen 5 7520U (4 cores / 8 threads), Radeon 610M
(RADV/radeonsi), approximately 15 GiB RAM, Linux 7.2.5-3-omarchy, Hyprland/Wayland
and Xwayland, Rust 1.98.0, libheif 1.23.4. Release uses opt-level 3, no LTO,
generic CPU code generation and stripped debug information. Viewer binary: about
19 MiB; benchmark executable: about 8.6 MiB. The initial release build took
4 min 10 s with two Cargo jobs; incremental application builds took about 13 s.

## Startup, navigation and resource use

Ten desktop launches of the 1920x1080 synthetic PNG with warm filesystem/driver
caches gave **80.58 ms median** from application initialization to first image
presentation submission (75.38–85.46 ms range). The earlier first observed launch
took 178.83 ms; it is not a controlled cold-cache benchmark. Window creation took
roughly 5–9 ms during the ten-run measurement; GPU readiness 37–47 ms.
The trace clock excludes dynamic loader work before Rust starts. Presentation
submission is not a photon/display-scanout measurement: compositor and refresh
latency follow. A separate window pixel capture confirmed that rendering reached
the desktop.

An actual X11 next/previous navigation check, after adjacent preloads completed,
reported 2.07 ms and 4.40 ms from key request to presentation submission. Cache
lookups in the microbenchmark took roughly 0.02–0.04 microseconds. These are
different metrics: a lookup excludes filesystem validation, GPU upload and present.
When a trace says `cached=true`, `decode_ms` is that image's earlier decoding
duration; `loaded_ms` measures the current cache/metadata servicing time.

Static-image measurement after both neighbors loaded:

| Metric | Observed |
|---|---:|
| Process resident RAM | 79.11 MiB |
| Process resident high-water mark | 79.11 MiB |
| CPU during a 2-second idle interval | 0.0% of one core |
| Current image GPU payload | 7.91 MiB |
| AMD driver-reported VRAM for the process | 13.92 MiB |
| AMD driver-reported GTT for the process | 41.98 MiB |

RSS includes fonts, caches, decoder/driver code and allocator overhead. Driver
memory includes more than the image texture; VRAM and GTT values must not be
interpreted as a universal GPU-cache total. Idle CPU was read from `/proc` CPU
ticks and has the sampling resolution of that interface; 0.0% means no additional
ticks were observed during this interval.

## Decoding

`vysyn-bench` performs eleven decodes, drops the first as warmup, and reports ten
warm samples. Timings include file read, parsing, orientation, color conversion
and any size adaptation. They exclude GPU upload/presentation. Synthetic fixtures
have different dimensions and content complexity; this table is not a codec
ranking or a guarantee for camera images. Decode and desktop measurements ran
sequentially; normal background activity can still affect results.

| Format | Dimensions / frames | Median ms | P95 ms |
|---|---|---:|---:|
| JPEG | 1920x1080 / 1 | 17.249 | 20.271 |
| PNG | 1920x1080 / 1 | 28.866 | 30.689 |
| WebP | 1920x1080 / 1 | 10.464 | 11.849 |
| GIF | 320x240 / 3 | 0.850 | 1.246 |
| BMP | 1920x1080 / 1 | 8.697 | 10.331 |
| TIFF | 1920x1080 / 1 | 5.706 | 7.712 |
| ICO | 256x144 / 1 | 0.294 | 0.374 |
| SVG | 1920x1080 / 1 | 14.812 | 15.880 |
| HEIC | 48x64 / 1 | 0.170 | 0.267 |
| AVIF | 64x48 / 1 | 0.708 | 0.846 |

Measured changes justified two straightforward lifetime improvements: reusing the
SVG font database, then avoiding font loading for SVGs without text, reduced this
SVG's warm decode from about 120 ms to about 15 ms;
retaining one lazily initialized libheif guard reduced repeated native fixture
decodes from about 10–13 ms to sub-millisecond times. Ordinary sRGB NCLX images
also skip a redundant CMS conversion. No Rayon, tiling or aggressive compiler
optimizations were added.

## Reproduce

```sh
cargo build --locked --release --bins --examples
target/release/examples/generate_samples
target/release/vysyn-bench artifacts/bench-images/*
python3 scripts/measure-desktop.py
python3 scripts/desktop-check.py
```

See [raw decode data](benchmarks/decode.tsv),
[startup/resource data](benchmarks/desktop-performance.json), and
[navigation trace](benchmarks/navigation.txt). Run comparable fixtures and record
OS, compiler, native codec versions and GPU/driver for meaningful comparisons.
Windows performance, controlled cold starts, 4K/large camera corpora, and other
GPU drivers have not been measured here.
