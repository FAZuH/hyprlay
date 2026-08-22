//! Overlay concerns: runtime state, Wayland layer-shell geometry math, and
//! the always-on-top view rendering. The shell wiring in main.rs composes
//! these with the adapters.

pub mod geometry;
pub mod state;
pub mod view;
