# Validation

Validated locally on 2026-10-08–10 (Europe/Oslo), Linux x86_64, Rust 1.98.0,
libheif 1.23.4, AMD Radeon 610M, Hyprland/Wayland and Xwayland.

## Automated checks

* `cargo fmt --all --check`: passed.
* `cargo clippy --locked --all-targets -- -D warnings`: passed.
* `cargo test --locked --all-targets`: 62 tests passed (28 unit, 34 integration).
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
Double-click tests cover nearby click pairs, time/distance limits, triple clicks,
dragging away and back, and long-held buttons.
The ten additional-format tests check P1–P7 pixels, 16-bit Farbfeld transparency,
TGA origins/RLE/palettes/footer detection, DDS BC1/2/3 color/alpha and unsupported
containers, HDR signatures/CRLF/RLE/tone mapping, corrupt files, reservation
release, decoded float-buffer limits and GPU downscaling.

## PSD composites

On 2026-10-10, eleven new PSD tests passed. They cover RGB/grayscale 8/16-bit
RAW, PackBits RLE (literal/run/no-op packets), ZIP and ZIP prediction, planar
ordering and per-row predictor resets, alpha IDs/negative layer counts/additional
transparency tags, saved masks, white matte removal, ICC conversion before
quantization and preserved alpha. Errors cover unsupported modes/PSB, missing
composites, section boundaries, every truncation of the four compression samples,
invalid RLE, ZIP checksum/trailing data/short output/excess expansion, oversized
ICC profiles, allocation limits, cancellation and released reservations.

A file with 200 MiB of unused layer data decoded with a 128-byte inspected-input
allowance and 16 MiB working budget; this verifies seeking over unused sections.
3,072 deterministic mutations of pixel and metadata samples ran under small
limits without panic or leaked reservations. This is bounded mutation testing,
not a sustained coverage-guided fuzzing campaign.

All pixels of seven unmodified, pinned psd-tools fixtures matched independent
ImageMagick/LCMS2 composite references within one code value. The files include
RGB/grayscale ICC, 8/16-bit samples, saved masks, layer adjustments and GIMP
transparency. [Fixture provenance and reproduction](../tests/fixtures/psd/README.md)
are checked in with the license and PNG references. They are not Photoshop exports.

The actual Release viewer passed 14 generated PSD variants on native Wayland
with Vulkan/RADV and the same 14 with GLES/radeonsi. Captured window pixels matched
known colors within two code values, including linear transparency and Display P3/
linear-gray ICC. Arrow navigation, previous/wrapping and GPU cache reuse passed.
The installed launcher passed all 14 with Vulkan. Tests captured and controlled
only self-created, marked windows on an otherwise empty workspace. One initial
GLES capture retained the previous pixels despite a completed presentation trace;
the complete repeated run passed. That intermittent capture/presentation event
was not diagnosed as a PSD decoder error. Logs remain in ignored `artifacts/psd/`.

GIO recognizes PSD as `image/vnd.adobe.photoshop`, and the installed desktop MIME
cache offers Vysyn for it. Windows registration includes `.psd`. PSD adds a direct
use of the already locked pure Rust `flate2`; no package version or native codec
library changed. Windows desktop opening still requires manual validation.

## Double-click fit

On 2026-10-09, actual left-button double-clicks in the Release build restored
the same sampled window pixels as `0` after zoom and pan, on native Wayland
with both Vulkan/RADV and GLES/radeonsi. Clicking the black margin also worked;
small images returned to their original size without upscaling. Single clicks
after short drags, slow/distant clicks and right-button double-clicks did not
fit the image. Tests used only synthetic images and self-created, marked
windows on an otherwise empty workspace. Logs remain under ignored
`artifacts/double-click/`.

The X11/Xwayland Release check also passed: whole-window RGB hashes after
double-click and `0` matched, and ordinary pan/navigation/fullscreen still worked.
The installed launcher passed the same native Wayland click checks with Vulkan.

## Additional image formats

On 2026-10-09, the bundled Release build passed 13 synthetic fixtures on native
Wayland with Vulkan/RADV, and the same 13 with GLES/radeonsi:

