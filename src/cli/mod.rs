//! Argv front-end: a clap builder tree classifies the invocation locally,
//! then hands wire commands to the daemon through the existing [`Command`]
//! grammar. The split of responsibilities is deliberate:
//!
//! - **clap owns argv shape** — unknown commands, wrong arity, and the
//!   `-h`/`-V` flags are answered here, in clap's own (unpinned) text.
//! - **core owns meaning** — every relayed invocation is rebuilt into its
//!   canonical wire line and validated by `Command::from_str`, so token
//!   vocabularies and error wording live in exactly one place and CLI-local
//!   semantic errors are byte-identical to what the daemon would reply.
//!
//! Contract note: CLI-local parse errors are clap-owned or locally printed
//! core text; they are NOT part of the byte-stable socket contract. Only
//! daemon replies keep that guarantee.

pub mod dispatch;
pub mod install;

use clap::Arg;
use clap::ArgAction;
use clap::ArgMatches;
use clap::Command as ClapCommand;
use hyprlay_core::ctl::examples_section;
use hyprlay_core::ctl::files_section;
use hyprlay_core::ctl::keys_table;
use hyprlay_core::domain::Command;

use crate::cli::dispatch::DAEMON_BIN;
use crate::cli::dispatch::GUI_BIN;
use crate::cli::dispatch::TRAY_BIN;
use crate::cli::dispatch::exec_sibling;

/// What the launcher does with one invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// Exec the sibling `hyprlayd` binary.
    Daemon,
    /// Exec the sibling `hyprlay-gui` binary.
    Gui,
    /// List compositor outputs without contacting the daemon.
    /// Run the system tray menu.
    Tray,
    Monitors,
    /// Write the user unit + desktop entry and (unless `no_start`) enable
    /// + start the service — all locally, before any socket connect.
    Install {
        no_start: bool,
    },
    /// Stop/disable the unit and remove both files; never touches the
    /// daemon socket.
    Uninstall,
    /// Send this command (canonical wire form) to the daemon.
    Relay(Command),
    /// A semantic parse failure: the message is the daemon's own error
    /// wording, printed locally because the line never left the process.
    LocalError(String),
    /// Print this rendered root help to stdout and exit 0. Bare `hyprlay`
    /// lands here so nothing starts unless asked for explicitly.
    Help(String),
}

const ROOT_HELP_TEMPLATE: &str = concat!(
    "{name} {version} — {about}\n",
    "\n",
    "USAGE:\n",
    "    {usage}\n",
    "\n",
    "For help on commands, add -h after: \"hyprlay <command> -h\"\n",
    "\n",
    "{subcommands}\n",
    "{options}\n",
    "{after-help}",
);

const MOVE_TOKENS_HELP: &str = "positions: left | right | center | top | bottom\n";

const RESET_SECTIONS_HELP: &str = "sections: position | layout | opacity | colors
Omit to reset everything; the monitor choice is always kept.
";

const NUDGE_RUNTIME_ONLY_HELP: &str =
    "Nudges shift the overlay at runtime only: re-anchoring the surface or
restarting the daemon resets the shift. Persistent shifts belong in the
config: use offset-x / offset-y instead.
";

