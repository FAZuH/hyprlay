//! Shared foundation of the hyprlay binaries: the domain vocabulary
//! (commands, keys, replies), the persisted config, framework-free color
//! math, Discord credential storage, compositor discovery, and the ctl
//! socket protocol. Everything here must stay free of UI frameworks and
//! async runtimes so every binary can afford to link it.

pub mod bins;
pub mod color;
pub mod compositor;
pub mod config;
pub mod credentials;
pub mod ctl;
pub mod daemon_control;
pub mod domain;
pub mod platform;
pub mod singleton;
