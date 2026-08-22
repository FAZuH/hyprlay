//! Thin settings-window main: the iced application lives in the `hyprlay`
//! lib (`gui` module).

fn main() {
    std::process::exit(hyprlay::gui::run());
}
