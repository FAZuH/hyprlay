//! Install/uninstall flows against real directories: these tests touch the
//! filesystem (tempdir XDG roots) and verify the exact bytes written plus
//! the systemctl command sequences, so they live in the integration suite
//! (ticket 02 test hygiene). systemctl itself is never shelled out to: the
//! flow takes an injectable [`install::Systemctl`] runner and the spy below
//! records every call.
#![cfg(target_os = "linux")]

mod common;

use std::cell::RefCell;
use std::path::PathBuf;

use common::unique_temp_dir;
use hyprlay::platform::service::systemd::Systemctl;
use hyprlay::platform::service::systemd::install;
use hyprlay::platform::service::systemd::uninstall;

const DAEMON_RELOAD: &str = "systemctl --user daemon-reload";
const ENABLE_NOW: &str = "systemctl --user enable --now hyprlay";
const ENABLE_NOW_TRAY: &str = "systemctl --user enable --now hyprlay-tray";
const DISABLE_NOW: &str = "systemctl --user disable --now hyprlay";
const DISABLE_NOW_TRAY: &str = "systemctl --user disable --now hyprlay-tray";
const TRAY_BIN: &str = "hyprlay-tray";
const DAEMON_BIN: &str = "hyprlayd";
const GUI_BIN: &str = "hyprlay-gui";

/// Spy double for the systemctl boundary: records each invocation as one
/// readable line and can be told to fail a specific command (simulating
/// e.g. "unit not found").
struct Spy {
    calls: RefCell<Vec<String>>,
    fail_when: Option<String>,
}

impl Spy {
    fn recording() -> Self {
        Self {
            calls: RefCell::new(Vec::new()),
            fail_when: None,
        }
    }

    fn failing_at(command: &str) -> Self {
        Self {
            calls: RefCell::new(Vec::new()),
            fail_when: Some(command.to_string()),
        }
    }

    fn lines(&self) -> Vec<String> {
        self.calls.borrow().clone()
    }
}

impl Systemctl for Spy {
    fn run(&self, args: &[&str]) -> Result<(), String> {
        let line = format!("systemctl --user {}", args.join(" "));
        self.calls.borrow_mut().push(line.clone());
        match &self.fail_when {
            Some(failing) if *failing == line => {
                Err("Failed to connect: unit not found".to_string())
            }
            _ => Ok(()),
        }
    }
}

/// One fresh world per test: isolated config/data bases and a bin dir that
/// mimics a real install location (empty unless the test populates it).
struct World {
    config_base: PathBuf,
    data_base: PathBuf,
    bin_dir: PathBuf,
}

impl World {
    fn new(tag: &str) -> Self {
        let root = unique_temp_dir(tag);
        let world = Self {
            config_base: root.join("config"),
            data_base: root.join("data"),
            bin_dir: root.join("bin"),
        };
        std::fs::create_dir_all(&world.bin_dir).unwrap();
        world
    }

    fn with_sibling_bins(&self) {
        std::fs::write(self.bin_dir.join("hyprlayd"), b"").unwrap();
        std::fs::write(self.bin_dir.join("hyprlay-gui"), b"").unwrap();
        std::fs::write(self.bin_dir.join(TRAY_BIN), b"").unwrap();
    }

    fn unit(&self) -> PathBuf {
        self.config_base.join("systemd/user/hyprlay.service")
    }

    fn tray_unit(&self) -> PathBuf {
        self.config_base.join("systemd/user/hyprlay-tray.service")
    }

    fn desktop(&self) -> PathBuf {
        self.data_base.join("applications/hyprlay.desktop")
    }

    /// Independently written truth: the spec-pinned unit text with the bin
    /// dir substituted. Deliberately NOT derived from any production code.
    fn expected_unit(&self) -> String {
        format!(
            "[Unit]\n\
             Description=hyprlay overlay daemon\n\
             \n\
             [Service]\n\
             ExecStart={}/hyprlayd\n\
             Restart=on-failure\n\
             EnvironmentFile=-%h/.config/hyprlay/service.env\n\
             \n\
             [Install]\n\
             WantedBy=default.target\n",
            self.bin_dir.display()
        )
    }

