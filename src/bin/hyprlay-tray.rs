//! Thin tray main: the StatusNotifierItem front lives in the `hyprlay` lib
//! (`tray` module).

fn main() {
    std::process::exit(hyprlay::tray::run());
}
