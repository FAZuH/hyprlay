//! The concrete [`Platform`] object for the host OS: detached child spawn.
//! Path resolution and secure-permission defaults come from the pure helpers
//! in `hyprlay-core::platform`; only spawn is inherently platform-crust.

use std::process::Command;
use std::process::Stdio;

use hyprlay_core::platform::Platform;

/// The host platform context, resolved once and injected into the fronts.
pub struct Host;

impl Platform for Host {
    fn spawn(&self, cmd: &mut Command) -> std::io::Result<()> {
        // Detach the child: its own process group (so it never holds our
        // terminal), null stdio (so it cannot be killed by a close of our
        // PTY), and no wait (the caller drops the handle).
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?;
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW: a detached,
            // console-less background child.
            cmd.creation_flags(0x00000200 | 0x08000000)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?;
        }
        #[cfg(not(any(unix, windows)))]
        {
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?;
        }
        Ok(())
    }
}

/// Resolve the host platform context (a cheap unit struct).
pub fn host() -> Host {
    Host
}
