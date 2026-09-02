//! Thin daemon main: the platform-selected surface-host shell lives in the
//! `hyprlay` lib (`daemon` module).

fn main() -> std::process::ExitCode {
    hyprlay::daemon::run()
}
