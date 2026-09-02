//! IPC transport adapters: the control socket transport behind the
//! core-owned [`ControlEndpoint`] (client connect) and [`ControlListener`]
//! (daemon bind/accept) ports. The Discord local IPC transport (unix socket
//! on Linux/macOS, named pipe on Windows) lives in
//! `src/daemon/adapters/ipc.rs`.

pub mod control;
