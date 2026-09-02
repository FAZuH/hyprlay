# Commit scopes

This document defines the allowed scopes for Conventional Commits.

## Allowed scopes

Use only these scopes:

| Scope | Code location |
|---|---|
| `cli` | `src/cli/` and `src/bin/hyprlay.rs` |
| `daemon` | `src/daemon/` and `src/bin/hyprlayd.rs` |
| `gui` | `src/gui/` and `src/bin/hyprlay-gui.rs` |
| `tray` | `src/tray/` and `src/bin/hyprlay-tray.rs` |
| `core` | `crates/hyprlay-core/` |
| `platform` | `src/platform/` |

Each scope maps to one crate, front, or module tree. If no scope fits, omit the scope. Do not create other scopes.

Examples:

```
feat(cli): add search filter
fix(core): clamp opacity at upper bound
feat: update CI workflow
```

## Scope does not affect version

Scope is human-readable only. It has no effect on the version bump.

CI determines the version bump from file paths under `crates/<member>/`. The scope does not change that result.

The root package (`Cargo.toml` + `src/`) contains four fronts. Each front maps to one scope as shown above. `crates/hyprlay-core` maps to `core`, and the shared adapter tree `src/platform/` maps to `platform`.

For bump rules, see `commit-changelog.md`.
