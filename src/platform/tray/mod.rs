//! System-tray backend adapters, selected by target OS.
//!
//! Each adapter implements the [`Tray`](crate::tray::Tray) port and is
//! registered by the tray front's `run()` dispatch:
//! - Linux: `ksni` — a D-Bus `StatusNotifierItem`.
//! - Windows/macOS: `tray-icon` — `Shell_NotifyIcon` / `NSStatusItem`.
//!
//! `tray-icon` is *not* used on Linux: the Linux build keeps `ksni` because
//! `tray-icon`'s Linux backend cannot deliver icon-click events (the
//! libappindicator limitation) and pulls GTK link dependencies the tray does
//! not otherwise need.

#[cfg(target_os = "linux")]
pub mod ksni;
#[cfg(any(target_os = "windows", target_os = "macos"))]
pub mod tray_icon;
