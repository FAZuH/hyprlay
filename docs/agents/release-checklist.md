# Release Checklist

A release ships from a version branch. One branch holds one version, for
example `v0.3.2` (tip `b501dd0`). The `version = "0.3.0"` field in
`Cargo.toml` stays old on purpose; release CI rewrites it
(`.github/workflows/patch-version.yml:60-64`).

## Procedure

1. Owner: human — Merge feature work into the version branch with a PR. The
   `v0.3.2` branch took PR #18 and PR #19 this way.
2. Owner: human — Write every user-visible change into the top
   `## [Unreleased]` section of `CHANGELOG.md`. This repo is manual mode
   (`.github/changelog-mode`), so CI renames that section instead of writing
   one (`docs/dev/changelog.md:7-13`, `_release.yml:215-225`). CI does not add
   a new `[Unreleased]` section after a release; you write it for the next one.
3. Owner: human — Pick the bump. A patch release needs no special commit. For
   a minor or major release, add one commit titled `chore!(minor): ...` or
   `chore!(major): ...` (`.github/.config.cjs:4-12`).
4. Owner: human — Run the local gates below and fix every failure.
5. Owner: human — Open a PR from the version branch into `main`. Merge it.
   Read the `## Release Preview` comment on that PR first; it names the next
   tag and shows the release body (`changelog-preview.yml:5-6,190-216`).
6. Owner: CI — A push to `main` merges `main` into `release`
   (`autopromote.yml:4-6,24-25`). You do not touch `release`.
7. Owner: CI — A push to `release` starts the release (`release.yml:3-6`,
   `_release.yml`): patch `Cargo.toml` and `Cargo.lock` (96-103), build the
   binaries (105-114), rename `[Unreleased]`, commit
   `chore(release): vX.Y.Z [skip ci]`, tag it, push `release` and the tag
   (199-238), merge `release` into `main` (240-248), create the GitHub
   Release and post to Discord (250-295). crates.io publishing is off here
   (`release.yml:20`).
8. Owner: human — Run the checks in the Verify section.

## Local gates

These mirror the PR runner (`_pr-checks.yml:49-55`):

```
cargo +nightly fmt --all -- --check &&
cargo clippy --all-features --all-targets --no-deps -- -D warnings &&
cargo test --workspace --all-features --no-fail-fast
```

## Verify

```
git fetch --tags --force origin '+refs/tags/*:refs/tags/*'
git tag -l --sort=-v:refname | head -1     # want: vX.Y.Z
git log --oneline -1 origin/main           # want: chore(release): vX.Y.Z
git rev-parse origin/main origin/release   # want: both lines show one SHA
head -12 CHANGELOG.md                      # want: "## X.Y.Z (date)" first
```

Then open `https://github.com/FAZuH/hyprlay/releases/latest` and read the body.

## Two observed paths into `main`

`autopromote.yml:4-6` reacts to any push to `main`. Both a merged PR (history:
`e33220c Merge pull request #17 from FAZuH/v0.3.1`) and a direct push start a
release. No file in this repo states which one is required; this checklist
uses the PR path. A manual push to `release` also starts a release
(`release.yml:3-6`), and it ships whatever `release` holds, so do not use it.
