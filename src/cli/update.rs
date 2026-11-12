//! CLI adapter for `hyprlay update`: runs the platform update flow and prints
//! its report. Like `install` and `uninstall`, it is answered before any
//! socket connect — it manages the on-disk install, not the running daemon.
//!
//! Exit codes: 0 updated or already current, 10 update available under
//! `--check` (scriptable), 1 failure.

use crate::platform::update::SystemHost;
use crate::platform::update::UpdateOptions;
use crate::platform::update::UpdateReport;

pub fn run_update(check: bool, force: bool) -> i32 {
    let options = UpdateOptions { check, force };
    match crate::platform::update::run(&SystemHost, &options) {
        Ok(UpdateReport {
            lines,
            update_available,
        }) => {
            for line in lines {
                println!("{line}");
            }
            if check && update_available { 10 } else { 0 }
        }
        Err(message) => {
            eprintln!("error: {message}");
            1
        }
    }
}
