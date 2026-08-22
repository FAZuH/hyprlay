//! Thin daemon main: the layer-shell application shell lives in the
//! `hyprlay` lib (`daemon` module).

fn main() -> iced_layershell::Result {
    hyprlay::daemon::run()
}
