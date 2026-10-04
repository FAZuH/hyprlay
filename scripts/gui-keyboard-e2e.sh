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

# ---------------------------------------------------------------- the shape ---
# Everything above proves pixels changed. None of it can tell a fill from a
# stroke, because both change the window and both are "some blue". These four
# helpers name what is drawn instead, which is the only way to assert a shape.
#
# The palette the probes name, as the bytes the renderer writes them:
# `theme::FOCUS_FILL` 0.33/0.35/0.42 -> #54596b, `FIELD_BG` 0.16/0.17/0.20 ->
# #292b33, the disabled background 0.108/0.112/0.130 -> #1c1d21, `ACCENT`
# 0.345/0.396/0.949 -> #5865f2, `ACCENT_LIT` -> #5966e6.
FOCUS_FILL='#54596b'
FIELD_BG='#292b33'
DISABLED_BG='#1c1d21'
ACCENT='#5865f2'
ACCENT_LIT='#5966e6'
# The old focus ring (`theme::FOCUS_RING`, since deleted). No pixel of the
# window may still be this colour, which is what says the ring is gone rather
# than merely covered over.
FOCUS_RING='#99ccff'

# The header's three buttons all sit on row 23 of the capture, and each probe
# below is a patch of that button's body clear of its glyphs: Clear changes
# 1082..1215, Reset all 1226..1315, Save 1325..1384, and the first sidebar item
# 9..152 on row 68. Re-derive them after any layout change:
#   convert f1.png -crop 500x60+900+0 +repage -depth 8 txt: | grep -o '#.......'
# shows every run on a row, and the geometry of each run is the button.

# assert_pixel LABEL SHOT X Y HEX -- what is drawn at one point
assert_pixel() {
	local got
	got=$(convert "$SHOTS/$2.png" -format "%[hex:p{$3,$4}]" info: 2>>"$ROOT/import.log")
	# `%[hex:p{..}]` prints bare uppercase hex; the palette constants carry a
	# leading `#`, so put the two in the same shape before comparing.
	if [ "#${got,,}" = "$5" ]; then
		pass "$1: $got at ($3,$4) of $2"
	else
		fail "$1: $got at ($3,$4) of $2, expected $5"
	fi
}

# assert_no_stroke LABEL SHOT HEX -- no pixel of that colour anywhere
assert_no_stroke() {
	local n
	n=$(convert "$SHOTS/$2.png" -format %c histogram:info:- 2>>"$ROOT/import.log" |
		grep -ci "#$3")
	[ "$n" = 0 ] && pass "$1: no #${3} pixel anywhere in $2" ||
		fail "$1: $n #${3} pixels still in $2"
}

# label_peak SHOT GEOMETRY -- the brightest channel in a crop, 0-255. A dim
# disabled label peaks near 120 (#787a82, 0.47 grey) and a lifted one near 219
# (#dbdee0 = BRIGHT), so this reads "is the focus cue here" for a control that
# cannot take a fill.
label_peak() {
	# `-colorspace Gray` first: `maxima` is a per-channel value on an RGB
	# image, so without this it reports one arbitrary channel rather than the
	# label's luminance.
	convert "$SHOTS/$1.png" -crop "$2" +repage -colorspace Gray \
		-format "%[fx:int(255*maxima)]" info: 2>>"$ROOT/import.log"
}

# assert_label_lift LABEL FOCUSED_SHOT UNFOCUSED_SHOT GEOMETRY
# (three arguments after the label: the geometry is $4, not $5)
assert_label_lift() {
	local before after
	before=$(label_peak "$3" "$4")
	after=$(label_peak "$2" "$4")
	if [ "$before" -lt 160 ] && [ "$after" -gt 200 ]; then
		pass "$1: label peak $before -> $after"
	else
		fail "$1: label peak $before -> $after, expected dim then lifted"
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

# Two Tabs past Reset all reach the first sidebar item — the fifth of the nine
# chrome controls, and the one whose selected accent the focus fill takes.
key Tab 1
key Tab 1
shot nav_focus

# ------------------------------------------------- the indicator is a fill ---
# Focus is on "Reset all" in f1b, on Save in f2, on the first sidebar item in
# nav_focus, and on "Clear changes" in focus_none/settled — the config in
# $ROOT/home is clean, so that button has no press target and the run's first
# Tab lands on it disabled. Those four states cover the shapes below.

# An enabled chrome button fills, and the same button unfocused does not: a
# stroke would put the ring colour on the edge and leave the body at its idle
# background in both states.
assert_pixel "a focused enabled button fills" f1b 1233 23 "$FOCUS_FILL"
assert_pixel "the same button unfocused does not" f2 1233 23 "$FIELD_BG"
assert_no_stroke "no stroke survives on a focused button" f1b "$FOCUS_RING"

# Requirement: a disabled control must not wear the enabled cue. Its body is
# byte-identical focused and unfocused, so it cannot read as pressable — and
# the lifted label is the only thing left that says where focus is.
assert_pixel "a focused disabled button does not fill" focus_none 1090 23 "$DISABLED_BG"
assert_pixel "the same disabled button unfocused is identical" f1b 1090 23 "$DISABLED_BG"
assert_no_stroke "no stroke survives on the disabled button" focus_none "$FOCUS_RING"
assert_label_lift "focus on a disabled button lifts its label" \
	focus_none f1b 110x20+1090+13

# The primary button and the selected sidebar item are the two elements the
# accent is reserved for, and the fill takes the accent from both while
# focused. Named here because it is the visible cost of one shape everywhere.
assert_pixel "Save loses its accent while focused" f2 1332 23 "$FOCUS_FILL"
assert_pixel "Save keeps its accent unfocused" f1b 1332 23 "$ACCENT"
assert_pixel "the selected sidebar item loses its accent while focused" \
	nav_focus 90 68 "$FOCUS_FILL"
assert_pixel "the selected sidebar item is accent-lit unfocused" f2 90 68 "$ACCENT_LIT"

# The fill is a background, so it moves no pixel of layout: everything below the
# header is byte-identical with focus on one header button and on the next.
assert_region_pixels "focus moves the button, not the page" f1b f2 600x760+176+60 same

# Back to Reset all, so the checks below start from the state they expect.
key shift+Tab 1
key shift+Tab 1
shot back_to_reset
assert_pixels "focus walks back to where it came from" f1b back_to_reset same

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