#!/usr/bin/env bash
# Keyboard end-to-end harness for `hyprlay gui`.
#
# Why this exists: the Wayland seat on this machine never delivers a
# MODIFIER to the client, so Shift+Tab, Ctrl+F and the search paths cannot be
# tested there. An X11 seat does deliver modifiers, so this harness drives the
# real binary under a real window manager (Xvfb + i3), sends real keystrokes
# with xdotool, screenshots the window with ImageMagick, and compares the
# screenshots. Every check is a pass/fail, and the script exits non-zero if
# any of them fail.
#
# What it compares: two screenshots of the same window differ by some number
# of pixels. "Differs" proves a keystroke changed what is on screen; it never
# proves the right thing is on screen. The list at the end names the paths this
# cannot reach.
#
# Tools needed on PATH: Xvfb, i3, xdotool, import, compare.
#   NixOS (the machine this was written on), one command:
#     nix shell nixpkgs#xvfb nixpkgs#i3 nixpkgs#xdotool nixpkgs#imagemagick \
#       --command scripts/gui-keyboard-e2e.sh target/release/hyprlay
#   On a distro that ships the X client libraries (libX11, libXcursor, …) in
#   the loader path, the plain command works:
#     scripts/gui-keyboard-e2e.sh target/release/hyprlay
#   winit dlopens those libraries by soname (x11-dl) rather than linking them,
#   so where they live is a property of the loader, not of the binary. When the
#   window does not appear, this script asks nix once for their store paths and
#   retries; if that is not possible either, the failed run says which soname
#   was missing.
#
# Build the binary with `--bin hyprlay` only: the GUI's one-shot daemon
# auto-start spawns a sibling `hyprlayd`, and the daemon must stay down for the
# whole run or its status line would change the screenshots mid-flight.
#
# Usage: scripts/gui-keyboard-e2e.sh [BIN]   (default target/release/hyprlay)
#   DISP=:97 ROOT=/tmp/hyprlay-gui-e2e override the display and work dir.
set -u

BIN="${1:-${BIN:-target/release/hyprlay}}"
DISP="${DISP:-:97}"
ROOT="${ROOT:-/tmp/hyprlay-gui-e2e}"
SHOTS="$ROOT/shots"
export DISPLAY="$DISP"

die() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }

PASS=0
FAIL=0
pass() {
	PASS=$((PASS + 1))
	printf '  PASS  %s\n' "$*"
}
fail() {
	FAIL=$((FAIL + 1))
	printf '  FAIL  %s\n' "$*"
}

# ---------------------------------------------------------------- preflight ---
# Tools come from PATH; nothing here is pinned to a nix store path, so the
# script keeps working when the store hashes move. Missing tools are the one
# thing worth naming a package for, because that is a one-line fix.
for tool in Xvfb i3 xdotool import compare; do
	command -v "$tool" >/dev/null ||
		die "'$tool' is not on PATH.
  NixOS: nix shell nixpkgs#xvfb nixpkgs#i3 nixpkgs#xdotool nixpkgs#imagemagick \\
    --command $0 $BIN"
done
[ -x "$BIN" ] || die "'$BIN' is not an executable file (build it: cargo build --release --bin hyprlay)"

rm -rf "$SHOTS"
mkdir -p "$SHOTS" "$ROOT/home" "$ROOT/run"

# Kill only what this script started (pidfile). Never `pkill Xvfb`: this box
# may host another session's display.
cleanup() {
	[ -f "$ROOT/pids" ] && while read -r p; do kill "$p" 2>/dev/null; done <"$ROOT/pids"
	rm -f "$ROOT/pids"
}
trap cleanup EXIT
: >"$ROOT/pids"

printf 'font pango:DejaVu Sans Mono 8\n' >"$ROOT/i3.conf"
: >"$ROOT/import.log"

# ------------------------------------------------------------------ display ---
Xvfb "$DISP" -screen 0 1400x900x24 -nolisten tcp >"$ROOT/xvfb.log" 2>&1 &
echo $! >>"$ROOT/pids"
sleep 2
i3 -c "$ROOT/i3.conf" >"$ROOT/i3.log" 2>&1 &
echo $! >>"$ROOT/pids"
sleep 2

# ----------------------------------------------------------------------- app ---
# A stale hyprlay from an earlier run would be the window xdotool finds first.
# Safe to match on: this script's own cmdline never contains "$BIN gui".
pkill -f "$BIN gui" 2>/dev/null
sleep 1

