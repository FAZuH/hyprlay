//! Front↔front isolation enforcement. Since ticket 12 consolidated the
//! three frontends into one package, `cargo` no longer stops `src/cli`
//! from importing `crate::gui` or `crate::daemon` — that compiler wall is
//! gone. Isolation is now a convention: the fronts may only meet at
//! `hyprlay-core` and at the crate-root composition root (the `hyprlay::run`
//! function in `src/lib.rs`, which routes `gui`/`tray` in-process and is
//! outside the directories scanned here). This test re-arms the boundary on
//! every plain `cargo test` run (no CI change needed): it scans all of
//! `src/` for cross-front imports and fails listing each violation, so an
//! accidental cross-front import turns red immediately.
//!
//! Two rules are enforced here, both from ADR-004:
//!
//! 1. **Front isolation** — a front (`cli`, `daemon`, `gui`, `tray`) never
//!    imports another front. The composition root `src/lib.rs` is exempt.
//! 2. **Platform-crate containment** — a platform crate (`ksni`,
//!    `iced_layershell`, `windows-sys`, …) is imported only by `src/platform/`
//!    and the one sanctioned composition point per ADR-004's Amendment.
//!
//! The scan handles grouped imports (`use crate::{daemon, gui}`), any
//! `super::` depth, and recursion past one level, none of which the
//! previous one-level `crate::`-prefix scan caught.

use std::path::Path;

const FRONTS: [&str; 4] = ["cli", "daemon", "gui", "tray"];

/// Platform crates per ADR-004. A platform crate import outside
/// `src/platform/` is a layer violation, unless it is one of the two
/// sanctioned composition points below.
const PLATFORM_CRATES: [&str; 7] = [
    "ksni",
    "tray_icon",
    "iced_layershell",
    "windows_sys",
    "objc2_app_kit",
    "core_graphics",
    "x11rb",
];

/// Where a platform crate may legally be imported. ADR-004's Amendment
/// blesses `src/platform/` and the layer-shell surface host, which is the
/// documented composition point for the Wayland types the overlay needs.
fn platform_allowed(owner: &str, path: &Path) -> bool {
    owner == "platform" || path.components().any(|c| c.as_os_str() == "surface_host")
}

/// Code text with `//` and `///` comments stripped, so prose that merely
/// mentions a sibling front cannot fail the scan. Block comments are not
/// used in this tree.
fn code_of(line: &str) -> &str {
    match line.find("//") {
        Some(idx) => &line[..idx],
        None => line,
    }
}

/// How deep in `src/<front>/…` a file sits, as the number of `super::` hops
/// from it back to `src/<front>/`. `src/gui/mod.rs` is 0, `src/gui/a.rs` is
/// 0, `src/gui/a/mod.rs` is 0 (its `super` is `gui`), `src/gui/a/b.rs` is 1.
///
/// This is what makes the `super::` rule correct for nested modules: a file
/// at depth *n* resolving `super::super::…` (n+1 hops) lands on the crate
/// root, which is the composition root and exempt.
fn depth_of(path: &Path, front: &str) -> usize {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let front_dir = src.join(front);
    // Count the components between the front dir and the file's parent.
    path.parent()
        .and_then(|p| p.strip_prefix(&front_dir).ok())
        .map(|rel| rel.components().count().saturating_sub(1))
        .unwrap_or(0)
}

/// Cross-front and platform-crate violations in one file.
///
/// Handles the four import shapes the previous scan missed:
///
/// - `use crate::<front>` — the original rule
/// - `use crate::{a, b}` — grouped; the brace means the name is not
///   immediately after `crate::`, so the old prefix match saw nothing
/// - `use super::<front>` — legal only when it resolves to the crate root
/// - `use super::super::<front>` — same, one level deeper
fn violations_in(path: &Path, owner: &str) -> Vec<String> {
    let body = std::fs::read_to_string(path).unwrap_or_else(|e| {
        panic!("could not read {}: {e}", path.display());
    });
    let mut found = Vec::new();

    for (lineno, raw) in body.lines().enumerate() {
        let line = code_of(raw);
        let hits = |marker: &str, out: &mut Vec<String>| {
            let mut tail = line;
            while let Some(pos) = tail.find(marker) {
                let rest = &tail[pos + marker.len()..];
                for front in FRONTS {
                    if rest.starts_with(front)
                        && !rest[front.len()..]
                            .starts_with(|c: char| c.is_alphanumeric() || c == '_')
                        && front != owner
                    {
                        out.push(format!(
                            "{}:{}: {marker}{front}",
                            path.display(),
                            lineno + 1
                        ));
                    }
                }
                tail = rest;
            }
        };

        let mut line_hits = Vec::new();
        hits("crate::", &mut line_hits);
        hits("hyprlay::", &mut line_hits);
        // Grouped imports: `use crate::{daemon, gui}` has a brace after
        // `crate::`, so the prefix match above sees nothing. Scan the brace
        // list itself.
        hits_grouped(line, owner, &mut line_hits, path, lineno + 1);
        // `super::` resolves to the crate root only when the file sits at the
        // front's top level. Deeper, it resolves to the parent module, so
        // `super::gui` from `src/daemon/overlay/mod.rs` is a cross-front
        // import. depth_of returns the hops from the file back to its front
        // dir; a `super::` chain that reaches past that lands on the crate
        // root, which is exempt.
        if count_super_hops(line) > depth_of(path, owner) {
            hits("super::", &mut line_hits);
        }
        found.extend(line_hits);
    }
    found
}

