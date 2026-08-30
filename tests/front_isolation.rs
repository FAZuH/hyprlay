//! Front↔front isolation enforcement. Since ticket 12 consolidated the
//! three frontends into one package, `cargo` no longer stops `src/cli`
//! from importing `crate::gui` or `crate::daemon` — that compiler wall is
//! gone. Isolation is now a convention: the fronts may only meet at
//! `hyprlay-core`. This test re-arms the boundary on every plain
//! `cargo test` run (no CI change needed): it scans `src/{cli,daemon,gui}`
//! for `use crate::<other-front>` style paths and fails listing each
//! violation, so an accidental cross-front import turns red immediately.

use std::path::Path;

const FRONTS: [&str; 4] = ["cli", "daemon", "gui", "tray"];

/// Code text with `//` and `///` comments stripped, so prose that merely
/// mentions a sibling front cannot fail the scan. Block comments are not
/// used in this tree.
fn code_of(line: &str) -> &str {
    match line.find("//") {
        Some(idx) => &line[..idx],
        None => line,
    }
}

/// Cross-front violations in one file: `crate::<front>` / `hyprlay::<front>`
/// paths pointing at a different front than the file's own, plus
/// `super::<front>` in a front's `mod.rs` (there — and only there —
/// `super` IS the crate root, so it can smuggle a cross-front import past
/// the `crate::` rule).
fn violations_in(path: &Path, owner: &str) -> Vec<String> {
    let body = std::fs::read_to_string(path).unwrap_or_else(|e| {
        panic!("could not read {}: {e}", path.display());
    });
    let at_mod_root = path.file_name().is_some_and(|n| n == "mod.rs");
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
                        found.push(format!(
                            "{}:{}: {marker}{front}",
                            path.display(),
                            lineno + 1
                        ));
                    }
                }
                tail = rest;
            }
        };
        hits("crate::");
        hits("hyprlay::");
        if at_mod_root {
            hits("super::");
        }
    }
    found
}

#[test]
fn no_front_imports_another_front() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut all = Vec::new();
    for front in FRONTS {
        let dir = src.join(front);
        let entries = std::fs::read_dir(&dir).unwrap_or_else(|e| {
            panic!("could not list {}: {e}", dir.display());
        });
        for entry in entries.flatten() {
            let path = entry.path();
            let kind = entry.file_type().expect("file type readable");
            if kind.is_file() && path.extension().is_some_and(|e| e == "rs") {
                all.extend(violations_in(&path, front));
            } else if kind.is_dir() {
                // Nested dirs belong to their parent front (adapters/,
                // overlay/ under daemon), so they inherit `front` as owner.
                let nested = std::fs::read_dir(&path).expect("nested dir readable");
                for inner in nested.flatten() {
                    let inner_path = inner.path();
                    if inner_path.extension().is_some_and(|e| e == "rs") {
                        all.extend(violations_in(&inner_path, front));
                    }
                }
            }
        }
    }
    assert!(
        all.is_empty(),
        "cross-front imports broke the isolation convention \
         (fronts may only meet at hyprlay-core):\n{}",
        all.join("\n")
    );
}