| Format group | Fixtures per backend |
|---|---:|
| PBM, PGM, PPM, RGBA PAM | 4 |
| Signatureless TGA 1.0 and renamed TGA 2.0 | 2 |
| 16-bit RGBA Farbfeld | 1 |
| DXT1, DXT3, DXT5, DX10 BC1 with transparency | 4 |
| RADIANCE HDR and RGBE with CRLF headers | 2 |

These checks used only self-created, marked Vysyn windows on an otherwise empty
workspace. Captured window pixels were compared with known RGB values, including
linear-light transparency and HDR highlight compression, with a two-code-value
tolerance. Arrow navigation, directory wrapping/previous and GPU cache reuse
also passed. Fixtures and exact-window capture logs remain under ignored
`artifacts/additional-formats/`. Large 1920x1080 synthetic samples for PBM, PGM,
PPM, PAM, TGA, Farbfeld, HDR and DDS also decoded with the Release benchmark.
The installed launcher passed the same 13 fixtures and navigation/cache checks
with Vulkan. GIO content-type detection identifies all 13 fixtures, and the
desktop MIME cache offers Vysyn for each resulting type. The MIME additions
cover PAM and the `.targa`/`.farbfeld` aliases; legacy PGM/PAM MIME spellings
are also listed in the desktop entry.

The five added `image` features bring no extra dependencies: Cargo.lock and all
ten bundled native library files are unchanged. Linux MIME additions and the
desktop file validate locally; Windows Open-with registration includes the new
extensions but still needs a Windows desktop check.

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
single-click behavior after a drag, double-click matching `0`, next/previous
navigation, fullscreen geometry changes and Esc. The captured PNG
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
No Windows compiler or Windows desktop was available locally: **Windows compilation and graphical
operation are not claimed as locally verified**. A Windows CI build is a
compilation/non-graphical test, not a desktop functional test.

Before a cross-platform release, verify on real Windows 10/11: bundled DLL loading,
DX12/Vulkan/GLES fallback, no console, fullscreen, double-click fit, drag/drop and Open-with
registration. Verify Wayland fractional scaling and moving between monitors with
different scale factors on both platforms. Test a standalone Xorg desktop in
addition to the tested Xwayland session. Inspect animation disposal/timing against
an independent reference player and color reproduction on calibrated displays.
Broader real camera/ICC/large-image corpora and additional driver models remain
release validation work; the synthetic tests do not cover every codec variation.

## Native Wayland file drops

On 2026-10-09, the installed release reproduced the failure: dragging a local
image from the production Browsey application into an empty Vysyn window left
it black. Stable winit 0.30.13 had no receiving Wayland data device, so its
`DroppedFile` handler was never reached.

After the backend patch, actual mouse drags from production Browsey and Nautilus
on Hyprland 0.56.2 passed all eight cases (four per file manager):

* A PNG into an empty viewer.
* A replacement PNG named `blå #? bilde.png`.
* Another replacement image in the same window.
* A directory containing an image.

Both source and viewer windows used native Wayland, with a release build and
Vulkan/RADV on the AMD Radeon 610M. Every case checked the exact dropped path,
decoding/presentation traces, the expected color in captured viewer pixels, and
the continued existence of the source file/directory. All eight cases also passed with the installed bundle. A further real drop
from native Nautilus into an installed X11/Xwayland viewer passed, including
its captured image pixels and retained source. These were real native
gestures, without injecting `DroppedFile` or opening the path on the command line.
Disposable fixtures, logs and screenshots remain under ignored
`artifacts/drag-drop/`.

Hyprland's action event precedes `enter` and reports MOVE when the source offers
COPY|MOVE. Its data-offer implementation ignores the receiver's `set_actions`
request. The patch therefore checks that COPY is offered and requests COPY,
without requiring a subsequent COPY action event. It performs no filesystem
move/delete operation. Browsey and Nautilus retained every original in these
tests. Do not infer the behavior of every compositor or arbitrary drag source
from these checks.

The five URI-list regression tests cover local hosts, comments/CRLF, escaped
spaces/Unicode/delimiters, non-UTF-8 Unix names, malformed/NUL/remote paths,
mixed valid/invalid entries, payload size and file-count limits. Windows drag
handling is unchanged and still requires the manual desktop checks above.
