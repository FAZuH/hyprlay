# Commit scopes

This document defines the allowed scopes for Conventional Commits.

## Allowed scopes

Use only these scopes:

| Scope | Code location |
|---|---|
| `cli` | `src/cli/`, `src/bin/hyprlay.rs`, the `hyprlay::run` composition root in `src/lib.rs` |
| `daemon` | `src/daemon/` and `src/bin/hyprlayd.rs` |
| `gui` | `src/gui/` |
| `tray` | `src/tray/` |
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

The root package (`Cargo.toml` + `src/`) contains four fronts. Each front maps to one scope as shown above; the crate-root composition root (`src/lib.rs`) that routes `gui`/`tray` in-process belongs to the `cli` scope. `crates/hyprlay-core` maps to `core`, and the shared adapter tree `src/platform/` maps to `platform`.

For bump rules, see `commit-changelog.md`.