# HOME and XDG_RUNTIME_DIR point into $ROOT so the run cannot read the real
# config or reach the real daemon's control socket. The launch's own stderr is
# redirected to gui.log so a dlopen panic is a log line to grep, not noise.
launch() {
	{
		env DISPLAY="$DISP" HOME="$ROOT/home" XDG_RUNTIME_DIR="$ROOT/run" \
			LD_LIBRARY_PATH="$LIBPATH" "$BIN" gui >"$ROOT/gui.log" 2>&1 &
		echo $! >>"$ROOT/pids"
		for _ in $(seq 40); do
			W=$(xdotool search --class hyprlay 2>/dev/null | head -1)
			[ -n "${W:-}" ] && return 0
			sleep 0.5
		done
		return 1
	} 2>>"$ROOT/gui.log"
}

LIBPATH="${LD_LIBRARY_PATH:-}"
if ! launch; then
	# winit dlopens the X client libraries by soname, and on NixOS they are
	# not on the loader path. Ask nix for their store paths once and retry.
	grep -q 'opening library failed' "$ROOT/gui.log" ||
		die "no hyprlay window, and no missing-library error to explain it:
$(tail -3 "$ROOT/gui.log")"
	command -v nix >/dev/null ||
		die "$(grep -o 'lib[^ ]*\.so[^ ]*' "$ROOT/gui.log" | head -1) is missing from the loader path and nix is not here to locate it.
  Put the X client libraries on LD_LIBRARY_PATH, then re-run."
	kill "$(tail -1 "$ROOT/pids")" 2>/dev/null
	LIBPATH="$(nix build --no-link --print-out-paths --impure \
		nixpkgs#libx11 nixpkgs#libxcursor nixpkgs#libxrandr nixpkgs#libxi \
		nixpkgs#libxext nixpkgs#libxfixes nixpkgs#libxrender nixpkgs#libxcb \
		nixpkgs#libxkbcommon 2>/dev/null |
		sed 's|$|/lib|' | paste -sd:)"
	launch || die "no hyprlay window after adding the X libraries to LD_LIBRARY_PATH:
$(grep -o 'lib[^ ]*\.so[^ ]*' "$ROOT/gui.log" | head -1) — see $ROOT/gui.log"
fi

xdotool windowactivate --sync "$W"
xdotool windowfocus --sync "$W"
sleep 1
pass "the settings window opened on X11: $(xdotool getwindowname "$W")"

# ---------------------------------------------------------------- assertions ---
# A capture that silently produced nothing would turn every later compare into
# a complaint about a corrupt file instead of naming the real fault, so a
# failed or empty capture stops the run here.
shot() {
	import -window "$W" "$SHOTS/$1.png" 2>>"$ROOT/import.log" ||
		die "screenshot '$1' failed: $(tail -2 "$ROOT/import.log")"
	[ -s "$SHOTS/$1.png" ] ||
		die "screenshot '$1' is empty: $(tail -2 "$ROOT/import.log")"
	printf '  shot  %s\n' "$1"
}

# assert_pixels LABEL BEFORE AFTER same|differs
# ImageMagick 7 prints "value (normalized)", and the value is not always a
# whole number, so compare it as text against zero instead of as an integer.
assert_pixels() {
	local raw n
	raw=$(compare -metric AE "$SHOTS/$2.png" "$SHOTS/$3.png" null: 2>&1)
	n=${raw%% *}
	case "$n" in
	'' | *[!0-9.]*) fail "$1: compare failed: $raw"; return ;;
	esac
	if [ "$4" = same ]; then
		[ "$n" = 0 ] && pass "$1: 0 px differ ($2 == $3)" ||
			fail "$1: $n px differ ($2, $3), expected no change"
	else
		[ "$n" = 0 ] && fail "$1: 0 px differ ($2 == $3), expected a change" ||
			pass "$1: $n px differ ($2 -> $3)"
	fi
}

# assert_region_pixels LABEL BEFORE AFTER GEOMETRY same|differs
# The same check over one rectangle of both shots (ImageMagick geometry), for
# the claims a whole-window diff cannot make: the window differs either way
# when focus moves, so only a crop of the part that must NOT have changed can
# say "the page did not move".
assert_region_pixels() {
	local raw n
	raw=$(compare -metric AE -extract "$4" "$SHOTS/$2.png" "$SHOTS/$3.png" null: 2>&1)
	n=${raw%% *}
	case "$n" in
	'' | *[!0-9.]*) fail "$1: compare failed on $4: $raw"; return ;;
	esac
	if [ "$5" = same ]; then
		[ "$n" = 0 ] && pass "$1: 0 px differ ($4 of $2 == $3)" ||
			fail "$1: $n px differ ($4 of $2, $3), expected no change"
	else
		[ "$n" = 0 ] && fail "$1: 0 px differ ($4 of $2 == $3), expected a change" ||
			pass "$1: $n px differ ($4 of $2 -> $3)"
	fi
}

