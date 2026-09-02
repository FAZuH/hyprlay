//! Library surface of the hyprlay package: the four fronts live here as
//! modules (`cli`, `daemon`, `gui`, `tray`) and the `src/bin/*` targets are
//! thin mains that call into them. Integration tests under `tests/` link
//! this lib and exercise the same surface an external consumer would. Not
//! public API — hence `doc(hidden)`.
//!
//! Front↔front isolation is a convention, not a compiler wall anymore:
//! the fronts must only meet at `hyprlay-core` **and at the crate-root
//! composition root** ([`run`]), which routes `gui`/`tray` in-process.
//! `tests/front_isolation.rs` enforces it on every `cargo test`.

//! The platform-crust adapters live outside the fronts (see `src/platform/`);
//! the isolation scanner only sweeps the four front modules.

#[doc(hidden)]
pub mod platform;

#[doc(hidden)]
pub mod cli;
#[doc(hidden)]
pub mod daemon;
#[doc(hidden)]
pub mod gui;
#[doc(hidden)]
pub mod tray;

/// Route one invocation and return the process exit code. The composition
/// root that the fronts may also meet at: `gui`/`tray` are in-process
/// fronts, everything else goes through [`cli::execute`]. The clap `Err`
/// arm is handled here too. Called by the thin `src/bin/hyprlay.rs` main.
pub fn run(args: &[String]) -> i32 {
    match cli::classify(args) {
        Ok(cli::Outcome::Gui) => gui::run(),
        Ok(cli::Outcome::Tray) => tray::run(),
        Ok(outcome) => cli::execute(outcome),
        Err(err) => {
            let _ = err.print();
            err.exit_code()
        }
    }
}
