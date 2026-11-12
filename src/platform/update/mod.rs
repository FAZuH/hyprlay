//! `hyprlay update`: replace both binaries with the latest GitHub release, or
//! rebuild them with `cargo install` when cargo installed them, and refresh the
//! autostart registration.
//!
//! The flow is one pass, never resumable state: check the release, download
//! and verify both assets into the install directory, swap them, then refresh
//! the service. Nothing is replaced until both downloads verify, so a failed
//! or truncated download leaves the install untouched.
//!
//! The network, filesystem, and self-replacement boundaries live behind
//! [`UpdateHost`] so this module stays testable with a fake host.

pub mod host;
pub mod release;

use std::fs;
use std::path::Path;
use std::path::PathBuf;

pub use host::SystemHost;
use hyprlay_core::bins::DAEMON_BIN;
use release::Asset;
use release::Release;
use release::asset_names;
use release::digest_matches;
use release::is_newer;

/// What the caller asked for.
#[derive(Debug, Clone, Copy, Default)]
pub struct UpdateOptions {
    /// Report the latest release without installing it.
    pub check: bool,
    /// Reinstall even when the running version is current or newer.
    pub force: bool,
}

/// The outcome: lines to print, and whether a newer release exists.
#[derive(Debug)]
pub struct UpdateReport {
    pub lines: Vec<String>,
    pub update_available: bool,
}

/// The boundary `run` drives. The real implementation is [`SystemHost`];
/// tests supply a fake so the whole flow runs against a temp directory.
pub trait UpdateHost {
    fn current_exe(&self) -> Result<PathBuf, String>;
    fn latest_release(&self) -> Result<Release, String>;
    fn download(&self, url: &str) -> Result<Vec<u8>, String>;
    /// Move `staged` over `target`. Replacing the running binary must go
    /// through the platform's self-replace path.
    fn replace(&self, staged: &Path, target: &Path) -> Result<(), String>;
    /// Whether an autostart entry (unit, plist, Startup script) is installed.
    fn service_installed(&self) -> bool;
    /// Rewrite the autostart entry for the new binaries and start the daemon.
    fn refresh_service(&self, exe_dir: &Path) -> Result<Vec<String>, String>;
    /// Stop the running daemon so its image can be replaced (Windows).
    fn stop_daemon(&self) -> Result<(), String>;
    /// Rebuild and reinstall with `cargo install`, because cargo owns the
    /// binary and its own bookkeeping must stay in step.
    fn run_cargo_install(&self, force: bool) -> Result<Vec<String>, String>;
}

/// How the running binary was installed, from the path it lives at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InstallKind {
    /// A directory `hyprlay update` may write: the release assets are the update.
    Portable,
    /// `cargo install` owns the binaries, so cargo has to rebuild them.
    Cargo,
    /// A system package manager owns the path.
    System,
}

/// Classify the install from its path shape, never from the filesystem.
fn install_kind(exe: &Path) -> InstallKind {
    let text = exe.to_string_lossy().replace('\\', "/");
    if text.contains("/.cargo/bin/") {
        InstallKind::Cargo
    } else if text.starts_with("/usr/") {
        InstallKind::System
    } else {
        InstallKind::Portable
    }
}

