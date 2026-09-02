//! The `Tray` port: what a system-tray backend must provide so the front's
//! shared poll loop can drive it.
//!
//! The port lives *package-local* (not in `hyprlay-core`) because it is
//! naturally coupled to the tray front's own model types: [`menu::TrayState`]
//! and [`icon::IconData`]. `IconData` is produced by [`icon::load_icon`],
//! which decodes an embedded PNG through the `image` crate — pulling that
//! into the framework-free core would drag a codec crate into the core
//! contract. The port itself is GUI-free, so keeping it next to the front it
//! serves is the cleanest seam.

use std::future::Future;

use crate::tray::menu::TrayState;

/// A registered system-tray backend.
///
/// The front owns exactly one `Tray`. It builds it with the action channel
/// the backend uses to report menu activations, then drives it from the
/// shared poll loop: pushing diff-gated [`TrayState`] snapshots and tearing
/// it down on quit. The backend hides all of its platform-specific plumbing —
/// for `ksni` that is a D-Bus `StatusNotifierItem` on the current-thread
/// runtime; for `tray-icon` it is an OS tray icon that must live on the main
/// thread so the menu/icon/menu-update calls land there.
pub trait Tray {
    /// Render `state` (icon + tooltip + menu) on the backend. The caller
    /// diff-gates, so an unchanged snapshot is never delivered here.
    fn update(&mut self, state: &TrayState) -> impl Future<Output = ()>;

    /// Tear the backend down (unregister the tray icon). Dropping the backend
    /// afterwards is harmless.
    fn shutdown(&mut self) -> impl Future<Output = ()>;
}