    /// Independently written truth for the tray unit, mirroring the daemon
    /// template (Restart=on-failure, PassEnvironment, EnvironmentFile,
    /// WantedBy=default.target).
    fn expected_tray_unit(&self) -> String {
        format!(
            "[Unit]\n\
             Description=hyprlay system tray menu\n\
             \n\
             [Service]\n\
             ExecStart={}/{}\n\
             Restart=on-failure\n\
             PassEnvironment=WAYLAND_DISPLAY DISPLAY XDG_RUNTIME_DIR HYPRLAND_INSTANCE_SIGNATURE\n\
             EnvironmentFile=-%h/.config/hyprlay/service.env\n\
             \n\
             [Install]\n\
             WantedBy=default.target\n",
            self.bin_dir.display(),
            TRAY_BIN
        )
    }

    fn expected_desktop(&self) -> String {
        format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=hyprlay\n\
             Exec={}/hyprlay-gui\n\
             Icon=hyprlay\n\
             Terminal=false\n",
            self.bin_dir.display()
        )
    }

    /// The hicolor paths under `data_base/icons/hicolor/.../apps/hyprlay.*`
    /// that install must create and uninstall must remove.
    fn icon_paths(&self) -> Vec<PathBuf> {
        let mut paths = vec![
            self.data_base
                .join("icons/hicolor/scalable/apps/hyprlay.svg"),
        ];
        for size in [48, 64, 128, 256] {
            paths.push(
                self.data_base
                    .join(format!("icons/hicolor/{size}x{size}/apps/hyprlay.png")),
            );
        }
        paths
    }
}

impl Drop for World {
    fn drop(&mut self) {
        // Roots nest under one tempdir; removing the parents clears all.
        if let Some(root) = self.config_base.parent() {
            let _ = std::fs::remove_dir_all(root);
        }
    }
}

#[test]
fn install_writes_the_unit_file_with_pinned_contents() {
    let world = World::new("unit-content");
    world.with_sibling_bins();

    install(
        &world.bin_dir,
        &world.config_base,
        &world.data_base,
        true,
        &Spy::recording(),
    )
    .expect("install succeeds");

    assert_eq!(
        std::fs::read_to_string(world.unit()).unwrap(),
        world.expected_unit()
    );
}

#[test]
fn install_writes_the_desktop_entry_with_pinned_contents() {
    let world = World::new("desktop-content");
    world.with_sibling_bins();

    install(
        &world.bin_dir,
        &world.config_base,
        &world.data_base,
        true,
        &Spy::recording(),
    )
    .expect("install succeeds");

    assert_eq!(
        std::fs::read_to_string(world.desktop()).unwrap(),
        world.expected_desktop()
    );
}

#[test]
fn install_writes_hicolor_app_icons_and_uninstall_removes_them() {
    let world = World::new("icons");
    world.with_sibling_bins();

    install(
        &world.bin_dir,
        &world.config_base,
        &world.data_base,
        false,
        &Spy::recording(),
    )
    .expect("install succeeds");

    // Every icon size (scalable SVG + each PNG) is written and non-empty.
    for path in world.icon_paths() {
        assert!(path.exists(), "icon written: {}", path.display());
        assert!(
            std::fs::metadata(&path).unwrap().len() > 0,
            "icon non-empty: {}",
            path.display()
        );
    }

    let report = uninstall(&world.config_base, &world.data_base, &Spy::recording())
        .expect("uninstall succeeds");

    for path in world.icon_paths() {
        assert!(!path.exists(), "icon removed: {}", path.display());
    }
    // Each removal was reported (not "already absent") because a real install
    // created them moments before.
    for path in world.icon_paths() {
        assert!(
            report
                .iter()
                .any(|line| line == &format!("removed {}", path.display())),
            "uninstall reports removal of {}",
            path.display()
        );
    }
}