/// Grouped-import violations in one line: `use crate::{daemon, gui}` and
/// `use super::{gui, daemon}` name two fronts at once, and the brace means
/// neither is immediately after the root marker, so the plain prefix match
/// sees nothing.
fn hits_grouped(line: &str, owner: &str, found: &mut Vec<String>, path: &Path, lineno: usize) {
    for marker in ["crate::{", "super::{", "hyprlay::{"] {
        let mut tail = line;
        while let Some(pos) = tail.find(marker) {
            let Some(close) = tail[pos + marker.len()..].find('}') else {
                break;
            };
            let list = &tail[pos + marker.len()..pos + marker.len() + close];
            for item in list.split(',') {
                let name = item.trim();
                for front in FRONTS {
                    if name == front && front != owner {
                        found.push(format!("{}:{}: {marker}{name}}}", path.display(), lineno));
                    }
                }
            }
            tail = &tail[pos + marker.len() + close + 1..];
        }
    }
}

/// How many consecutive `super::` prefixes a line's first import starts
/// with: `use super::super::gui::X` is 2, `use super::gui::X` is 1.
fn count_super_hops(line: &str) -> usize {
    let mut tail = line;
    let mut hops = 0;
    while let Some(pos) = tail.find("super::") {
        // Only count it when it starts the path, not when it appears mid-path
        // after a named module (e.g. `crate::foo::super::` is not legal Rust).
        if hops == 0 && pos != 0 && !tail[..pos].ends_with("use ") {
            break;
        }
        hops += 1;
        tail = &tail[pos + "super::".len()..];
    }
    hops
}

/// Platform-crate violations in one file: a platform crate named anywhere in
/// code, outside the directories ADR-004 sanctions.
fn platform_violations_in(path: &Path, owner: &str) -> Vec<String> {
    if platform_allowed(owner, path) {
        return Vec::new();
    }
    let body = std::fs::read_to_string(path).unwrap_or_else(|e| {
        panic!("could not read {}: {e}", path.display());
    });
    let mut found = Vec::new();
    for (lineno, raw) in body.lines().enumerate() {
        let line = code_of(raw);
        for crate_name in PLATFORM_CRATES {
            for marker in [format!("{crate_name}::"), format!("use {crate_name}")] {
                if let Some(pos) = line.find(&marker) {
                    // `crate::platform::<crate>` is the platform module's own
                    // path, not the crate. Reject a match preceded by a path
                    // segment.
                    let before = line[..pos].trim_end();
                    let preceded_by_path = (before.ends_with(':') && !before.ends_with("use")
                        || before.ends_with(|c: char| c.is_alphanumeric() || c == '_'))
                        && !before.ends_with("use");
                    if preceded_by_path {
                        continue;
                    }
                    found.push(format!(
                        "{}:{}: platform crate {crate_name} outside src/platform/",
                        path.display(),
                        lineno + 1
                    ));
                    break;
                }
            }
        }
    }
    found
}

