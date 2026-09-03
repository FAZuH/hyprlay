//! Thin launcher main: argv routing lives in the `hyprlay` composition root
//! (`hyprlay::run` in `src/lib.rs`), which dispatches `gui`/`tray` in-process
//! and hands the rest to `cli`. The `cli` module owns install/uninstall,
//! socket relay, and the sibling exec that reaches the `hyprlayd` daemon.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(hyprlay::run(&args));
}