#[test]
fn install_runs_daemon_reload_then_enable_now_both_units_in_order() {
    let world = World::new("flow-order");
    world.with_sibling_bins();
    let spy = Spy::recording();

    install(
        &world.bin_dir,
        &world.config_base,
        &world.data_base,
        true,
        &spy,
    )
    .expect("install succeeds");

    assert_eq!(
        spy.lines(),
        vec![
            DAEMON_RELOAD.to_string(),
            ENABLE_NOW.to_string(),
            ENABLE_NOW_TRAY.to_string()
        ]
    );
}

#[test]
fn install_writes_the_tray_unit_file_with_pinned_contents() {
    let world = World::new("tray-unit-content");
    world.with_sibling_bins();

    install(
        &world.bin_dir,
        &world.config_base,
        &world.data_base,
        true,
        &Spy::recording(),
    )
    .expect("install succeeds");

    assert_eq!(
        std::fs::read_to_string(world.tray_unit()).unwrap(),
        world.expected_tray_unit()
    );
}

#[test]
fn no_start_still_writes_files_but_skips_the_enable_call() {
    let world = World::new("no-start");
    world.with_sibling_bins();
    let spy = Spy::recording();

    install(
        &world.bin_dir,
        &world.config_base,
        &world.data_base,
        false,
        &spy,
    )
    .expect("install succeeds");

    assert_eq!(
        std::fs::read_to_string(world.unit()).unwrap(),
        world.expected_unit()
    );
    assert_eq!(
        std::fs::read_to_string(world.desktop()).unwrap(),
        world.expected_desktop()
    );
    assert_eq!(spy.lines(), vec![DAEMON_RELOAD.to_string()]);
}

#[test]
fn install_overwrites_existing_files_idempotently() {
    let world = World::new("idempotent");
    world.with_sibling_bins();
    // Pre-existing junk from an earlier install must not survive or merge.
    std::fs::create_dir_all(world.unit().parent().unwrap()).unwrap();
    std::fs::create_dir_all(world.desktop().parent().unwrap()).unwrap();
    std::fs::write(world.unit(), b"[stale]").unwrap();
    std::fs::write(world.desktop(), b"[stale]").unwrap();

    let first = install(
        &world.bin_dir,
        &world.config_base,
        &world.data_base,
        true,
        &Spy::recording(),
    );
    let second = install(
        &world.bin_dir,
        &world.config_base,
        &world.data_base,
        true,
        &Spy::recording(),
    );

    first.expect("first install succeeds");
    second.expect("second install succeeds");
    assert_eq!(
        std::fs::read_to_string(world.unit()).unwrap(),
        world.expected_unit(),
        "second install leaves exactly the pinned unit"
    );
    assert_eq!(
        std::fs::read_to_string(world.desktop()).unwrap(),
        world.expected_desktop(),
        "second install leaves exactly the pinned desktop entry"
    );
    for path in world.icon_paths() {
        assert!(
            path.exists(),
            "icon survives a second install: {}",
            path.display()
        );
    }
}

#[test]
fn failed_daemon_reload_reports_the_step_and_never_enables() {
    let world = World::new("reload-fails");
    world.with_sibling_bins();
    let spy = Spy::failing_at(DAEMON_RELOAD);

    let err = install(
        &world.bin_dir,
        &world.config_base,
        &world.data_base,
        true,
        &spy,
    )
    .unwrap_err();

    assert!(err.contains("daemon-reload"), "error names the step: {err}");
    assert_eq!(spy.lines(), vec![DAEMON_RELOAD.to_string()]);
}

#[test]
fn failed_enable_now_reports_the_step_after_writing_both_files() {
    let world = World::new("enable-fails");
    world.with_sibling_bins();
    let spy = Spy::failing_at(ENABLE_NOW);

    let err = install(
        &world.bin_dir,
        &world.config_base,
        &world.data_base,
        true,
        &spy,
    )
    .unwrap_err();

    assert!(err.contains("enable"), "error names the step: {err}");
    assert_eq!(
        spy.lines(),
        vec![DAEMON_RELOAD.to_string(), ENABLE_NOW.to_string()]
    );
    assert!(world.unit().exists(), "unit stays written");
    assert!(world.desktop().exists(), "desktop entry stays written");
}

