//! Platform-crust adapters: the concrete implementations of every port trait
//! owned by `hyprlay-core`, selected by `detect()` factories and injected
//! into the fronts. Each submodule is `#[cfg]`-gated so a target only pulls
//! the backend it can run. Nothing in here may be imported by a *front* as a
//! peer — fronts depend only on `hyprlay-core` plus an injected adapter.

pub mod compositor;
pub mod cursor;
pub mod host;
pub mod icon;
pub mod ipc;
pub mod service;
pub mod tray;

pub use host::Host;
pub use host::host;
