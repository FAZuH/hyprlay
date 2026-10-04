//! Platform-selected surface host for the daemon overlay window.
//!
//! Two arms share the roster view (`overlay::view`), the pure state machine
//! (`overlay::state`), and the geometry math (`overlay::geometry`), and differ
//! only in how the surface is created and how placement, resize, and hover
//! are applied:
//! - `layershell` (Linux/Wayland): the existing `iced_layershell` shell, kept
//!   as it was except for the surface height, which now comes from
//!   `boot_size`. It anchors a layer surface to a screen edge with margins.
//! - `winit` (Windows/macOS): a frameless, transparent, always-on-top
//!   `iced` window moved to the computed on-screen position.
//!
//! The domain logic (command resolution, subscriptions, the singleton probe,
//! and the daemon lifecycle) lives in the parent `daemon` module and is
//! shared by both arms; only the shell-specific emission of the effects and
//! the hover-rect source differ here.

use std::process::ExitCode;
use std::sync::Mutex;

use hyprlay_core::config::Config;

use crate::daemon::adapters::auth::OwnAppAuth;
use crate::daemon::overlay::state::Overlay;

#[cfg(target_os = "linux")]
mod layershell;

#[cfg(not(target_os = "linux"))]
mod winit;

/// Height an overlay with no rows to show starts at. Nothing is drawn in it
/// (an empty roster renders an empty transparent surface), but the layer-shell
/// protocol rejects a zero height unless the surface is anchored to opposite
/// edges. The horizontal may be — centered is — but the vertical never is, so
/// the zero dimension is the height. The winit arm has no such constraint and
/// inherits the value for continuity.
const EMPTY_HEIGHT: u32 = 64;

/// The size a host creates its surface at: the height the overlay's rows need,
/// or [`EMPTY_HEIGHT`] when it has no rows yet.
pub(crate) fn boot_size(overlay: &Overlay) -> (u32, u32) {
    let (width, height) = overlay.desired_size();
    if height == 0 {
        (width, EMPTY_HEIGHT)
    } else {
        (width, height)
    }
}

/// The overlay a boot closure hands to iced: the one the surface was sized
/// from, taken out of `slot` on the first call.
///
/// `BootFn` is an `Fn`, not an `FnOnce`, so iced may call it again. A later
/// call re-reads the roster cache instead of panicking — the surface is
/// already created, so there is nothing left to size, and a panic here would
/// take the daemon down over a re-read.
pub(crate) fn take_boot(slot: &Mutex<Option<Overlay>>, cfg: &Config) -> Overlay {
    let taken = slot.lock().unwrap_or_else(|p| p.into_inner()).take();
    taken.unwrap_or_else(|| Overlay::boot(cfg.clone()))
}

/// Run the platform-selected overlay shell. Returns the process exit code.
pub(crate) fn run(cfg: Config, auth: Option<OwnAppAuth>) -> ExitCode {
    #[cfg(target_os = "linux")]
    {
        layershell::run(cfg, auth)
    }

    #[cfg(not(target_os = "linux"))]
    {
        winit::run(cfg, auth)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marked_overlay(cfg: &Config) -> Overlay {
        // 999 is a width no config or cache file produces, so finding it
        // proves the slot's overlay was the one handed over.
        let mut overlay = Overlay::new(cfg.clone());
        overlay.config_mut().width = 999;
        overlay
    }

    #[test]
    fn the_boot_closure_takes_the_overlay_the_surface_was_sized_from() {
        let cfg = Config::default();
        let slot = Mutex::new(Some(marked_overlay(&cfg)));

        assert_eq!(take_boot(&slot, &cfg).config().width, 999);
    }

    #[test]
    fn a_second_boot_call_re_boots_instead_of_panicking() {
        let cfg = Config::default();
        let slot = Mutex::new(Some(marked_overlay(&cfg)));
        take_boot(&slot, &cfg);

        let second = take_boot(&slot, &cfg);

        assert_eq!(second.config().width, cfg.width);
    }

    #[test]
    fn boot_size_falls_back_to_64_for_an_empty_roster() {
        let overlay = Overlay::new(Config::default());

        assert_eq!(boot_size(&overlay), (Config::default().width, 64));
    }
}

/// A boot that reads the roster cache needs a real cache file, so it gets a
/// real directory.
#[cfg(test)]
mod fs_tests {
    use super::*;
    use crate::daemon::adapters::cache::Roster;
    use crate::daemon::adapters::discord::DiscordEvent;
    use crate::daemon::adapters::discord::Participant;

    /// A fresh directory per call. Cleaned up on drop.
    struct TempDir(std::path::PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("hyprlay-boot-test-{}-{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn participant(id: &str, name: &str, speaking: bool) -> Participant {
        Participant {
            id: id.to_string(),
            name: name.to_string(),
            avatar_hash: None,
            speaking,
            self_mute: false,
            self_deaf: false,
            server_mute: false,
            server_deaf: false,
        }
    }

    #[test]
    fn a_restart_sizes_the_surface_to_the_roster_it_reads_from_the_cache() {
        let dir = TempDir::new("cached-roster");
        let users = vec![
            participant("1", "fazuh", false),
            participant("2", "quiet_guest", false),
            participant("3", "dingus", false),
        ];
        Roster {
            // A channel no other roster test writes: `Roster::write_to` dedups
            // on content alone, across every directory.
            channel: Some("general".to_string()),
            me_id: Some("1".to_string()),
            users: users.clone(),
        }
        .write_to(dir.path());
        assert!(
            dir.path().join("roster.json").exists(),
            "the write was deduped, not written"
        );
        let cfg = Config {
            avatar_size: 34,
            spacing: 4,
            scale: 100,
            ..Config::default()
        };

        let mut overlay = Overlay::boot_from(cfg.clone(), dir.path());

        // 3 cached rows: 3 * (34+8) + 2 * 4.
        assert_eq!(boot_size(&overlay), (cfg.width, 134));
        // The recorded size is what keeps the first live roster event from
        // resizing a surface that already has that height. The boot overlay is
        // still Connecting, so applying the event does not write the cache.
        assert_eq!(overlay.size(), (cfg.width, 134));
        overlay.apply_discord(DiscordEvent::Participants(users));
        assert!(overlay.take_size_change().is_none());
    }

    #[test]
    fn a_restart_with_no_cache_file_has_no_rows_to_size_the_surface_to() {
        let dir = TempDir::new("no-cache");
        let cfg = Config::default();

        let overlay = Overlay::boot_from(cfg.clone(), dir.path());

        assert_eq!(boot_size(&overlay), (cfg.width, 64));
    }
}
