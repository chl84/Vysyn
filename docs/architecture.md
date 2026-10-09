# Architecture

The repository was empty when inspected on 2026-10-08. This implementation uses
Rust 2024, winit 0.30, wgpu 30, image 0.25, resvg 0.48, libheif-rs 3, and
moxcms 0.9. Versions were checked against the Cargo registry; Cargo.lock records
the mutually compatible resolution. libheif-rs uses the 1.20 API baseline for
native security limits, with newer pinned native builds in CI. AVIF uses that
same decoder because image's native AVIF constructor cannot enforce predecode
allocation limits; see limitations.md.

PNM, TGA, Farbfeld, DDS and Radiance HDR enable dependency-free `image` features;
Cargo.lock and the native codec set are unchanged. Detection reads at most a 4 KiB
prefix and, for plausible TGA headers, a 26-byte footer. Only signatureless TGA
falls back to validated extensions. DDS keeps image's RGB decoder and restores
BC1's missing alpha selector without duplicating color decoding. HDR uses a
bounded, normalized header plus the original pixel stream with the strict decoder,
then maps float linear RGB to SDR before quantizing.

Stable winit 0.30.13 does not implement Wayland file drops. The vendored crate
adds a receiving data device to its existing connection and calloop event loop,
then emits the same `DroppedFile` event used on X11/Windows. URI reads are
nonblocking, bounded and timed out; image decoding still goes through `loader`.
The patch adds no unsafe code and leaves the other window-system backends alone.
See [patch provenance and maintenance](https://github.com/chl84/Vysyn/blob/main/vendor/winit/VYSYN_PATCH.md).

* `app`: winit lifecycle, physical-pixel input, on-demand redraw and GIF clock.
* `render`: one GPU backend at a time, sRGB textures, a byte-bounded texture LRU
  and a linear blending shader.
* `decode`: bounded content detection, image/resvg/libheif decoding, orientation,
  conversion to sRGB and linear premultiplied alpha for filtering.
* `loader`: bounded background jobs, request generations, adjacent preloading,
  and an LRU of shared decoded images. The latest foreground request can adopt
  an active decode of the same path. Memory pressure defers background jobs until
  reservations change; foreground work evicts only needed unpinned LRU buffers.
* `navigation`: deterministic filename ordering and asynchronous directory scans.
* `view`: fit, cursor-anchored zoom and panning, exclusively in physical pixels.
* `limits`: validated environment configuration and shared allocation accounting.

Create the borderless window, initialize rendering, then submit the initial
decode. Present the first image before scanning its directory or preloading.
Background workers never touch a window or GPU. Static images use winit Wait;
animated images use WaitUntil at the next frame deadline. Static images retain
GPU textures within the existing total image-byte budget, avoiding upload on a
GPU cache hit. Weak decoded-image identity keys neither pin CPU buffers nor match
a new decode after file invalidation. Evicted same-size textures can be reused.
Animations use a mutable texture outside the static LRU, counted within the same
budget. Eviction precedes new allocation so retained image payloads remain bounded.

Security takes precedence over showing an oversized image: reject unsafe decode
sizes with a controlled error; downscale an otherwise safe decode if it exceeds
the adapter texture or configured GPU limits. The RAM budget accounts for shared
images and reserved decoding work, including images evicted while still visible.
Native decoder internals and driver allocations remain outside exact accounting.
