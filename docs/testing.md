# Validation

Validated locally on 2026-10-08/09 (Europe/Oslo), Linux x86_64, Rust 1.98.0,
libheif 1.23.4, AMD Radeon 610M, Hyprland/Wayland and Xwayland.

## Automated checks

* `cargo fmt --all --check`: passed.
* `cargo clippy --locked --all-targets -- -D warnings`: passed.
* `cargo test --locked --all-targets`: 25 tests passed (17 unit, 8 integration).
* `cargo build --locked --release --bins --examples`: passed.
* `cargo audit`: no RustSec vulnerability findings in the locked dependencies.
  Advisory DB retrieval succeeded; the secondary registry version-info refresh
  warned. This does not audit native codecs or certify absence of vulnerabilities.
* Workflow YAML, vcpkg manifest, Bash and Python syntax: validated locally.

Tests cover header sniffing and corrupt/truncated files, seven raster formats
with extensionless filenames, bounded dimensions/memory, HEIC and AVIF native
limits, HEIF container rotation, JPEG EXIF rotation, PNG ICC conversion,
transparency, GIF composition/delays/repeats, safe SVG resolution, directory
ordering/wrapping/deletion positions, cache eviction/recency, allocation lifetime,
cursor-anchored zoom/panning, HiDPI physical-pixel sizing and one-to-one texel
alignment, GPU backend order and thin-image texture budget rounding.

## Actual local graphical checks

Successful presentation smoke tests:

| Window system | Backend | Result |
|---|---|---|
| Wayland | Vulkan / RADV | Image presented |
| Wayland | GLES / radeonsi | Image presented |
| Wayland | Vulkan unavailable, automatic GLES fallback | Image presented |
| Xwayland (X11 protocol) | Vulkan / RADV | Image presented |
| Wayland | Vulkan, animated GIF | Presented and ran without error |
| Wayland | Vulkan, HEIC and AVIF | Images presented |

The fallback case set `VK_DRIVER_FILES` to a nonexistent ICD path and verified
that the trace reported `backend=Gl`. This exercises actual initialization failure,
not only the unit-tested preference list.

`scripts/desktop-check.py` uses X11/XTest, targets a viewer process it launches,
and captures pixels from that process's window. It verified actual nonblack image
pixels, keyboard zoom, fit restoration, cursor-wheel zoom, dragging/panning,
next/previous navigation, fullscreen geometry changes and Esc. The captured PNG
was visually inspected: a centered gradient, preserved aspect ratio and black
letterboxing, with no controls. The script requires an X11/Xwayland desktop,
`xprop`, libX11 and libXtst. It may briefly focus its own test window.

```sh
cargo run --locked --release --example generate_samples
python3 scripts/desktop-check.py
python3 scripts/measure-desktop.py
```

Raw logs, screenshot and measurements are generated in `artifacts/`; selected
results are retained under `docs/benchmarks/`. The Linux bundle was also tested
for native HEIC/AVIF decoding using its own launcher and bundled libraries.

## CI and remaining manual checks

The supplied GitHub Actions workflow builds/tests Linux and Windows and produces
archives. Linux CI uses Xvfb/software drivers for Vulkan and GLES presentation.
The workflow has not been run remotely from this workspace. No Windows compiler
or Windows desktop was available locally: **Windows compilation and graphical
operation are not claimed as locally verified**. A Windows CI build is a
compilation/non-graphical test, not a desktop functional test.

Before a cross-platform release, verify on real Windows 10/11: bundled DLL loading,
DX12/Vulkan/GLES fallback, no console, fullscreen, drag/drop and Open-with
registration. Verify Wayland fractional scaling and moving between monitors with
different scale factors on both platforms. Test a standalone Xorg desktop in
addition to the tested Xwayland session. Inspect animation disposal/timing against
an independent reference player and color reproduction on calibrated displays.
Broader real camera/ICC/large-image corpora and additional driver models remain
release validation work; the synthetic tests do not cover every codec variation.
