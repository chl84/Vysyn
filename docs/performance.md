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

## Native Wayland navigation with 12 MP camera JPEGs

Measured on 2026-10-09 using the installed Release Vysyn binary at commit
`9b622c7be9c67924d2ccda2ec54e0e42ec9fbe32` and installed imv 5.0.1-2
(the desktop's **Image Viewer**). This uses the machine described above, native
Wayland, the Radeon 610M Vulkan backend for Vysyn, and a 1920x1080 display at
59.997 Hz. Both measurement windows were fullscreen with the same viewport.
Four private JPEGs each have 4000x3000 source pixels; two require EXIF rotation.
Filesystem data was warmed equally before each set of trials. Each scenario
started a fresh viewer process, allowed 1.6 seconds for initial loading/window
settling, and sent 16 native Right/Left press-release events. Two repeats reversed
application order, giving **32 requests per scenario and 96 per application**.
Vysyn used its default limits (two workers, 512 MiB RAM budget, 192 MiB CPU cache)
and `--trace` for diagnostic correlation.

Latency runs from `CLOCK_MONOTONIC` immediately before the native virtual-keyboard
press to the compositor's `presentation_time` for the first captured window frame
matching the requested image. A 32x24 RGB sample grid was compared with settled
references captured separately from each application. All accepted matches had
zero sample error. Only owned measurement windows were captured and given input.
This verifies image pixels beyond the application's presentation-submission
trace. It is a compositor measurement with approximately one 16.7 ms display
frame of resolution, without a photodiode or physical scanout measurement.
Frame receipt/readback followed its timestamp by roughly 2.3–5.2 ms in the
baseline comparison and is excluded from the primary latency.

| Application | Input sequence | Median ms | P95 ms | Image observed before next key |
|---|---|---:|---:|---:|
| imv | Right every 1000 ms | 131.14 | 148.05 | 32/32 |
| Vysyn default | Right every 1000 ms | 37.85 | 304.51 | 32/32 |
| imv | Right every 250 ms | 120.52 | 137.35 | 32/32 |
| Vysyn default | Right every 250 ms | 35.46 | 201.83 | 23/32 |
| imv | Alternating Right/Left every 350 ms | 121.99 | 133.93 | 32/32 |
| Vysyn default | Alternating Right/Left every 350 ms | 210.98 | 304.44 | 27/32 |

**Medians and P95 include only requests whose target image was observed before
the next key.** Unmatched requests receive no artificial latency; the final
request gets another second of observation. Consequently, the 35 ms fast-browse
median does not represent all requests: nine targets were not observed within
their input interval. The fast default Vysyn runs also differed substantially:
8/16 matches with a 191.35 ms median, then 15/16 with a 30.40 ms median. imv
observed every target in both repeats.

Default Vysyn trace correlation showed 28.8 ms median for the 32 visible cache
hits and 224.9 ms for the 50 visible requests needing fresh decoding. Even at
1000 ms intervals, only 17/32 requests hit the cache; none of the 27 observed
alternating requests did. These findings locate the main inconsistency in
preloading/cache retention, rather than the cost of an already cached switch.

Two controls used the identical installed binary with a temporary environment
change confined to the measurement child process. All other limits stayed at
their defaults; neither change was installed permanently.

| Temporary Vysyn setting | Input sequence | Median ms | P95 ms | Image observed before next key |
|---|---|---:|---:|---:|
| `VYSYN_RAM_MIB=1024` | Right every 1000 ms | 29.60 | 35.98 | 32/32 |
| `VYSYN_RAM_MIB=1024` | Right every 250 ms | 29.63 | 30.90 | 30/32 |
| `VYSYN_RAM_MIB=1024` | Alternating Right/Left every 350 ms | 24.29 | 29.53 | 32/32 |
| `VYSYN_WORKERS=1` | Right every 1000 ms | 30.35 | 59.99 | 32/32 |
| `VYSYN_WORKERS=1` | Right every 250 ms | 30.35 | 46.73 | 30/32 |
| `VYSYN_WORKERS=1` | Alternating Right/Left every 350 ms | 24.16 | 29.72 | 32/32 |

Both controls retained the cache effectively: all 94 observed targets per control
were cache hits. Each fresh fast-browse process still missed the first request for
the initially non-neighbor image before the next 250 ms key. Thus these controls
demonstrate the cache-retention improvement, not a complete solution for every
rapid-browse case. End-of-scenario resident RAM ranged from 144–149 MiB for imv,
158–250 MiB for default Vysyn, 239–248 MiB for the 1024 MiB control, and 232–241 MiB
for the single-worker control. These are RSS samples, not peak memory or budget
reservations.

The baseline code supplies a consistent explanation: each 12 MP decode conservatively
reserves about 282.7 MiB of work space (`6 * RGBA bytes + 8 MiB`). Two such
reservations plus the current 45.8 MiB image require about 611 MiB, already beyond
the default 512 MiB budget before encoded input buffers. A budget failure cleared
the decoded cache and retried. The two independent controls strongly implicate
this interaction between concurrent preloads and the memory budget. Promotion or
cancellation of an in-flight preload can still affect rapid navigation.

The apps retain their own rendering behavior: captured references show Vysyn
applying the two portrait EXIF orientations while this installed imv displays
those files in landscape. Color/orientation pipelines are therefore not
identical. This comparison measures the installed apps' actual user-visible
behavior on four files, and does not establish a general decoder or GPU ranking.
Normal background activity and capture overhead can affect the measurements.

All **384 individual requests** and the environment/method summary are available
as [anonymized CSV](benchmarks/navigation-comparison.csv) and
[summary JSON](benchmarks/navigation-comparison.json). Photo hashes were checked
before and after; all four inputs were unchanged. Captured photo samples,
filenames, window identifiers and absolute input timestamps remain in ignored
local artifacts and are absent from these exported results.

## Navigation after memory scheduling and GPU caching

The three follow-up changes retain the existing defaults: 512 MiB shared image
RAM, 192 MiB CPU cache, 128 MiB total GPU image payload, and two workers.

* Background memory pressure defers a job until reservations change, preserving
  cached neighbors. Foreground work releases only needed unpinned LRU entries.
  Parallel small decodes still run when their actual reservations fit.
* An active preload can satisfy the latest foreground request for its path.
  Obsolete results cannot replace the latest selection. This avoids restarting
  the selected preload when navigation changes the request generation.
* Static textures use a byte-bounded GPU LRU with weak decoded-image identity
  keys. A GPU hit needs no pixel upload; a new decode after file invalidation
  cannot match an old key. Evicted textures of the same size can be reused.
  GIFs use a separate mutable texture, counted within the same GPU budget.

Measured again on 2026-10-09 with the identical four private JPEGs, settled pixel
references, fullscreen viewport, capture/key helpers and input schedules described
above. The old installed Release binary and new bundled Release binary ran
sequentially; their order was reversed on the second repeat. The source and
binary fingerprints are recorded in the summary JSON. This is a fresh paired
comparison, so its baseline differs from the earlier runs of the variable old
implementation. Each row combines 32 requests from two fresh processes.

| Input sequence | Version | Median ms | P95 ms | Image observed before next key |
|---|---|---:|---:|---:|
| Right every 1000 ms | Before | 183.65 | 305.91 | 32/32 |
| Right every 1000 ms | After | 34.45 | 37.95 | 32/32 |
| Right every 250 ms | Before | 198.10 | 322.31 | 16/32 |
| Right every 250 ms | After | 35.73 | 52.11 | 32/32 |
| Alternating Right/Left every 350 ms | Before | 199.26 | 346.04 | 29/32 |
| Alternating Right/Left every 350 ms | After | 5.57 | 20.44 | 32/32 |

Timing statistics remain conditional on observing the target before the next key;
the final request has one extra second. Both versions matched the original
settled image samples exactly. The new version observed **96/96 targets**, with
94 CPU cache hits and two requests adopted from ongoing neighbor preloads.
The alternating runs each had 15/16 GPU cache hits; the initial next image still
needed an upload. Cycling all four 12 MP images does not fit the 128 MiB GPU cache
(each is 45.8 MiB), so those runs still upload pixels, but retain the decoded CPU
images reliably. GPU image payload peaked at 91.55 MiB in these traces.

End-of-scenario RSS was 249.5–251.8 MiB after the change, versus 158.6–241.7 MiB
before. The higher resident memory reflects retaining decoded images instead of
repeatedly losing the cache; the configured RAM/GPU limits did not increase.
These are sampled process RSS values and payload accounting, not peak driver/OS
memory. A 5.6 ms captured compositor latency is not a 5.6 ms photon/scanout claim:
the screen still runs at 60 Hz, with approximately 16.7 ms frame resolution.

Validation: all 37 Rust unit/integration tests passed, `cargo clippy --locked
--all-targets -- -D warnings` passed, formatting passed, and the Release binaries
and examples built. Native Wayland pixel checks also verified next/previous
restoration, synthetic file modification invalidating old GPU pixels, all three
GIF frame colors, restoration of a static image after GIF navigation, and GPU
budget enforcement at both 128 MiB and **1 MiB**. These desktop checks captured
only owned test windows and modified only generated fixtures. The user JPEG
hashes remained unchanged. Windows desktop behavior has not been exercised here.

See [192 anonymized before/after requests](benchmarks/navigation-optimized.csv)
and [method, fingerprints and functional checks](benchmarks/navigation-optimized.json).

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

## PSD composite decoding

Measured on 2026-10-10 with the same Linux/Ryzen environment and Release settings.
The 1920x1080 samples are generated RGB gradients with RAW planar composites and
no ICC profile. Eleven sequential decodes per file discard the first as warmup;
the remaining ten include reading, parsing and 16-to-8-bit conversion, and exclude
GPU upload/presentation.

| PSD sample | Median ms | P95 ms |
|---|---:|---:|
| RGB 8-bit, 1920x1080 | 7.854 | 10.084 |
| RGB 16-bit, 1920x1080 | 16.584 | 20.701 |

Small profiled fixtures demonstrate fixed CMS setup costs: the 5x5 RGB 16-bit
fixture took 9.303 ms, and 4x4 grayscale 16-bit took 9.472 ms. Source precision is
retained through ICC conversion; profile transforms are created per decode.
These samples do not represent large professional layered documents or rank PSD
against unrelated JPEG/PNG content. Skipping layer pixels avoids loading/rendering
them; metadata, the composite and color conversion still cost time.

A separate bundled benchmark process decoding both large samples eleven times
each reached **39.98 MiB peak RSS**, measured with Linux `getrusage`. It includes
the previous iteration's retained image and excludes GUI/GPU memory. The final
RGBA8 payload is 7.91 MiB for either depth. Application reservations and process
RSS measure different things; neither is a hard ceiling for the whole desktop.

See [all nine warm decode results](benchmarks/psd-decode.tsv),
[benchmark process resources](benchmarks/psd-resources.json) and
[actual Wayland pixel/navigation checks](benchmarks/psd-presentation.json).

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
Windows performance, controlled cold starts, larger or more varied camera
corpora, and other GPU drivers have not been measured here.