#[test]
fn uninstall_disables_then_removes_both_files() {
    let world = World::new("uninstall");
    world.with_sibling_bins();
    install(
        &world.bin_dir,
        &world.config_base,
        &world.data_base,
        false,
        &Spy::recording(),
    )
    .expect("setup install succeeds");
    let spy = Spy::recording();

    uninstall(&world.config_base, &world.data_base, &spy).expect("uninstall succeeds");

    assert!(!world.unit().exists());
    assert!(!world.tray_unit().exists());
    assert!(!world.desktop().exists());
    for path in world.icon_paths() {
        assert!(!path.exists(), "icon removed: {}", path.display());
    }
    assert_eq!(
        spy.lines(),
        vec![DISABLE_NOW.to_string(), DISABLE_NOW_TRAY.to_string()]
    );
}

#[test]
fn uninstall_tolerates_a_failed_disable_call_and_still_removes_files() {
    let world = World::new("disable-tolerated");
    std::fs::create_dir_all(world.unit().parent().unwrap()).unwrap();
    std::fs::create_dir_all(world.desktop().parent().unwrap()).unwrap();
    std::fs::write(world.unit(), b"[unit]").unwrap();
    std::fs::write(world.tray_unit(), b"[tray]").unwrap();
    std::fs::write(world.desktop(), b"[entry]").unwrap();
    let spy = Spy::failing_at(DISABLE_NOW);

    let report = uninstall(&world.config_base, &world.data_base, &spy)
        .expect("uninstall tolerates the disable failure");

    assert!(!world.unit().exists(), "unit still removed");
    assert!(!world.tray_unit().exists(), "tray unit still removed");
    assert!(!world.desktop().exists(), "desktop still removed");
    assert_eq!(
        spy.lines(),
        vec![DISABLE_NOW.to_string(), DISABLE_NOW_TRAY.to_string()]
    );
    assert!(
        report
            .iter()
            .any(|line| line.contains(DISABLE_NOW) && line.contains("tolerated")),
        "report admits the tolerated failure: {report:?}"
    );
}

#[test]
fn uninstall_succeeds_when_nothing_is_installed() {
    let world = World::new("uninstall-empty");
    let spy = Spy::recording();

    uninstall(&world.config_base, &world.data_base, &spy).expect("uninstall succeeds");

    assert_eq!(
        spy.lines(),
        vec![DISABLE_NOW.to_string(), DISABLE_NOW_TRAY.to_string()]
    );
}

#[test]
fn install_aborts_before_any_write_when_bins_are_missing() {
    let world = World::new("missing-bins");
    // No sibling binaries present at all.

    let err = install(
        &world.bin_dir,
        &world.config_base,
        &world.data_base,
        true,
        &Spy::recording(),
    )
    .unwrap_err();

    // The error names every missing bin.
    assert!(err.contains(DAEMON_BIN), "error names hyprlayd: {err}");
    assert!(err.contains(GUI_BIN), "error names hyprlay-gui: {err}");
    assert!(err.contains(TRAY_BIN), "error names hyprlay-tray: {err}");
    // Nothing was written before the check failed.
    assert!(!world.unit().exists(), "daemon unit must not be written");
    assert!(!world.tray_unit().exists(), "tray unit must not be written");
    assert!(
        !world.desktop().exists(),
        "desktop entry must not be written"
    );
}

#[test]
fn install_aborts_and_names_only_the_bins_that_are_absent() {
    let world = World::new("partial-bins");
    // Present: hyprlayd only. Missing: hyprlay-gui and hyprlay-tray.
    std::fs::write(world.bin_dir.join(DAEMON_BIN), b"").unwrap();

    let err = install(
        &world.bin_dir,
        &world.config_base,
        &world.data_base,
        true,
        &Spy::recording(),
    )
    .unwrap_err();

    assert!(
        !err.contains(DAEMON_BIN),
        "present bin must not be named: {err}"
    );
    assert!(err.contains(GUI_BIN), "missing hyprlay-gui named: {err}");
    assert!(err.contains(TRAY_BIN), "missing hyprlay-tray named: {err}");
    assert!(!world.unit().exists(), "nothing written on abort");
}