/// Run one `hyprlay update`.
pub fn run(host: &dyn UpdateHost, options: &UpdateOptions) -> Result<UpdateReport, String> {
    let current = env!("CARGO_PKG_VERSION");

    let (cli_name, daemon_name) = asset_names(std::env::consts::OS, std::env::consts::ARCH)
        .ok_or_else(|| {
            format!(
                "no prebuilt binaries for {}/{}; build from source instead",
                std::env::consts::OS,
                std::env::consts::ARCH
            )
        })?;

    let release = host.latest_release()?;
    if !options.force && !is_newer(current, &release.tag_name) {
        return Ok(UpdateReport {
            lines: vec![format!("hyprlay {current} is up to date")],
            update_available: false,
        });
    }

    let cli_asset = release
        .asset(&cli_name)
        .ok_or_else(|| format!("release {} does not ship {cli_name}", release.tag_name))?;
    let daemon_asset = release
        .asset(&daemon_name)
        .ok_or_else(|| format!("release {} does not ship {daemon_name}", release.tag_name))?;

    if options.check {
        return Ok(UpdateReport {
            lines: vec![format!(
                "hyprlay {} is available (running {current})",
                release.version()
            )],
            update_available: true,
        });
    }

    let exe = host.current_exe()?;
    let Some(dir) = exe.parent().map(Path::to_path_buf) else {
        return Err(format!("could not find the directory of {}", exe.display()));
    };

    let mut lines = match install_kind(&exe) {
        InstallKind::Cargo => host.run_cargo_install(options.force)?,
        InstallKind::System if !options.force => {
            return Err(format!(
                "hyprlay at {} is owned by the system package manager\nupdate it with your package manager, or re-run with --force",
                exe.display()
            ));
        }
        InstallKind::Portable | InstallKind::System => {
            let cli_bytes = download_verified(host, cli_asset)?;
            let daemon_bytes = download_verified(host, daemon_asset)?;

            let staged_cli = dir.join(format!(".{cli_name}.new"));
            let staged_daemon = dir.join(format!(".{daemon_name}.new"));
            stage(&staged_cli, &cli_bytes)?;
            stage(&staged_daemon, &daemon_bytes)?;

            // On Windows a running image is locked, and only the daemon holds
            // the sibling binary open. Unix replaces a running image without
            // this. The quit is best effort: a daemon that is not running has
            // nothing to stop.
            #[cfg(target_os = "windows")]
            host.stop_daemon().ok();

            let daemon_target = dir.join(format!("{DAEMON_BIN}{}", std::env::consts::EXE_SUFFIX));
            host.replace(&staged_daemon, &daemon_target)?;
            host.replace(&staged_cli, &exe)?;

            // `self-replace` may copy rather than move; staged leftovers are noise.
            let _ = fs::remove_file(&staged_cli);
            let _ = fs::remove_file(&staged_daemon);

            vec![format!(
                "updated hyprlay {current} -> {}",
                release.version()
            )]
        }
    };

    if host.service_installed() {
        lines.extend(host.refresh_service(&dir)?);
    } else {
        lines.push("no autostart registration found; run `hyprlay install` to add one".to_string());
    }
    Ok(UpdateReport {
        lines,
        update_available: true,
    })
}

fn download_verified(host: &dyn UpdateHost, asset: &Asset) -> Result<Vec<u8>, String> {
    let Some(digest) = asset.digest.as_deref() else {
        return Err(format!(
            "release asset {} has no checksum; refusing to install it",
            asset.name
        ));
    };
    let bytes = host.download(&asset.browser_download_url)?;
    if !digest_matches(&bytes, digest) {
        return Err(format!(
            "checksum mismatch for {}; nothing was replaced",
            asset.name
        ));
    }
    Ok(bytes)
}