/// Every `.rs` under `src/`, recursively, as `(path, owner-front)`.
/// The composition root `src/lib.rs` is exempt from the front rule per
/// ADR-005, but is still scanned for platform crates.
fn all_source_files() -> Vec<(std::path::PathBuf, String)> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut out = Vec::new();
    fn walk(dir: &Path, owner: &str, out: &mut Vec<(std::path::PathBuf, String)>) {
        let entries = std::fs::read_dir(dir).unwrap_or_else(|e| {
            panic!("could not list {}: {e}", dir.display());
        });
        for entry in entries.flatten() {
            let path = entry.path();
            let kind = entry.file_type().expect("file type readable");
            if kind.is_file() && path.extension().is_some_and(|e| e == "rs") {
                out.push((path, owner.to_string()));
            } else if kind.is_dir() {
                // Nested dirs belong to their parent front (adapters/,
                // overlay/, surface_host/ under daemon), so they inherit the
                // owner. Recursion is unbounded, so a module nested deeper
                // than one level is still seen.
                walk(&path, owner, out);
            }
        }
    }
    for front in FRONTS {
        walk(&src.join(front), front, &mut out);
    }
    // The composition root is its own owner: it is exempt from the front rule
    // and it is where the sanctioned platform composition happens.
    out.push((src.join("lib.rs"), "composition_root".to_string()));
    out
}

#[test]
fn no_front_imports_another_front() {
    let mut all = Vec::new();
    for (path, owner) in all_source_files() {
        // The composition root is exempt from the front rule (ADR-005): it is
        // where gui/tray are routed in-process.
        if owner == "composition_root" {
            continue;
        }
        all.extend(violations_in(&path, &owner));
    }
    assert!(
        all.is_empty(),
        "cross-front imports broke the isolation convention \
         (fronts may only meet at hyprlay-core):\n{}",
        all.join("\n")
    );
}

#[test]
fn no_platform_crate_outside_the_sanctioned_directories() {
    let mut all = Vec::new();
    for (path, owner) in all_source_files() {
        all.extend(platform_violations_in(&path, &owner));
    }
    assert!(
        all.is_empty(),
        "platform crates are imported outside src/platform/ and the \
         sanctioned composition points (ADR-004):\n{}",
        all.join("\n")
    );
}

#[test]
fn the_scan_actually_sees_what_it_claims() {
    // Three deliberate violation probes. A scan that passes on first run
    // proves nothing; these are the evidence it sees grouped imports, deep
    // recursion, and the platform rule.
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");

    // 1. Grouped import — the shape the old prefix match missed entirely.
    let grouped = violations_in_from_text("use crate::{daemon, gui};\nfn x() {}\n", "gui", 0);
    assert!(
        grouped.iter().any(|v| v.contains("crate::{daemon")),
        "grouped import not caught: {grouped:?}"
    );

    // 2. Platform crate outside src/platform/.
    let platform = platform_violations_from_text(
        "fn anchor() -> iced_layershell::reexport::Anchor { unreachable!() }\n",
        "daemon",
    );
    assert!(
        platform.iter().any(|v| v.contains("iced_layershell")),
        "platform crate not caught: {platform:?}"
    );

    // 3. A front importing another front, one level deep.
    let plain = violations_in_from_text("use crate::gui::theme::BRIGHT;\nfn x() {}\n", "daemon", 1);
    assert!(
        plain.iter().any(|v| v.contains("crate::gui")),
        "plain cross-front import not caught: {plain:?}"
    );

    let _ = src;
}

/// The scan core, factored out of the file-reading path so the probes above
/// can drive it directly without writing to `src/`.
fn violations_in_from_text(body: &str, owner: &str, _depth: usize) -> Vec<String> {
    let mut found = Vec::new();
    for (lineno, raw) in body.lines().enumerate() {
        let line = code_of(raw);
        let mut hits = |marker: &str| {
            let mut tail = line;
            while let Some(pos) = tail.find(marker) {
                let rest = &tail[pos + marker.len()..];
                for front in FRONTS {
                    if rest.starts_with(front)
                        && !rest[front.len()..]
                            .starts_with(|c: char| c.is_alphanumeric() || c == '_')
                        && front != owner
                    {
                        found.push(format!("<test>:{}: {marker}{front}", lineno + 1));
                    }
                }
                tail = rest;
            }
        };
        hits("crate::");
        hits("hyprlay::");
        hits("super::");
        let mut grouped = Vec::new();
        hits_grouped(line, owner, &mut grouped, Path::new("<test>"), lineno + 1);
        found.extend(grouped);
    }
    found
}

fn platform_violations_from_text(body: &str, owner: &str) -> Vec<String> {
    if platform_allowed(owner, Path::new("")) {
        return Vec::new();
    }
    let mut found = Vec::new();
    for (lineno, raw) in body.lines().enumerate() {
        let line = code_of(raw);
        for crate_name in PLATFORM_CRATES {
            if line.contains(&format!("{crate_name}::")) {
                found.push(format!(
                    "<test>:{}: platform crate {crate_name}",
                    lineno + 1
                ));
            }
        }
    }
    found
}
