# Vendored softbuffer patch

`softbuffer` 0.4.8, verbatim from crates.io except one change.

## The patch

`src/backends/wayland/buffer.rs`, two sites (`Buffer::new`, `Buffer::resize`):

```diff
-            wl_shm::Format::Xrgb8888,
+            wl_shm::Format::Argb8888,
```

`Xrgb8888` has no alpha byte. tiny-skia writes premultiplied RGBA into that
pixmap, so a transparent surface arrived at the compositor as opaque black:
the voice-chat overlay painted an opaque rectangle over the desktop. The
compositor is not at fault, the surface format was.

Upstream bug: rust-windowing/softbuffer#17. No upstream fix at 0.4.8.

## Why it is vendored

Dropping iced's `wgpu` feature cut ~110 MB RSS and ~52% off the release
binaries, but left tiny-skia as the only renderer, which exposed the format
bug above. Keeping `wgpu` restored transparency at the cost of the whole
saving, and only where a Vulkan or GL driver happened to work.

## Refreshing to a newer version

1. Copy the new crate over `vendor/softbuffer/`, keeping `README.md`
   (`src/lib.rs` does `include_str!("../README.md")`, the build fails
   without it) and both LICENSE files.
2. Re-apply the two `Xrgb8888` -> `Argb8888` edits.
3. Diff every vendored file against `~/.cargo/registry/src/*/softbuffer-<v>`.
   Any difference outside `buffer.rs` is an unintended edit.
4. If upstream now requests an alpha-capable format, drop this patch, delete
   `vendor/`, and remove `[patch.crates-io]` from the root `Cargo.toml`.

## The failure mode to remember

Cargo stays silent when a `[patch]` stops matching. Nothing errors; the only
symptom is the opaque black overlay coming back. If the overlay goes black
again with no other change, suspect this file first.