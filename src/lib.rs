//! Library surface of the hyprlay multi-binary package: the three
//! frontends live here as modules (`cli`, `daemon`, `gui`) and the
//! `src/bin/*` targets are thin mains that call into them. Integration
//! tests under `tests/` link this lib and exercise the same surface an
//! external consumer would. Not public API — hence `doc(hidden)`.
//!
//! Front↔front isolation is a convention, not a compiler wall anymore:
//! the fronts must only meet at `hyprlay-core`. `tests/front_isolation.rs`
//! enforces it on every `cargo test`.

#[doc(hidden)]
pub mod cli;
#[doc(hidden)]
pub mod daemon;
#[doc(hidden)]
pub mod gui;
#[doc(hidden)]
pub mod tray;
