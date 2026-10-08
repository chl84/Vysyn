# Architecture

The repository was empty when inspected on 2026-10-08. This implementation uses
Rust 2024, winit 0.30, wgpu 30, image 0.25, resvg 0.48, libheif-rs 3, and
moxcms 0.9. Versions were checked against the Cargo registry; Cargo.lock records
the mutually compatible resolution. libheif-rs uses the 1.20 API baseline for
native security limits, with newer pinned native builds in CI. AVIF uses that
same decoder because image's native AVIF constructor cannot enforce predecode
allocation limits; see limitations.md.

* `app`: winit lifecycle, physical-pixel input, on-demand redraw and GIF clock.
* `render`: one GPU backend at a time, sRGB textures and a linear blending shader.
* `decode`: bounded content detection, image/resvg/libheif decoding, orientation,
  conversion to sRGB and linear premultiplied alpha for filtering.
* `loader`: bounded background jobs, request generations, adjacent preloading,
  and an LRU of shared decoded images. A foreground request supersedes preloads.
* `navigation`: deterministic filename ordering and asynchronous directory scans.
* `view`: fit, cursor-anchored zoom and panning, exclusively in physical pixels.
* `limits`: validated environment configuration and shared allocation accounting.

Create the borderless window, initialize rendering, then submit the initial
decode. Present the first image before scanning its directory or preloading.
Background workers never touch a window or GPU. Static images use winit Wait;
animated images use WaitUntil at the next frame deadline. Only the current frame
occupies GPU image memory, and same-size texture uploads reuse the allocation.

Security takes precedence over showing an oversized image: reject unsafe decode
sizes with a controlled error; downscale an otherwise safe decode if it exceeds
the adapter texture or configured GPU limits. The RAM budget accounts for shared
images and reserved decoding work, including images evicted while still visible.
Native decoder internals and driver allocations remain outside exact accounting.
