# Vysyn

A borderless image viewer for Linux and Windows. Only the image, centered on
black. Written in Rust with winit and wgpu; no GUI framework or visible controls.

```sh
vysyn /path/to/image.jpg
vysyn /path/to/directory
```

Without a path, Vysyn opens a black window accepting a dropped image or directory.
Files open according to their contents, including extensionless files. Directory
navigation follows filename order, wraps at either end, and resets the view.
TGA 1.0 needs a recognized TGA extension; TGA 2.0 can also be recognized by its footer.
The previous image stays visible while a replacement decodes in the background.
Local file and directory drops work on native Wayland as well as X11. The
Wayland implementation uses a small patch to stable winit; see
[drag-and-drop validation](docs/testing.md#native-wayland-file-drops).

| Input | Action |
|---|---|
| Mouse wheel | Zoom around the pointer |
| Left button + drag | Pan |
| Drop a file or directory | Open |
| Right / left arrow | Next / previous image |
| `+` / `-` | Zoom around the center |
| `0` | Fit without enlarging small images |
| `F11` | Fullscreen |
| `Esc` | Close |

JPEG, PNG, WebP, GIF (including animation), BMP, TIFF, HEIC/HEIF, AVIF, SVG,
ICO, PNM (PBM/PGM/PPM/PAM), TGA, Farbfeld, DDS (DXT1/3/5 and BC1/2/3), and
Radiance HDR/RGBE are supported. HDR uses fixed-exposure Reinhard tone mapping
to the 8-bit SDR output. EXIF orientation and HEIF rotation are applied automatically.
RGB/grayscale ICC profiles convert to sRGB; transparency is filtered and blended
against black in linear light. Rendering and zoom coordinates use physical
pixels, so 100% zoom maps source pixels to display pixels without a second DPI
scale. See [known limitations](docs/limitations.md) for the safe SVG subset and
color/display limitations.

## Build on Linux

Install Rust 1.90 or newer and libheif **1.20 or newer**, with an HEVC decoder
(libde265) and an AV1 decoder (dav1d or libaom). A working Vulkan driver is
preferred; GLES is the fallback. X11 and Wayland are enabled in the same binary.

On a current Arch Linux system:

```sh
sudo pacman -S --needed rust base-devel pkgconf libheif libde265 dav1d libxkbcommon
cargo build --locked --release --bins
./target/release/vysyn tests/fixtures/native-source.png
```

On Ubuntu 24.04, its system libheif is too old for the security-limit API.
Build the pinned upstream libheif with built-in decoders instead:

```sh
sudo apt-get install build-essential pkg-config cmake ninja-build \
  libde265-dev libdav1d-dev libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev \
  libegl1 libegl-mesa0 libgl1-mesa-dri libgles2 libvulkan1 mesa-vulkan-drivers
bash scripts/build-native-linux.sh
export PKG_CONFIG_PATH="$PWD/.native/install/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
export LD_LIBRARY_PATH="$PWD/.native/install/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
cargo build --locked --release --bins
python3 scripts/bundle-linux.py
```

`artifacts/vysyn-linux-x64.tar.gz` bundles the viewer and native codec libraries.
Extract it and run `vysyn-linux-x64/vysyn`; end users need no Rust/compiler setup.
Desktop libraries and graphics drivers come from the operating system. A package
built on Ubuntu 24.04 requires glibc 2.39 or newer; a locally built package follows
the build host's glibc requirement.

## Build on Windows

Use 64-bit Windows 10/11, Rust's `x86_64-pc-windows-msvc` toolchain, Visual Studio
Build Tools with C++ support, Git, CMake, Ninja and PowerShell. Run:

```powershell
./scripts/build-windows.ps1
```

The script pins vcpkg, installs libheif with libde265 and libaom, runs formatting,
Clippy and tests, builds release binaries, and creates
`artifacts/vysyn-windows-x64.zip`. Extract the whole zip; keep its DLLs beside
`vysyn.exe`. End users need no development tools or separately installed codecs.
The viewer uses the Windows GUI subsystem, so it does not open a console.
Errors go to `%TEMP%\vysyn.log`; Linux errors go to stderr.

## File manager integration

Linux: add the extracted bundle directory to PATH, keeping its launcher, binary
and `lib` directory together, or put a system-linked compiled executable on PATH. Copy
`packaging/vysyn.desktop` to `~/.local/share/applications/`, run
`update-desktop-database ~/.local/share/applications`, then select Vysyn in the
file manager's **Open with** dialog. These instructions do not change the default
application automatically.

For PAM and the `.targa`/`.farbfeld` aliases, also install the supplied MIME additions:

```sh
mkdir -p ~/.local/share/mime/packages
cp packaging/vysyn-mime.xml ~/.local/share/mime/packages/
update-mime-database ~/.local/share/mime
```

In an extracted bundle, the file is named `vysyn-mime.xml` at its top level.

Windows: run the supplied `register-windows.ps1` from the extracted bundle. It
registers Vysyn under the current user's **Open with** choices. Select Vysyn in
Windows Default Apps if desired; administrator access is unnecessary. To remove
registration, delete `HKCU\Software\Classes\Vysyn.Image` and the `Vysyn.Image`
values under each extension's `OpenWithProgids`.

## Resource limits

Settings are integers read from environment variables at startup. Limits must be
positive; `VYSYN_CACHE_MIB=0` disables decoded CPU caching and preloading.

| Variable | Default | Meaning |
|---|---:|---|
| `VYSYN_MAX_PIXELS` | 32000000 | Maximum source pixels |
| `VYSYN_DECODED_MIB` | 128 | Maximum decoded image / total GIF frames |
| `VYSYN_RAM_MIB` | 512 | Shared image and decoding-work budget |
| `VYSYN_CACHE_MIB` | 192 | Decoded LRU cache budget |
| `VYSYN_GPU_MIB` | 128 | Total GPU image texture/cache budget |
| `VYSYN_FILE_MIB` | 64 | Maximum input file size |
| `VYSYN_WORKERS` | 2 | Decoding workers, allowed range 1–4 |

Source dimensions and decoder buffers are checked before pixel allocation.
Temporary work is reserved conservatively (six RGBA buffers plus 8 MiB), so
the practical source limit may be lower than `VYSYN_MAX_PIXELS`. Active images,
preloads and cached images share the same budget. Oversized safe decodes are
downscaled to the adapter's texture limits and the GPU budget. Unsafe source
decodes are rejected. GIFs also have a 512-frame limit. SVG sources are capped at
1 MiB, 10,000 XML nodes and 16 KiB of shaped text. Directory lists are capped at 100,000 entries and
16 MiB of path storage.

Concurrent decoding adapts to the available RAM reservations: background jobs
wait for memory instead of clearing the decoded cache. Selecting an image already
being decoded adopts that job for the latest request. Static GPU textures use a
byte-bounded LRU; animation frames reuse a separate mutable texture counted within
the same GPU image budget. GPU identity keys do not retain CPU pixel buffers.

`VYSYN_BACKEND=vulkan`, `gles`, or (Windows) `dx12` forces a backend for diagnosis.
Without an override, Linux tries Vulkan then GLES; Windows tries DX12, Vulkan,
then GLES. Each backend is initialized only when reached.

## Development and measurements

```sh
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo build --locked --release --bins --examples
cargo run --locked --release --example generate_samples
target/release/vysyn-bench artifacts/bench-images/*
target/release/vysyn --trace --smoke-ms 5000 tests/fixtures/native-source.png
```

The benchmark executable reports decoding and cache lookup times. `--trace`
records window/GPU initialization, presentation submission, navigation latency,
accounted RAM and GPU image bytes. `--smoke-ms` closes automatically and fails if
an expected image was not presented. These diagnostics add no on-screen controls.

CI builds/tests Linux and Windows, packages native libraries, and exercises Linux
Vulkan/GLES presentation under Xvfb. Windows graphical behavior needs a real
desktop session. Read [validation](docs/testing.md),
[performance results](docs/performance.md), and [architecture](docs/architecture.md)
for what was actually verified.

Release uses Rust's standard `opt-level=3` and strips debug information. LTO,
`target-cpu=native`, abort-on-panic and additional parallel image processing are
not enabled. This keeps unwind diagnostics and generic CPU compatibility while
avoiding unmeasured optimization tradeoffs.