key() { printf '  key   %-12s sleep %s\n' "$1" "$2"; xdotool key "$1"; sleep "$2"; }
typ() { printf '  type  %-12s sleep %s\n' "$1" "$2"; xdotool type --delay 60 "$1"; sleep "$2"; }
# tab N: the same key N times, one step at a time, so a focus reveal per step
# settles instead of racing the next keystroke.
tab() { printf '  key   Tab x%-8s sleep 1\n' "$1"; for _ in $(seq "$1"); do xdotool key Tab; sleep 0.2; done; sleep 1; }

printf '\nwindow %s = %s\n\n' "$W" "$(xdotool getwindowname "$W")"

# The first synthetic key after activation is swallowed while the X input focus
# settles, so warm the focus up before any A/B. The tab order starts at the
# header's Clear-changes button and wraps at both ends (see `move_focus`), so
# every state here is reached from a known one.
key Tab 1

# Everything below is compared against this state, so it has to be steady:
# async monitor discovery, the daemon probe and the auto-start attempt all
# land in the first seconds and would otherwise show up as a diff.
shot focus_none
printf '  wait  3 (let the boot-time async work land)\n'
sleep 3
shot settled
assert_pixels "window settles before any key is measured" focus_none settled same

key Tab 1
shot f1
assert_pixels "Tab moves focus onto Clear-changes" settled f1 differs

key Tab 1
shot f2
assert_pixels "Tab moves focus on again" f1 f2 differs

key shift+Tab 1
shot f1a
assert_pixels "Shift+Tab walks focus back where Tab came from" f2 f1a differs
assert_pixels "Shift+Tab returns to the same row Tab left" f1 f1a same

key Tab 1
shot f2b
assert_pixels "Tab is deterministic" f2 f2b same

key shift+Tab 1
shot f1b
assert_pixels "Shift+Tab is deterministic" f1a f1b same

# 'f' alone must do nothing: no control is a text field yet, so if this one
# changes the screen, the Ctrl+F check below proves nothing.
key f 1
shot plain_f
assert_pixels "plain f is inert" f1b plain_f same

key ctrl+f 1
shot ctrl_f
assert_pixels "Ctrl+F opens the search box" plain_f ctrl_f differs

typ anchor 1
shot typed
assert_pixels "typing filters the page" ctrl_f typed differs

key Escape 1
shot escape
assert_pixels "Escape leaves the search view" typed escape differs

key ctrl+2 2
shot ctrl_2
assert_pixels "Ctrl+2 scrolls to a section" escape ctrl_2 differs

# Focus held on a field row rather than a chrome button: the rows are what the
# keyboard user spends the run in, and the slider rows carry a track and a
# number input at once, so they are where an indicator painted over its own
# content shows. The tab order is Clear-changes, Reset all, Save, the five
# sidebar buttons, then the field rows in visual order (see `tab_order`), and
# focus still sits on Reset all here, so `offset x` — the first row with a
# slider — is Tab 12.
tab 12
shot slider_row

# Tab 13 is `offset y`: the next slider row, adjacent to this one and on
# screen with it, so the reveal leaves the page where it is and everything
# below the two rows is a fixed yardstick for "focus moved, nothing else did".
key Tab 1
shot slider_row_next
assert_pixels "Tab walks focus to the next slider row" slider_row slider_row_next differs
assert_region_pixels "focus moves the indicator, not the page" \
	slider_row slider_row_next 1216x300+176+520 same

# ------------------------------------------------------------------- summary ---
cat <<SUMMARY

  $PASS passed, $FAIL failed

  Not exercised by this harness:
    - No daemon runs, so only the GUI's own rendering is compared. That a
      keystroke reached the daemon (Save, Reset, Ctrl+R) is unproven here.
    - The overlay daemon is layer-shell only (ADR-004), so it cannot run on
      X11 at all and this harness never sees a surface.
    - Modifier delivery on a Wayland seat: the gap this harness exists for.
    - Enter and Space, which activate the focused row: no check above
      presses them.
    - A pixel diff says "something changed", never "the right thing changed":
      no assertion here reads what is drawn.

  screenshots: $SHOTS
  app log:     $ROOT/gui.log
SUMMARY

[ "$FAIL" -eq 0 ]