fn stage(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;

    let mut file = fs::File::create(path)
        .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    file.write_all(bytes)
        .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))
            .map_err(|error| format!("could not make {} executable: {error}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::fs;
    use std::path::Path;
    use std::path::PathBuf;
    use std::process::id;

    use super::InstallKind;
    use super::UpdateHost;
    use super::UpdateOptions;
    use super::UpdateReport;
    use super::install_kind;
    use super::run;
    use crate::platform::update::release::Release;

    const NEW_BODY: &[u8] = b"new binary";
    const NEW_DIGEST: &str =
        "sha256:2f17c9ffb972a6c5da72c2b3df01f7e2ccf52dad2c0059dac631232a15126d2e";

    /// A temp install directory with a fake `hyprlay` + `hyprlayd` pair.
    struct World {
        dir: PathBuf,
    }

    impl World {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("hyprlay-update-{name}-{}", id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("temp dir");
            fs::write(dir.join("hyprlay"), b"old cli").expect("old cli");
            fs::write(dir.join("hyprlayd"), b"old daemon").expect("old daemon");
            Self { dir }
        }

        fn exe(&self) -> PathBuf {
            self.dir.join("hyprlay")
        }
    }

    impl Drop for World {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    /// Records what the flow asked for and serves canned bytes.
    struct FakeHost {
        exe: PathBuf,
        tag: String,
        body: Vec<u8>,
        digest: String,
        installed: bool,
        cargo_error: Option<String>,
        replaced: RefCell<Vec<(PathBuf, PathBuf)>>,
        refreshed: RefCell<bool>,
        downloaded: RefCell<Vec<String>>,
        cargo_runs: RefCell<Vec<bool>>,
    }

    impl FakeHost {
        fn for_world(world: &World, tag: &str) -> Self {
            Self {
                exe: world.exe(),
                tag: tag.to_string(),
                body: NEW_BODY.to_vec(),
                digest: NEW_DIGEST.to_string(),
                installed: true,
                cargo_error: None,
                replaced: RefCell::new(Vec::new()),
                refreshed: RefCell::new(false),
                downloaded: RefCell::new(Vec::new()),
                cargo_runs: RefCell::new(Vec::new()),
            }
        }

        fn release_json(&self) -> String {
            let (cli, daemon) =
                super::release::asset_names(std::env::consts::OS, std::env::consts::ARCH)
                    .expect("the test host runs on a released platform");
            format!(
                r#"{{"tag_name":"{}","assets":[
                    {{"name":"{cli}","browser_download_url":"https://x/cli","digest":"{digest}"}},
                    {{"name":"{daemon}","browser_download_url":"https://x/daemon","digest":"{digest}"}}
                ]}}"#,
                self.tag,
                digest = self.digest
            )
        }
    }

    impl UpdateHost for FakeHost {
        fn current_exe(&self) -> Result<PathBuf, String> {
            Ok(self.exe.clone())
        }

        fn latest_release(&self) -> Result<Release, String> {
            Release::parse(&self.release_json())
        }

        fn download(&self, url: &str) -> Result<Vec<u8>, String> {
            self.downloaded.borrow_mut().push(url.to_string());
            Ok(self.body.clone())
        }

        fn replace(&self, staged: &Path, target: &Path) -> Result<(), String> {
            self.replaced
                .borrow_mut()
                .push((staged.to_path_buf(), target.to_path_buf()));
            fs::rename(staged, target).map_err(|error| error.to_string())
        }

        fn service_installed(&self) -> bool {
            self.installed
        }

        fn refresh_service(&self, _exe_dir: &Path) -> Result<Vec<String>, String> {
            *self.refreshed.borrow_mut() = true;
            Ok(vec!["refreshed the service".into()])
        }

        fn stop_daemon(&self) -> Result<(), String> {
            Ok(())
        }

        fn run_cargo_install(&self, force: bool) -> Result<Vec<String>, String> {
            self.cargo_runs.borrow_mut().push(force);
            match &self.cargo_error {
                Some(error) => Err(error.clone()),
                None => Ok(vec!["built hyprlay with cargo install".into()]),
            }
        }
    }

    fn options() -> UpdateOptions {
        UpdateOptions::default()
    }

    #[test]
    fn a_newer_release_replaces_both_binaries_and_refreshes_the_service() {
        let world = World::new("replace");
        let host = FakeHost::for_world(&world, "v99.0.0");

        let report: UpdateReport = run(&host, &options()).expect("update succeeds");

        assert!(report.update_available);
        assert_eq!(fs::read(world.exe()).unwrap(), NEW_BODY);
        assert_eq!(fs::read(world.dir.join("hyprlayd")).unwrap(), NEW_BODY);
        assert_eq!(host.replaced.borrow().len(), 2);
        assert!(*host.refreshed.borrow());
    }

    #[test]
    fn a_checksum_mismatch_aborts_before_any_replacement() {
        let world = World::new("digest");
        let mut host = FakeHost::for_world(&world, "v99.0.0");
        host.digest = format!("sha256:{}", "0".repeat(64));

        let error = run(&host, &options()).expect_err("checksum mismatch fails");

        assert!(error.contains("checksum mismatch"), "{error}");
        assert_eq!(fs::read(world.exe()).unwrap(), b"old cli");
        assert_eq!(fs::read(world.dir.join("hyprlayd")).unwrap(), b"old daemon");
        assert!(host.replaced.borrow().is_empty());
    }

    #[test]
    fn a_current_version_is_left_alone() {
        let world = World::new("current");
        let host = FakeHost::for_world(&world, &format!("v{}", env!("CARGO_PKG_VERSION")));

        let report = run(&host, &options()).expect("run succeeds");

        assert!(!report.update_available);
        assert!(report.lines[0].contains("up to date"));
        assert!(host.replaced.borrow().is_empty());
    }

    #[test]
    fn check_reports_availability_without_touching_the_install() {
        let world = World::new("check");
        let host = FakeHost::for_world(&world, "v99.0.0");

        let report = run(
            &host,
            &UpdateOptions {
                check: true,
                force: false,
            },
        )
        .expect("check succeeds");

        assert!(report.update_available);
        assert!(host.replaced.borrow().is_empty());
        assert_eq!(fs::read(world.exe()).unwrap(), b"old cli");
    }

    #[test]
    fn without_a_registration_the_service_is_not_touched() {
        let world = World::new("no-service");
        let mut host = FakeHost::for_world(&world, "v99.0.0");
        host.installed = false;

        let report = run(&host, &options()).expect("update succeeds");

        assert!(!*host.refreshed.borrow());
        assert!(
            report
                .lines
                .iter()
                .any(|line| line.contains("hyprlay install"))
        );
    }

    #[test]
    fn install_kind_reads_the_path_shape() {
        assert_eq!(
            install_kind(Path::new("/home/u/.cargo/bin/hyprlay")),
            InstallKind::Cargo
        );
        assert_eq!(
            install_kind(Path::new("C:\\Users\\u\\.cargo\\bin\\hyprlay.exe")),
            InstallKind::Cargo
        );
        assert_eq!(
            install_kind(Path::new("/usr/bin/hyprlay")),
            InstallKind::System
        );
        assert_eq!(
            install_kind(Path::new("/home/u/.local/bin/hyprlay")),
            InstallKind::Portable
        );
    }

    #[test]
    fn a_cargo_install_runs_cargo_instead_of_downloading_assets() {
        let world = World::new("cargo");
        let mut host = FakeHost::for_world(&world, "v99.0.0");
        host.exe = world.dir.join(".cargo/bin/hyprlay");

        let report = run(&host, &options()).expect("cargo update succeeds");

        assert!(report.update_available);
        assert_eq!(*host.cargo_runs.borrow(), vec![false]);
        assert!(host.downloaded.borrow().is_empty());
        assert!(host.replaced.borrow().is_empty());
        assert_eq!(fs::read(world.exe()).unwrap(), b"old cli");
        assert_eq!(fs::read(world.dir.join("hyprlayd")).unwrap(), b"old daemon");
    }

    #[test]
    fn check_on_a_cargo_install_reports_without_running_cargo() {
        let world = World::new("cargo-check");
        let mut host = FakeHost::for_world(&world, "v99.0.0");
        host.exe = world.dir.join(".cargo/bin/hyprlay");

        let report = run(
            &host,
            &UpdateOptions {
                check: true,
                force: false,
            },
        )
        .expect("check succeeds");

        assert!(report.update_available);
        assert!(host.cargo_runs.borrow().is_empty());
    }

    #[test]
    fn a_failed_cargo_install_surfaces_the_error_and_keeps_the_binaries() {
        let world = World::new("cargo-fail");
        let mut host = FakeHost::for_world(&world, "v99.0.0");
        host.exe = world.dir.join(".cargo/bin/hyprlay");
        host.cargo_error = Some("cargo install failed".into());

        let error = run(&host, &options()).expect_err("cargo failure fails");

        assert!(error.contains("cargo"), "{error}");
        assert_eq!(fs::read(world.exe()).unwrap(), b"old cli");
        assert_eq!(fs::read(world.dir.join("hyprlayd")).unwrap(), b"old daemon");
        assert!(host.replaced.borrow().is_empty());
    }

    #[test]
    fn system_owned_binaries_are_refused() {
        let world = World::new("system");
        let mut host = FakeHost::for_world(&world, "v99.0.0");
        host.exe = PathBuf::from("/usr/bin/hyprlay");

        let error = run(&host, &options()).expect_err("system path is refused");

        assert!(error.contains("package manager"), "{error}");
        assert!(host.downloaded.borrow().is_empty());
        assert!(host.cargo_runs.borrow().is_empty());
    }
}