/// The clap argument tree. Single construction point so the help output,
/// the accepted grammar, and `outcome` can never disagree.
fn cli() -> ClapCommand {
    let leaf = |name: &'static str, about: &'static str| ClapCommand::new(name).about(about);

    ClapCommand::new("hyprlay")
        .version(env!("CARGO_PKG_VERSION"))
        .about("lightweight Discord voice overlay (Wayland/Hyprland)")
        .subcommand_required(true)
        .help_template(ROOT_HELP_TEMPLATE)
        .after_help(format!("{}\n{}", files_section(), examples_section()))
        .subcommand(leaf("daemon", "run the overlay daemon"))
        .subcommand(leaf("gui", "open the settings window"))
        .subcommand(leaf("tray", "run the system tray menu"))
        .subcommand(
            leaf("get", "read one setting")
                .arg(Arg::new("key").help("setting name (see KEYS below)"))
                .after_help(keys_help()),
        )
        .subcommand(
            leaf(
                "set",
                "change one setting; omit value to cycle enums / flip flags",
            )
            .arg(Arg::new("key").help("setting name (see KEYS below)"))
            // Negative numbers must survive as values (`set offset-x -50`)
            // while real flags like -h stay flags.
            .arg(
                Arg::new("value")
                    .required(false)
                    .allow_negative_numbers(true)
                    .help("new value; omit to cycle enums / flip flags"),
            )
            .after_help(keys_help()),
        )
        .subcommand(leaf("status", "connection + config summary"))
        .subcommand(leaf("dump", "live runtime config as TOML"))
        .subcommand(
            leaf("move", "re-anchor to a screen edge")
                .arg(
                    Arg::new("pos")
                        .required(true)
                        .value_name("pos")
                        .help("screen edge or center"),
                )
                .after_help(MOVE_TOKENS_HELP),
        )
        .subcommand(
            leaf("nudge", "shift the overlay by pixels")
                .arg(
                    Arg::new("dx")
                        .value_name("dx")
                        .allow_negative_numbers(true)
                        .help("horizontal pixels"),
                )
                .arg(
                    Arg::new("dy")
                        .value_name("dy")
                        .allow_negative_numbers(true)
                        .help("vertical pixels"),
                )
                .after_help(NUDGE_RUNTIME_ONLY_HELP),
        )
        .subcommand(
            leaf(
                "reset",
                "reset all or one group to defaults (keeps the monitor)",
            )
            .arg(
                Arg::new("section")
                    .required(false)
                    .value_name("section")
                    .help("config group"),
            )
            .after_help(RESET_SECTIONS_HELP),
        )
        .subcommand(leaf("save", "persist runtime config to config.toml"))
        .subcommand(leaf("reload", "re-read config.toml"))
        .subcommand(leaf(
            "restart",
            "re-exec the daemon (applies new credentials)",
        ))
        .subcommand(leaf("monitors", "list outputs"))
        .subcommand(
            leaf(
                "install",
                "install the systemd user service + desktop entry",
            )
            .arg(
                Arg::new("no_start")
                    .long("no-start")
                    .action(ArgAction::SetTrue)
                    .help("write the files but do not enable/start the service"),
            ),
        )
        .subcommand(leaf(
            "uninstall",
            "remove the systemd user service + desktop entry",
        ))
        .subcommand(leaf("quit", "stop the running daemon cleanly"))
}

/// The KEYS appendix for `get -h` / `set -h`: generated at runtime from
/// `Key::ALL` through the SAME formatter the wire `help` reply uses, so
/// the two surfaces cannot drift.
fn keys_help() -> String {
    format!("KEYS (use with get/set):\n{}", keys_table())
}

/// Parse argv into a locally executable [`Outcome`]. Help/version requests
/// come back as clap display errors carrying their rendered text; nothing
/// in the `Err` arm ever touches the socket.
pub(crate) fn classify(args: &[String]) -> Result<Outcome, clap::Error> {
    // Bare `hyprlay` shows help and exits 0; starting the daemon is
    // reserved for explicit paths (`daemon`, direct `hyprlayd`, the
    // systemd service). Rendering goes through the same clap tree as
    // `-h`, so the two outputs cannot drift.
    if args.is_empty() {
        return Ok(Outcome::Help(cli().render_help().to_string()));
    }
    let matches = cli()
        .try_get_matches_from(std::iter::once("hyprlay".to_string()).chain(args.iter().cloned()))?;
    Ok(outcome(&matches))
}

