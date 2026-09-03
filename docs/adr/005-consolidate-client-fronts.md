# ADR-005: The GUI and tray fronts run inside the hyprlay binary

## Status

Accepted

## Date

2026-09-02

## Context

The package shipped four binaries: `hyprlay`, `hyprlayd`, `hyprlay-gui`,
and `hyprlay-tray`. Each binary was a thin main over one front module. All
four linked the same library and the same dependency tree.

That split bought no real isolation. Cargo already built everything as one
package, so the compiler wall between fronts was gone; the isolation
convention (`tests/front_isolation.rs`) is what holds the boundary. What
the four-bin split did cost:

- Four release assets per platform instead of two.
- Install checks that demand co-location: "the hyprlay binaries must be
  installed together" errors.
- Partial upgrades: a user who replaces two binaries but not the other two
  gets mismatched behaviour that looks like a bug.

The daemon is different. It is a supervised service with its own systemd
units, re-exec logic, and process identity. Users start and stop it
independently of any client. It keeps its own binary.

## Decision

Fold the client fronts into the launcher. `hyprlay gui` opens the settings
window and `hyprlay tray` runs the tray, both in-process. The daemon stays
a separate `hyprlayd` binary.

- One composition root routes every invocation: `run(args)` in `src/lib.rs`
  classifies argv, runs the `gui`/`tray` fronts in-process, and hands
  everything else to `cli::execute`. The `src/bin/hyprlay.rs` main stays a
  one-liner.
- The daemon is still reached by exec (`hyprlay daemon` execs the sibling
  `hyprlayd`). No front imports the daemon module.
- Front isolation is amended, not dropped. The fronts (`cli`, `daemon`,
  `gui`, `tray`) meet at `hyprlay-core` and at the composition root. The
  scanner sweeps only `src/{cli,daemon,gui,tray}`; `lib.rs` sits outside
  those directories, so it is the one legal meeting point besides core.
- Logical identities stay stable. The flock names (`hyprlay-gui`,
  `hyprlay-tray`), the GUI app id (`hyprlay-gui`), the unit names
  (`hyprlay.service`, `hyprlay-tray.service`), the desktop entry
  (`hyprlay.desktop`), and the ksni SNI id (`hyprlay-tray`) do not change.
  Only the binary file names go away.
- The tray still spawns the GUI as a separate process: it launches its own
  image with the `gui` argument (`hyprctl dispatch exec <exe> gui` when the
  environment is headless on Hyprland). Focusing an already-running GUI is
  unchanged.
- Install now verifies one sibling: `REQUIRED_BINS` shrinks to
  `[hyprlayd]`. The desktop entry runs `hyprlay gui`; the tray unit runs
  `hyprlay tray`. Unit names and every report line stay the same.
- No shim binaries, no binary-file migration. Old `hyprlay-gui` and
  `hyprlay-tray` files left by a previous release are harmless orphans.
  After updating, users re-run `hyprlay install`, which rewrites the unit
  and desktop files with the new commands.

This amends ADR-001 in one detail: the tray is no longer a fourth thin
bin. The decision behind ADR-001 stands — the tray is still a separate
resident process that outlives the daemon.

## Rejected alternatives

### Keep four binaries

Zero migration cost, but the distribution cost stays: four assets per
platform, co-location errors, partial-upgrade failures. The thin bins
separate nothing that the isolation convention does not already separate.

### Merge the daemon into `hyprlay`

Rejected explicitly. The daemon is a supervised service. It needs its own
units, its own restart policy, and a process identity the user can stop
and start on its own. One binary would tie the service lifecycle to a
client invocation.

### Ship compatibility shim binaries

Keep `hyprlay-gui` and `hyprlay-tray` as tiny launchers that exec
`hyprlay gui` / `hyprlay tray`. Rejected: the shims recreate the
co-location problem they exist to paper over, and the old files are
harmless orphans anyway.

## Consequences

Two artifacts ship per platform: `hyprlay` and `hyprlayd`.

The tray and GUI processes now show as `hyprlay` in `ps` and process
lists, not `hyprlay-tray` / `hyprlay-gui`. Accepted cosmetic change.

On Windows, `hyprlay gui` and `hyprlay tray` no longer go through the
spawn-and-exit approximation of exec. One process runs the front. The
daemon exec path keeps its Windows branch unchanged.

ADR-001 keeps its force: the tray is still a separate resident process
that outlives the daemon. Only its packaging changed.
