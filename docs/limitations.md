# Known limitations and security boundaries

* Native HEIF/AVIF decoding requires libheif >= 1.20 and working libde265 plus
  dav1d/libaom codecs. CI pins newer native sources. Update native libraries as
  well as Cargo dependencies; Cargo audit does not inspect shared libraries.
* AVIF deliberately uses libheif rather than `image`'s `avif-native` feature:
  image 0.25.10 creates decoded AV1 pictures inside its constructor, before
  caller-supplied pixel/allocation limits can be enforced. libheif lets Vysyn set
  native pixel, block, total-memory, item/tile and profile limits **before** reading
  the container. Only one AVIF decoder is enabled. This is a security-driven
  exception to the preferred decoder choice in agent.md.
* HEIC uses container rotation/mirroring from libheif. Extra EXIF-only HEIF
  orientation is not applied a second time. Conventional JPEG/PNG/WebP/TIFF
  EXIF orientation is applied through image's decoder metadata.
* Output is 8-bit SDR sRGB. RGB/grayscale ICC and supported NCLX/CICP spaces are
  converted; malformed/unsupported profiles fall back to sRGB with a diagnostic.
  CMYK ICC workflows, calibrated monitor profiles, HDR tone mapping and native
  wide-gamut output are not implemented. Wide-gamut values clip to sRGB. The OS
  compositor is responsible for mapping the declared sRGB surface to the monitor.
* SVG support is deliberately bounded: UTF-8 vector shapes, gradients and text;
  no DTDs, `use` expansion, raster image hrefs (including data URIs), filters,
  masks or patterns. External image resolvers return None, so neither files nor
  URLs are loaded. Scripts never execute. Some complex SVGs are rejected or have
  unavailable embedded images omitted. Clip paths and group opacity are supported,
  with conservative offscreen layer reservations. Total SVG text is capped at
  16 KiB before font shaping to bound glyph expansion; fonts load only when text
  is present, and the font database is reused.
* Animated GIFs are fully composited by image and bounded in total bytes/frames.
  Frame rectangles are validated against the canvas before decoding.
  Over-limit animations fail as a whole. Nonzero delays below 10 ms and zero
  delays use a 10 ms minimum. Missing repeat extensions play once; finite repeats
  and infinite loops are preserved. Hidden windows pause uploads and catch up
  when exposed. WebP/APNG/AVIF/HEIF animations are displayed as static images.
* Source images beyond the decode budget are rejected rather than tiled. Safely
  decoded images above GPU limits use a smaller representation and retain their
  original view dimensions. Their 100% zoom cannot restore discarded detail.
* The shared budget controls application image buffers and reserved working sets.
  It is not a hard OS process-memory ceiling: codec scratch allocations, SVG
  fonts/trees, allocator overhead, wgpu staging buffers, swapchains, shared driver
  memory and native-library internals cannot be counted exactly. For hostile files
  requiring a strict process-wide ceiling, use an OS sandbox/resource limit.
  In-process native libraries cannot be made memory-safe by safe Rust wrappers.
* A new foreground request replaces queued preloads and discards obsolete
  results; an active decode of the selected path is reused for the newest request.
  An already-running native decode cannot be forcibly canceled; worker
  count and input limits bound concurrent work. Closing does not wait for it.
* Cached files are checked by size and modification time, then checked again
  after decoding. A deliberate same-size rewrite preserving timestamps can evade
  cache invalidation. Directories refresh after failed opens, or by dropping/
  reopening the directory; there is no filesystem watcher for newly added files.
* Errors remain off the image surface: stderr on Linux, `%TEMP%\vysyn.log` on
  Windows. A failed navigation retains the previous image. A failed initial
  image leaves the black drop target open.
* Wayland drops accept local `text/uri-list` file URLs, including directories,
  escaped names and Unix filename bytes. Remote URLs and portal-only offers
  are unsupported. A transfer is limited to 1 MiB, 1,024 paths and five seconds;
  only one transfer is active at a time. COPY is requested; MOVE-only offers
  are rejected. Vysyn never moves or deletes source files. Compositor/source
  action handling remains outside the viewer: Hyprland 0.56.2 reports MOVE for
  ordinary COPY|MOVE offers despite the receiver's COPY request. Actual Browsey
  and Nautilus tests verified retained source files; see testing.md.
* GPU device loss or out-of-memory is a controlled error/exit. Backend fallback
  handles initialization failure; it does not live-migrate a lost device.
* Windows graphical behavior, mixed-DPI monitor moves and file-manager integration
  require manual validation; see testing.md. Available local checks cannot certify
  a production release on all drivers and platforms.

Distribution bundles must retain codec copyright/license notices. libheif and
libde265 include LGPL requirements; distribution of dynamic libraries requires
their notices and corresponding source/relinking rights. The pinned Windows
manifest disables encoder defaults; distro libheif builds may contain additional
codecs with different licenses. Review the actual bundled native dependencies.

Primary references:

* [image AVIF decoder source](https://docs.rs/image/0.25.10/src/image/codecs/avif/decoder.rs.html)
* [libheif security limits](https://docs.rs/libheif-rs/3.0.0/libheif_rs/struct.SecurityLimits.html)
* [resvg/usvg resolver](https://docs.rs/usvg/0.48.1/usvg/struct.ImageHrefResolver.html)