/// Map parsed matches onto the outcome enum. Wire-shaped subcommands are
/// rebuilt as their canonical line and validated by the core grammar, so a
/// bad value fails HERE with the daemon's own wording instead of making a
/// doomed socket round-trip.
fn outcome(matches: &ArgMatches) -> Outcome {
    let (name, sub) = matches.subcommand().expect("root requires a subcommand");
    match name {
        "daemon" => Outcome::Daemon,
        "gui" => Outcome::Gui,
        "tray" => Outcome::Tray,
        "monitors" => Outcome::Monitors,
        "install" => Outcome::Install {
            no_start: sub.get_flag("no_start"),
        },
        "uninstall" => Outcome::Uninstall,
        _ => match wire_line(name, sub).and_then(|line| line.parse::<Command>()) {
            Ok(command) => Outcome::Relay(command),
            Err(message) => Outcome::LocalError(message),
        },
    }
}

/// Rebuild the canonical wire line for a wire-shaped subcommand. Token
/// vocabularies are NOT repeated here: the core `FromStr` is the authority.
fn wire_line(name: &str, sub: &ArgMatches) -> Result<String, String> {
    let word = |id: &str| {
        sub.get_one::<String>(id)
            .map(String::as_str)
            .unwrap_or_default()
    };
    Ok(match name {
        "get" => format!("get {}", word("key")),
        "set" => match sub.get_one::<String>("value") {
            Some(value) => format!("set {} {value}", word("key")),
            None => format!("set {}", word("key")),
        },
        "move" => format!("move {}", word("pos")),
        "nudge" => format!("nudge {} {}", word("dx"), word("dy")),
        "reset" => match sub.get_one::<String>("section") {
            Some(section) => format!("reset {section}"),
            None => "reset".to_string(),
        },
        simple @ ("save" | "dump" | "status" | "reload" | "restart" | "quit") => simple.to_string(),
        other => unreachable!("non-wire subcommand reached the relay mapper: {other}"),
    })
}

/// Route the invocation and carry it out. Returns the process exit code;
/// successful sibling execs replace this process and never return. `pub`
/// because the thin `src/bin/hyprlay.rs` main calls into it.
pub fn run(args: &[String]) -> i32 {
    match classify(args) {
        Ok(Outcome::Daemon) => exec_sibling(DAEMON_BIN),
        Ok(Outcome::Gui) => exec_sibling(GUI_BIN),
        Ok(Outcome::Tray) => exec_sibling(TRAY_BIN),
        Ok(Outcome::Monitors) => list_monitors(),
        Ok(Outcome::Install { no_start }) => crate::cli::install::run_install(no_start),
        Ok(Outcome::Uninstall) => crate::cli::install::run_uninstall(),
        Ok(Outcome::Relay(command)) => relay(&command.to_string()),
        Ok(Outcome::LocalError(message)) => {
            eprintln!("{message}");
            1
        }
        Ok(Outcome::Help(text)) => {
            print!("{text}");
            0
        }
        Err(err) => {
            let _ = err.print();
            err.exit_code()
        }
    }
}

/// Compositor outputs, answered locally: detection needs no daemon and
/// `monitors` exists to help pick a value for `set monitor`.
fn list_monitors() -> i32 {
    let monitors = crate::platform::compositor::detect().monitors();
    if monitors.is_empty() {
        eprintln!("error: no monitors reported (is a supported compositor running?)");
        return 1;
    }
    for m in monitors {
        println!(
            "{:<12} {}{}",
            m.name,
            m.description,
            if m.active { "   [active]" } else { "" }
        );
    }
    0
}

/// The daemon owns persistence (autosave, default on): the CLI just relays
/// one command line and prints the reply byte-for-byte.
fn relay(line: &str) -> i32 {
    match hyprlay_core::ctl::send_command_line(&crate::platform::ipc::control::Control, line) {
        Some(reply) => {
            print!("{reply}");
            0
        }
        None => 1,
    }
}

#[cfg(test)]
mod tests {
    use clap::error::ErrorKind;
    use hyprlay_core::ctl::keys_table;
    use hyprlay_core::domain::Command;
    use hyprlay_core::domain::Group;
    use hyprlay_core::domain::Key;
    use hyprlay_core::domain::Value;

    use super::Outcome;
    use super::classify;
    use super::cli;
    use super::run;

