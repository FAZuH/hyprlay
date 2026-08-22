//! Thin launcher main: argv routing, sibling exec, install/uninstall and
//! socket relay all live in the `hyprlay` lib (`cli` module).

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(hyprlay::cli::run(&args));
}
