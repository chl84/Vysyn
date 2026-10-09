# Vysyn's stable winit patch

This directory contains the source distribution of **winit 0.30.13**, downloaded
from crates.io, with the original Apache-2.0 LICENSE retained. Vysyn selects it
with `[patch.crates-io]`, so builds do not depend on an unpublished branch or a
network download outside Cargo's normal dependency resolution. The original
crate archive SHA-256 is
`a6755fa58a9f8350bd1e472d4c3fcc25f824ec358933bba33306d0b63df5978d`.

Upstream: https://github.com/rust-windowing/winit/tree/v0.30.13

The stable Wayland backend does not create `wl_data_device` or emit
`WindowEvent::DroppedFile`. This patch implements receiving local file/directory
offers using the existing Smithay Client Toolkit and calloop dependencies.
It does not upgrade Vysyn to the unstable winit 0.31 API or change X11/Windows.

Modified upstream files:

* `Cargo.toml` and `Cargo.toml.orig`: enable rustix's `fs` feature for nonblocking
  pipe flags. No dependency version is changed.
* `src/platform_impl/linux/wayland/state.rs`: bind the optional data-device
  manager and retain one bounded pending transfer.
* `src/platform_impl/linux/wayland/seat/mod.rs`: retain a receiving data device
  for initial and newly added seats.

Added files:

* `src/platform_impl/linux/wayland/seat/data_device.rs`: COPY-only requests,
  nonblocking pipe reads (at most 64 KiB per callback), a 1 MiB total cap,
  a five-second timeout, cleanup and standard `DroppedFile` events. New code
  forbids unsafe Rust. Unsupported MOVE-only offers are rejected.
* `src/platform_impl/linux/wayland/seat/drop_paths.rs`: bounded local URI-list
  parsing with percent decoding and support for Unix filename bytes.

The receiving path requests COPY and does not modify source files. Hyprland
0.56.2 ignores receiver action negotiation and reports MOVE for COPY|MOVE
sources; receiving must not depend on a later COPY action event. This behavior
was checked against its data-device source and actual protocol traces. Production
Browsey and Nautilus retained their source files during native drag tests.
See `docs/testing.md` and `docs/limitations.md` in the Vysyn repository.

When upgrading winit, compare these files with the published source, rerun the
URI-list tests plus native Browsey/Nautilus drops, and remove this patch once a
compatible stable release supplies Wayland file receiving. Retain winit's license
and this modification notice in distribution bundles.

Primary references:

* https://github.com/rust-windowing/winit/blob/v0.30.13/src/platform_impl/linux/wayland/state.rs
* https://docs.rs/smithay-client-toolkit/0.19.2/smithay_client_toolkit/data_device_manager/index.html
* https://raw.githubusercontent.com/hyprwm/Hyprland/efb50993780079460b0cbed1363e2166a2de1d9f/src/protocols/core/DataDevice.cpp