    fn argv(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    fn classified(args: &[&str]) -> Outcome {
        classify(&argv(args)).expect("classification succeeds")
    }

    fn clap_rejected(args: &[&str]) -> clap::Error {
        classify(&argv(args)).expect_err("clap answers this locally")
    }

    /// Rendered help/version text exactly as the user would see it.
    fn rendered(args: &[&str]) -> String {
        clap_rejected(args).render().to_string()
    }

    #[test]
    fn bare_invocation_classifies_as_help_not_the_daemon() {
        // Bare `hyprlay` must never silently start the daemon: only
        // explicit paths (`hyprlay daemon`, direct `hyprlayd`, the
        // systemd service) may launch it.
        assert!(matches!(classified(&[]), Outcome::Help(_)));
    }

    #[test]
    fn bare_help_is_identical_to_dash_h() {
        let Outcome::Help(bare) = classified(&[]) else {
            panic!("bare argv must classify as Help");
        };
        assert_eq!(
            bare,
            rendered(&["-h"]),
            "bare help must be the same text as `-h`"
        );
    }

    #[test]
    fn bare_invocation_exits_zero() {
        // Help is a successful answer, not an error.
        assert_eq!(run(&[]), 0);
    }

    #[test]
    fn explicit_daemon_word_launches_the_daemon() {
        assert_eq!(classified(&["daemon"]), Outcome::Daemon);
    }

    #[test]
    fn gui_word_launches_the_settings_window() {
        assert_eq!(classified(&["gui"]), Outcome::Gui);
    }

    #[test]
    fn tray_word_launches_the_system_tray() {
        assert_eq!(classified(&["tray"]), Outcome::Tray);
    }

    #[test]
    fn help_flags_stay_local_and_never_reach_the_socket() {
        // All three spellings are answered in-process as clap display
        // requests; keeping them local preserves scripts that run
        // `hyprlay help`. A display error carries no Outcome, so no relay
        // can possibly be constructed from them.
        for spelling in ["-h", "--help", "help"] {
            assert_eq!(
                clap_rejected(&[spelling]).kind(),
                ErrorKind::DisplayHelp,
                "`{spelling}` must render help locally"
            );
        }
    }

    #[test]
    fn version_flags_stay_local() {
        for spelling in ["-V", "--version"] {
            assert_eq!(clap_rejected(&[spelling]).kind(), ErrorKind::DisplayVersion);
        }
    }

    #[test]
    fn monitors_is_answered_locally_from_compositor_detection() {
        assert_eq!(classified(&["monitors"]), Outcome::Monitors);
    }

    #[test]
    fn install_classifies_as_a_local_install_outcome() {
        assert_eq!(
            classified(&["install"]),
            Outcome::Install { no_start: false }
        );
    }

    #[test]
    fn install_no_start_flag_lands_in_the_outcome() {
        // --no-start must reach the flow: files are written but the unit
        // stays disabled.
        assert_eq!(
            classified(&["install", "--no-start"]),
            Outcome::Install { no_start: true }
        );
    }

    #[test]
    fn uninstall_classifies_as_a_local_uninstall_outcome() {
        assert_eq!(classified(&["uninstall"]), Outcome::Uninstall);
    }

    #[test]
    fn single_word_wire_commands_relay_their_canonical_form() {
        let cases = [
            (&["status"][..], Command::Status),
            (&["save"][..], Command::Save),
            (&["dump"][..], Command::Dump),
            (&["reload"][..], Command::Reload),
            (&["restart"][..], Command::Restart),
            (&["quit"][..], Command::Quit),
        ];
        for (args, expected) in cases {
            assert_eq!(classified(args), Outcome::Relay(expected), "{args:?}");
        }
    }

    #[test]
    fn set_and_get_relay_typed_wire_commands() {
        assert_eq!(
            classified(&["set", "opacity", "80"]),
            Outcome::Relay(Command::Set(Key::Opacity, Value::Num(80)))
        );
        assert_eq!(
            classified(&["get", "width"]),
            Outcome::Relay(Command::Get(Key::Width))
        );
    }

    #[test]
    fn bare_set_value_relays_the_cycle_form() {
        assert_eq!(
            classified(&["set", "talking-only"]),
            Outcome::Relay(Command::Set(Key::TalkingOnly, Value::Cycle))
        );
    }

    #[test]
    fn placement_commands_relay_typed_wire_commands() {
        assert_eq!(
            classified(&["move", "top"]),
            Outcome::Relay(Command::MoveEdge(hyprlay_core::domain::Edge::Top))
        );
        assert_eq!(
            classified(&["nudge", "3", "-4"]),
            Outcome::Relay(Command::Nudge(3, -4))
        );
        assert_eq!(classified(&["reset"]), Outcome::Relay(Command::ResetAll));
        assert_eq!(
            classified(&["reset", "colors"]),
            Outcome::Relay(Command::ResetGroup(Group::Colors))
        );
    }

    #[test]
    fn negative_values_survive_parsing_for_set_and_nudge() {
        assert_eq!(
            classified(&["set", "offset-x", "-50"]),
            Outcome::Relay(Command::Set(Key::OffsetX, Value::Num(-50)))
        );
        assert_eq!(
            classified(&["nudge", "-5", "10"]),
            Outcome::Relay(Command::Nudge(-5, 10))
        );
    }

    #[test]
    fn move_middle_reports_the_full_token_list_locally() {
        // `middle` was an undocumented alias and is gone everywhere (spec
        // D2). The CLI rejects it with the daemon's own full-token error
        // text, without needing a daemon to say it.
        assert_eq!(
            classified(&["move", "middle"]),
            Outcome::LocalError("error: move <left|right|center|top|bottom>".to_string())
        );
    }

    #[test]
    fn unknown_reset_group_reports_the_full_group_list_locally() {
        assert_eq!(
            classified(&["reset", "everything"]),
            Outcome::LocalError("error: reset <position|layout|opacity|colors>".to_string())
        );
    }

    #[test]
    fn non_numeric_nudge_reports_the_wire_syntax_locally() {
        assert_eq!(
            classified(&["nudge", "a", "b"]),
            Outcome::LocalError("error: nudge <dx> <dy>".to_string())
        );
    }

    #[test]
    fn unknown_key_errors_keep_the_daemons_wording() {
        for args in [&["get", "nonsense"][..], &["set", "nonsense", "1"][..]] {
            let Outcome::LocalError(message) = classified(args) else {
                panic!("{args:?} must fail semantically");
            };
            assert!(message.starts_with("error: unknown key"), "{message}");
        }
    }

    #[test]
    fn out_of_range_set_value_is_rejected_before_the_socket() {
        assert_eq!(
            classified(&["set", "width", "10000"]),
            Outcome::LocalError("error: width <200-600>".to_string())
        );
    }

    #[test]
    fn unrecognized_subcommands_are_clap_owned_text() {
        // Contract change: the front-end's own rejections no longer travel
        // the socket, so their wording is clap's and unpinned — only the
        // local, non-relayed outcome is guaranteed here.
        let err = clap_rejected(&["explode"]);
        assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn missing_arguments_are_clap_owned_text() {
        let err = clap_rejected(&["move"]);
        assert_eq!(err.kind(), ErrorKind::MissingRequiredArgument);
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn every_subcommand_answers_h_locally() {
        let names: Vec<String> = cli()
            .get_subcommands()
            .map(|c| c.get_name().to_string())
            .collect();
        assert!(!names.is_empty());
        for name in names {
            assert_eq!(
                clap_rejected(&[&name, "-h"]).kind(),
                ErrorKind::DisplayHelp,
                "`{name} -h` must render help locally"
            );
        }
    }

    #[test]
    fn root_help_keeps_the_tailscale_shape() {
        let help = rendered(&["-h"]);
        let mut lines = help.lines();
        assert_eq!(
            lines.next(),
            Some(concat!(
                "hyprlay ",
                env!("CARGO_PKG_VERSION"),
                " — lightweight Discord voice overlay (Wayland/Hyprland)"
            )),
            "banner opens the root help"
        );
        let usage = help.find("USAGE:").expect("USAGE heading");
        let pointer = help
            .find("For help on commands, add -h after: \"hyprlay <command> -h\"")
            .expect("pointer line");
        let files = help.find("FILES:").expect("FILES section retained");
        let examples = help
            .find("EXAMPLES (Hyprland):")
            .expect("EXAMPLES section retained");
        assert!(
            usage < pointer && pointer < files && files < examples,
            "section order"
        );
        assert!(
            !help.contains("KEYS"),
            "KEYS must not appear in the main help"
        );
        assert!(help.contains("-h, --help"), "FLAGS: help flag listed");
        assert!(help.contains("-V, --version"), "FLAGS: version flag listed");
    }

    #[test]
    fn root_help_lists_every_command_with_a_description() {
        let help = rendered(&["-h"]);
        for sub in cli().get_subcommands() {
            let name = sub.get_name();
            if name == "help" {
                continue; // clap's built-in help shortcut, not a hyprlay command
            }
            let about = sub.get_about().expect("every command documents itself");
            let row = name.to_string();
            let description = about.to_string();
            assert!(
                help.contains(&row) && help.contains(&description),
                "root help must list `{name}` — {description}"
            );
        }
        for required in ["get", "set", "install", "uninstall", "quit"] {
            assert!(
                help.contains(required),
                "`{required}` missing from root help"
            );
        }
    }

    #[test]
    fn set_help_appends_the_runtime_keys_table_without_duplication() {
        let help = rendered(&["set", "-h"]);
        assert!(help.contains("KEYS (use with get/set):"));
        assert!(
            help.contains(&keys_table()),
            "rows must be the shared formatter's"
        );
        for key in Key::ALL {
            // Row-anchored: key names also occur inside group hints
            // (`offsets, rtl, output`) and prose (`per-part opacity`),
            // so raw substring counts would be meaningless.
            let rows = help
                .lines()
                .filter(|line| {
                    line.trim_start()
                        .strip_prefix(key.name())
                        .is_some_and(|rest| rest.starts_with(char::is_whitespace))
                })
                .count();
            assert_eq!(rows, 1, "{} must appear exactly once", key.name());
        }
    }

    #[test]
    fn get_help_appends_the_same_keys_table_as_set() {
        let appendix = |args: &[&str]| {
            let help = rendered(args);
            let start = help
                .find("KEYS (use with get/set):")
                .expect("KEYS appendix");
            help[start..].to_string()
        };
        assert_eq!(
            appendix(&["get", "-h"]),
            appendix(&["set", "-h"]),
            "get and set share one KEYS appendix"
        );
    }

    #[test]
    fn move_help_is_terse_but_keeps_the_full_token_list_nearby() {
        let help = rendered(&["move", "-h"]);
        assert!(
            help.contains("hyprlay move <pos>"),
            "terse synopsis: {help}"
        );
        assert!(
            help.contains("left | right | center | top | bottom"),
            "full tokens documented"
        );
        assert!(!help.contains("middle"));
    }

    #[test]
    fn reset_help_is_terse_but_names_the_groups() {
        let help = rendered(&["reset", "-h"]);
        assert!(help.contains("hyprlay reset [section]"), "terse synopsis");
        for group in Group::ALL {
            assert!(help.contains(group.to_string().as_str()), "{group} named");
        }
    }

    #[test]
    fn nudge_help_documents_runtime_only_semantics() {
        // clap wraps prose at the terminal width, so match on
        // whitespace-normalized text instead of exact line breaks.
        let flat = rendered(&["nudge", "-h"])
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            flat.contains("re-anchoring the surface or restarting"),
            "{flat}"
        );
        assert!(flat.contains("offset-x / offset-y"), "{flat}");
    }
}
