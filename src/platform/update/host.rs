//! The real [`UpdateHost`]: HTTPS through `ureq`, the GitHub release endpoint,
//! file replacement (including replacing the running binary), and the
//! platform service adapter.

use std::io::Read;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use super::UpdateHost;
use super::release::LATEST_RELEASE_URL;
use super::release::Release;

/// The production boundary `hyprlay update` runs against.
pub struct SystemHost;

/// Where `cargo install` builds hyprlay from.
const CARGO_REPOSITORY: &str = "https://github.com/FAZuH/hyprlay";

fn agent() -> String {
    format!("hyprlay/{}", env!("CARGO_PKG_VERSION"))
}

impl UpdateHost for SystemHost {
    fn current_exe(&self) -> Result<PathBuf, String> {
        std::env::current_exe()
            .map_err(|error| format!("could not locate the running hyprlay binary: {error}"))
    }

    fn latest_release(&self) -> Result<Release, String> {
        let body = ureq::get(LATEST_RELEASE_URL)
            .set("User-Agent", &agent())
            .set("Accept", "application/vnd.github+json")
            .timeout(Duration::from_secs(10))
            .call()
            .map_err(|error| format!("could not reach GitHub: {error}"))?
            .into_string()
            .map_err(|error| format!("could not read the release response: {error}"))?;
        Release::parse(&body)
    }

    fn download(&self, url: &str) -> Result<Vec<u8>, String> {
        let response = ureq::get(url)
            .set("User-Agent", &agent())
            .timeout(Duration::from_secs(120))
            .call()
            .map_err(|error| format!("could not download {url}: {error}"))?;
        let mut bytes = Vec::new();
        let mut reader = response.into_reader();
        reader
            .read_to_end(&mut bytes)
            .map_err(|error| format!("could not read {url}: {error}"))?;
        Ok(bytes)
    }

    fn replace(&self, staged: &Path, target: &Path) -> Result<(), String> {
        let running = std::env::current_exe().ok();
        if running.as_deref() == Some(target) {
            // Windows cannot rename over a running image; the crate does the
            // platform-specific dance so the swap also works there.
            return self_replace::self_replace(staged).map_err(|error| {
                #[cfg(windows)]
                let hint =
                    "\nthe tray may still hold the image: quit it from the tray menu and retry";
                #[cfg(not(windows))]
                let hint = "";
                format!("could not replace {}: {error}{hint}", target.display())
            });
        }
        replace_file(staged, target)
    }

    fn service_installed(&self) -> bool {
        crate::platform::service::registration_installed()
    }

    fn refresh_service(&self, exe_dir: &Path) -> Result<Vec<String>, String> {
        let config = dirs::config_dir().ok_or_else(|| {
            "could not determine the config directory (XDG_CONFIG_HOME or $HOME/.config)"
                .to_string()
        })?;
        let data = dirs::data_dir().ok_or_else(|| {
            "could not determine the data directory (XDG_DATA_HOME or $HOME/.local/share)"
                .to_string()
        })?;
        crate::platform::service::refresh_registration(exe_dir, &config, &data)
            .map_err(|error| error.to_string())
    }

    fn stop_daemon(&self) -> Result<(), String> {
        crate::platform::service::quit_daemon().map_err(|error| error.to_string())
    }

    fn run_cargo_install(&self, force: bool) -> Result<Vec<String>, String> {
        let mut command = std::process::Command::new("cargo");
        command
            .args(["install", "--locked", "--git", CARGO_REPOSITORY])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        if force {
            command.arg("--force");
        }
        // A console window over the tray is noise, as with the other spawns.
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let output = command
            .output()
            .map_err(|error| format!("could not run cargo: {error}"))?;
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if !output.status.success() {
            return Err(format!(
                "cargo install failed ({})\n{}",
                output.status,
                tail(&combined, 5).join("\n")
            ));
        }
        let mut lines = vec!["updated hyprlay with cargo install".to_string()];
        lines.extend(tail(&combined, 3));
        Ok(lines)
    }
}

/// The last `count` non-empty lines, where cargo puts its error or the final
/// install summary. The build log itself is too noisy to print.
fn tail(text: &str, count: usize) -> Vec<String> {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    lines[lines.len().saturating_sub(count)..]
        .iter()
        .map(|line| (*line).to_string())
        .collect()
}

/// Replace a file that is not the running image. Unix renames over the target
/// atomically; Windows refuses when the target exists and a just-quit daemon
/// may hold its image for a moment, so retry briefly there.
fn replace_file(staged: &Path, target: &Path) -> Result<(), String> {
    #[cfg(not(windows))]
    {
        std::fs::rename(staged, target)
            .map_err(|error| format!("could not replace {}: {error}", target.display()))
    }
    #[cfg(windows)]
    {
        let mut last = None;
        for _ in 0..20 {
            let _ = std::fs::remove_file(target);
            match std::fs::rename(staged, target) {
                Ok(()) => return Ok(()),
                Err(error) => last = Some(error),
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Err(format!(
            "could not replace {}: {}",
            target.display(),
            last.expect("at least one rename attempt ran")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::tail;

    #[test]
    fn tail_keeps_the_last_non_empty_lines() {
        let text = "error: boom\n\n  warning: other  \nInstalled package hyprlay v0.5.0\n";
        assert_eq!(
            tail(text, 2),
            vec!["warning: other", "Installed package hyprlay v0.5.0"]
        );
        assert!(tail("", 3).is_empty());
    }
}
