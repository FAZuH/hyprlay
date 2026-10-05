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

# A credential pair, preloaded into the sandboxed config dir so the two
# Connection rows render text and are worth photographing. The two halves hold
# the SAME string on purpose: it makes "the secret is masked" a falsifiable
# pixel claim, because a revealed secret would render identically to the client
# id directly above it, and an identical pair is exactly what that check
# detects. Nothing is written back — Apply is the only path to auth.json, and
# it is never pressed here.
PROBE_ID='mmhke01hyprlayprobe'
mkdir -p "$ROOT/home/.config/hyprlay"
printf '{"client_id":"%s","client_secret":"%s"}' "$PROBE_ID" "$PROBE_ID" >"$ROOT/home/.config/hyprlay/auth.json"
chmod 600 "$ROOT/home/.config/hyprlay/auth.json"

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
		# XDG_CONFIG_HOME is pointed into $ROOT as well as HOME: `dirs::config_dir`
		# honours it first, so a stray value in the caller's environment would
		# otherwise read the real config and ignore the preloaded auth.json.
		env DISPLAY="$DISP" HOME="$ROOT/home" XDG_CONFIG_HOME="$ROOT/home/.config" \
			XDG_RUNTIME_DIR="$ROOT/run" LD_LIBRARY_PATH="$LIBPATH" \
			"$BIN" gui >"$ROOT/gui.log" 2>&1 &
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

# crop SHOT NAME GEOMETRY -- write one rectangle of a shot as its own image,
# so two regions of the SAME window can be compared against each other.
# `assert_region_pixels` applies one geometry to both of its shots, which can
# only say "this rectangle changed between two states"; a claim like "these two
# rectangles of one state differ from each other" needs both on disk first.
crop() {
	convert "$SHOTS/$1.png" -crop "$3" +repage "$SHOTS/crop-$2.png" 2>>"$ROOT/import.log" ||
		die "crop '$2' ($3 of $1) failed: $(tail -2 "$ROOT/import.log")"
	printf '  crop  %-22s %s\n' "$2" "$3"
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
# The content background a row with no fill of its own shows through: the theme
# background 0.118/0.121/0.133 -> #1e1f22. Needed by the credential probes,
# which assert on a row that is *not* focused.
PANEL_BG='#1e1f22'
# Windows the two search assertions below compare. `SEARCH_BOX` is the header
# field, where the typed text sat. `PAGE_BODY` is a patch of the content pane
# below the rows the search filtered away, so a cleared search puts real rows
# back there. Re-derive if the header or the one-pager's first rows move.
SEARCH_BOX='180x20+180+13'
PAGE_BODY='400x120+190+150'
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

# ink SHOT GEOMETRY -- the bright pixels in a rectangle, as a count. Where
# assert_pixel names one colour at one point, this counts what was drawn inside
# a shape: the glyphs of a number, the caret next to them, the border of a
# focused input. Two states of the same control compared with it say whether
# something appeared there, which a window-wide diff cannot.
ink() {
	convert "$SHOTS/$1.png" -crop "$2" +repage -colorspace Gray \
		-threshold 55% -format "%[fx:round(mean*w*h)]" info: 2>>"$ROOT/import.log"
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

# The window's position on the root, read once: a screenshot is
# window-relative, and a click is not, so every coordinate below has to have
# the window's own offset added or the click lands somewhere else entirely.
eval "$(xdotool getwindowgeometry --shell "$W")"
WIN_X=${X:-0}
WIN_Y=${Y:-0}

key() { printf '  key   %-12s sleep %s\n' "$1" "$2"; xdotool key "$1"; sleep "$2"; }
typ() { printf '  type  %-12s sleep %s\n' "$1" "$2"; xdotool type --delay 60 "$1"; sleep "$2"; }
# walk KEY N: the same key N times, one step at a time, so a focus reveal per
# step settles instead of racing the next keystroke.
walk() { printf '  key   %-12s x%-8s sleep 1\n' "$1" "$2"; for _ in $(seq "$2"); do xdotool key "$1"; sleep 0.2; done; sleep 1; }
tab() { walk Tab "$1"; }
# hold KEY SECONDS: the key stays down for SECONDS, which is what X11
# auto-repeat delivers to a client that leans on an arrow — a stream of further
# KeyPressed events. This Xvfb seat does repeat (a held Tab walks several
# targets), so the repeat is testable here rather than only in a unit test.
hold() {
	printf '  hold  %-12s %ss sleep 1\n' "$1" "$2"
	xdotool keydown "$1"
	sleep "$2"
	xdotool keyup "$1"
	sleep 1
}

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

# Escape with the search box holding the keyboard. This used to assert only that
# "something differs", which the caret disappearing satisfied on its own — so it
# passed while the search stayed applied. It now names what has to be true: the
# page is back to every field, which is the row the search filtered away, and the
# search box itself is empty again.
key Escape 1
shot escape
assert_region_pixels "Escape put back the rows the search had filtered away" \
	typed escape "$PAGE_BODY" differs
assert_pixels "Escape leaves the one-pager" typed escape differs
assert_region_pixels "and the search box is empty again, not just unringed" \
	ctrl_f escape "$SEARCH_BOX" same
assert_region_pixels "Escape restored the page exactly as it was before searching" \
	plain_f escape "$PAGE_BODY" same

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

# ------------------------------------------------------- the credential rows ---
# The Connection section is last on the page, so reaching it by Tab means
# walking every row above it. The count comes from the tab order itself rather
# than from counting on screen: three header actions, five sidebar items, then
# one press per row in `FIELDS` order. `offset y` — where focus sits now, one
# press past the "Tab 12" above — is the 7th row of 30, so 23 keyed rows remain
# before the credentials:
#   Tab 24 -> client id, Tab 25 -> client secret, Tab 26 -> the daemon toggle.
# `tab` presses one at a time so each row's reveal settles before the next
# keystroke; a burst would race the reveals and Tab into nothing.
tab 24
shot cred_id
tab 1
shot cred_secret

# The indicator lands ON the credential row, and it is the same one every other
# row takes. The probe sits in the row body's right-hand margin at x=1300,
# clear of the input's own frame and of every glyph, where an unfocused row
# shows the panel background through. Re-derive after any layout change with:
#   convert cred_id.png -crop 1396x120+0+690 -depth 8 txt: | grep -o '#.......'
# The two rows are at y 696..715 (client id) and y 749..768 (client secret);
# their text bands are y 721..736 and y 774..789.
assert_pixel "the focused client id takes the focus fill" cred_id 1300 705 "$FOCUS_FILL"
assert_pixel "the unfocused client secret below it does not" cred_id 1300 758 "$PANEL_BG"
assert_pixel "the focused client secret takes the focus fill" cred_secret 1300 758 "$FOCUS_FILL"
assert_pixel "the unfocused client id above it does not" cred_secret 1300 705 "$PANEL_BG"

# Focus moved between the two credentials and moved nothing else: the strip
# below them, which no reveal can shift from here (Connection is the last
# section and the page is already at its end), is byte-identical.
assert_region_pixels "Tab moves between the two credential rows" \
	cred_id cred_secret 1216x120+176+690 differs
assert_region_pixels "Tab moved the indicator, not the page" \
	cred_id cred_secret 1216x90+176+790 same

# Focus does not reveal the secret. Both halves of the pair were preloaded with
# the SAME string (see the top of this script), so an unmasked secret would
# render the client's own glyphs at the client's own position. Cropping each
# row's own text band and comparing the two is therefore a falsifiable claim
# about the mask: drop `.secure(true)` and the two crops match.
crop cred_id client_id_text 400x16+182+721
crop cred_secret client_secret_text 400x16+182+774
assert_pixels "the secret is masked, not rendered in the clear" \
	crop-client_id_text crop-client_secret_text differs

# And it stays masked once focused and once edited, which is the part a focus
# change could plausibly break. The bullet run is a fixed 35 non-background
# pixels while the id's 19 characters are 187; a revealed secret would land on
# the id's number, since the two hold the same string.
masked_glyphs() {
	convert "$SHOTS/$1" -crop "$2" +repage -format "%c" histogram:info:- |
		grep -Evi "#1E1F22|#54596B" | grep -c '#'
}
ID_GLYPHS=$(masked_glyphs cred_id.png 400x16+182+721)
SECRET_GLYPHS=$(masked_glyphs cred_secret.png 400x16+182+774)
if [ "$SECRET_GLYPHS" -lt "$ID_GLYPHS" ]; then
	pass "the secret stays masked while focused: $SECRET_GLYPHS glyph px vs the id's $ID_GLYPHS"
else
	fail "the secret is drawn in the clear while focused: $SECRET_GLYPHS glyph px vs the id's $ID_GLYPHS"
fi

# Shift+Tab walks back out of the pair onto the same row state Tab reached, so
# the credentials are reachable in BOTH directions. A credential Tab reaches
# and Shift+Tab does not strands a keyboard-only user on the row.
key shift+Tab 1
shot cred_shift_back
assert_pixels "Shift+Tab leaves the credential rows" cred_secret cred_shift_back differs
assert_region_pixels "Shift+Tab lands on the same state Tab reached" \
	cred_id cred_shift_back 1216x120+176+690 same

# Wrapping. The credentials are the last two rows of the order, so Tab forward
# off them lands on the daemon toggle (both rows unfocused behind it), and
# Shift+Tab from that toggle wraps back onto the last credential. Focus is on
# the client id here, so the way off the pair is two presses.
tab 2
shot cred_toggle
assert_pixel "Tab leaves the credentials for the daemon toggle" cred_toggle 1300 758 "$PANEL_BG"
assert_pixel "and the id row is unfocused behind it" cred_toggle 1300 705 "$PANEL_BG"
key shift+Tab 1
shot cred_wrapped
assert_pixel "Shift+Tab off the last target wraps onto the secret" cred_wrapped 1300 758 "$FOCUS_FILL"

# Typing into a focused credential, and Tab taking typing away again. Both
# fields are preloaded, so typing appends a character and the focused row's
# text band must gain glyphs; Tab out then typing must add none. That second
# half is the one that matters: a credential still holding iced's focus after
# the ring moved off would keep swallowing every later keystroke into a row
# the user is no longer looking at.
#
# Each comparison is within one row's own text band, so the focus fill moving
# between shots cannot satisfy any of them:
#   y 718..733 is the client id's band, y 771..786 the secret's.
key shift+Tab 1
shot cred_typing_on
typ z 1
shot cred_typing_typed
assert_region_pixels "typing into a focused credential edits the row" \
	cred_typing_on cred_typing_typed 400x16+182+718 differs

# Tab out of the pair entirely — two presses, since the secret is between the
# id and the toggle — and typing again must add nothing to either field.
key Tab 1
key Tab 1
shot cred_typing_off
typ z 1
shot cred_typing_off_typed
assert_region_pixels "typing after Tab adds nothing to the client id" \
	cred_typing_off cred_typing_off_typed 400x16+182+718 same
assert_region_pixels "typing after Tab adds nothing to the secret" \
	cred_typing_off cred_typing_off_typed 400x16+182+771 same

# The caret is the direct evidence that focus really left the input rather
# than the ring merely moving: a focused text input draws one, an unfocused one
# never does, whatever the blink phase. So the id band must lose the caret it
# had while focused — a 1x2 stroke at the end of the text, not another glyph.
assert_region_pixels "Tab took the caret off the client id" \
	cred_typing_typed cred_typing_off 400x16+182+718 differs

# The mouse still reaches the secret field, and clicking it does not unmask it.
# A click inside the input focuses it the same way Tab does, so the same
# glyph-count claim applies afterwards. (700, 778) in window coordinates lands
# in the middle of the secret input's text band.
printf '  click %-12s sleep 1\n' "secret field"
xdotool mousemove $((700 + WIN_X)) $((778 + WIN_Y)) click 1
sleep 1
shot cred_mouse
typ q 1
shot cred_mouse_typed
assert_region_pixels "the mouse still edits the secret field" \
	cred_mouse cred_mouse_typed 400x16+182+771 differs
MOUSE_SECRET_GLYPHS=$(masked_glyphs cred_mouse_typed.png 400x16+182+771)
if [ "$MOUSE_SECRET_GLYPHS" -lt "$ID_GLYPHS" ]; then
	pass "clicking the secret does not reveal it: $MOUSE_SECRET_GLYPHS glyph px vs the id's $ID_GLYPHS"
else
	fail "clicking the secret revealed it: $MOUSE_SECRET_GLYPHS glyph px vs the id's $ID_GLYPHS"
fi

# Tab must bring a credential row back into view even when nothing else on the
# page has. Tabbing to a row always passes a keyed row first, which reveals on
# its own account, so the credential reveal has nothing to do on that path —
# until the page moves under the ring. The page is at its end here (Connection
# is the last section), so scrolling UP walks the credential rows off the top
# with focus left where it is. Then Shift+Tab back onto the client id: only
# the reveal can pull it back, and an indicator parked off-screen with the user
# told nothing is the failure this catches.
key shift+Tab 1
shot cred_before_scroll
xdotool mousemove $((700 + WIN_X)) $((400 + WIN_Y))
for _ in 1 2 3 4 5 6 7 8 9 10; do xdotool click 4; done
sleep 1
shot cred_scrolled_away
assert_pixels "the wheel scrolls the credential rows off screen" \
	cred_before_scroll cred_scrolled_away differs
key shift+Tab 1
shot cred_revealed
assert_pixel "Shift+Tab brings the client id back into view" cred_revealed 1300 705 "$FOCUS_FILL"
assert_pixel "and the secret below it is unfocused" cred_revealed 1300 758 "$PANEL_BG"
# The reveal parks the page where the reveal asks for, not merely somewhere on
# screen. Compared against `cred_before_scroll` — the same page before the
# wheel moved it — everything BELOW the two credential rows is byte-identical,
# so the rows came back to the same offset on the page. (The rows themselves
# differ between the two shots: focus was on the secret before the scroll and
# is on the client id after, which is the fill moving, which is the point.)
assert_region_pixels "the reveal lands the rows where they were" \
	cred_before_scroll cred_revealed 1216x30+176+792 same

# Credentials are not config keys, so no reset reaches them: Ctrl+Shift+R is the
# one reset that could touch Connection at all, and both rows must come through
# it byte-identical. The daemon is down in this harness, so this proves the GUI
# had nothing to send that would rewrite them; the wire-level half — that no
# reset command can carry a credential at all — is pinned by
# `no_reset_command_carries_a_credential` in src/gui/commands.rs.
key ctrl+shift+r 2
shot cred_after_reset
assert_pixel "reset all leaves the client id focused and intact" \
	cred_after_reset 1300 705 "$FOCUS_FILL"
# The secret's bullet run is the content that must survive: a reset that
# reached a credential would clear the field, and an empty field is a shorter
# run of bullets. The client id is not compared on the same grounds because the
# caret sitting in it blinks, which is a pixel or two, not text.
assert_region_pixels "reset all leaves the secret's value alone" \
	cred_revealed cred_after_reset 400x16+182+771 same
assert_region_pixels "and the page where it stood" \
	cred_revealed cred_after_reset 1216x30+176+792 same

# ------------------------------------------------------ the option selects ---
# The rows that present a fixed set of choices: the corner presets, the anchor
# chips, the roster-order chips and the monitor chips. Enter, Space and Right
# pick the next option, Left the previous one, and the selection stops at either
# end rather than wrapping. Every check below probes the colour of the chip
# itself — accent for the selected one, the idle chip background for the rest —
# so it cannot be satisfied by the window merely changing.
#
# Focus is on the client id here, which is tab-order index 38: three header
# actions, five sidebar items, one press per keyed row, then the two credential
# rows. The corner presets are the first keyed row, index 8, so 30 Shift+Tabs
# walk back onto them, and the last of those reveals the row and parks the page
# 45 px down. Re-derive the geometries after any layout change, in this state:
#   magick opt_first.png -crop 1x220+700+40 -depth 8 txt:-   # the row's bands
#   magick opt_first.png -crop 500x1+600+90 -depth 8 txt:-    # a chip row's runs
# The presets row then spans y 55..147 with its fill, the chip grid on y 75..106
# and y 116..147, each chip half the content width: 176..773 on the left and
# 782..1379 on the right. The anchor row below has its three chips on y 180..207
# at x 176..231 (auto), 238..283 (top) and 291..367 (bottom). The probes sit
# clear of every chip's glyphs: x 700 and 1000 on the presets, x 180, 240 and
# 295 on the anchor row, and x 774 in the row's own fill margin between two
# chips. Nothing below the two rows (y 300..500) is touched by any step, which
# is what makes it the yardstick for "the choice moved, the page did not".
walk shift+Tab 30
shot opt_first
assert_pixel "the first preset is selected on a clean config" opt_first 700 90 "$ACCENT"
assert_pixel "and the other three are not" opt_first 1000 90 "$FIELD_BG"
assert_pixel "the focused row wears the fill behind its chips" opt_first 774 60 "$FOCUS_FILL"

# Left on the first option: the boundary. Nothing wraps, and nothing moves —
# not one pixel of the row, and not the window.
key Left 1
shot opt_first_boundary
assert_pixels "Left on the first option changes nothing" opt_first opt_first_boundary same
assert_region_pixels "and not one pixel of the row" \
	opt_first opt_first_boundary 1220x100+176+50 same
assert_pixel "the first preset is still the selected one" \
	opt_first_boundary 700 90 "$ACCENT"

# Enter steps to the next option, and the row keeps the fill: the chip moved,
# not the focus and not the page.
key Enter 1
shot opt_enter
assert_pixels "Enter selects the next option" opt_first_boundary opt_enter differs
assert_pixel "the next preset is now selected" opt_enter 1000 90 "$ACCENT"
assert_pixel "and the one Enter left is not" opt_enter 700 90 "$FIELD_BG"
assert_pixel "the last preset is still not selected" opt_enter 1000 130 "$FIELD_BG"
assert_pixel "focus stayed on the row" opt_enter 774 60 "$FOCUS_FILL"
assert_region_pixels "Enter moved the choice, not the page" \
	opt_first_boundary opt_enter 1220x200+176+300 same

# Right steps to the next option, which is the chip below-and-left here: the
# order the arrow walks is the order the 2x2 grid paints.
key Right 1
shot opt_right
assert_pixel "Right selects the next option" opt_right 700 130 "$ACCENT"
assert_pixel "and the one before it is unselected" opt_right 1000 90 "$FIELD_BG"

# Left walks the same row back.
key Left 1
shot opt_left
assert_pixel "Left selects the previous option" opt_left 1000 90 "$ACCENT"
assert_pixel "and the one it left is unselected" opt_left 700 130 "$FIELD_BG"

# A held key repeats, and stops on the last option. Held from top-right for
# three seconds: if the repeat were ignored the row would sit on bottom-left,
# and if the last option wrapped it would be back on top-left — so the
# bottom-right chip being the accent one says both.
hold Right 3
shot opt_held
assert_pixel "a held Right repeats the step" opt_held 1000 130 "$ACCENT"
assert_pixel "the option before the last one is not selected" opt_held 700 130 "$FIELD_BG"
assert_pixel "and the selection did not wrap to the first one" opt_held 700 90 "$FIELD_BG"
assert_region_pixels "a held key moved the choice, not the page" \
	opt_left opt_held 1220x200+176+300 same

# Held at that last option it stops there: the boundary holds under repeat too.
hold Right 3
shot opt_held_stop
assert_pixels "a held Right on the last option changes nothing" opt_held opt_held_stop same
assert_region_pixels "and not one pixel of the row" \
	opt_held opt_held_stop 1220x100+176+50 same
assert_pixel "the last preset is still the selected one" opt_held_stop 1000 130 "$ACCENT"

# A second row of the same shape, three chips in a line: Tab reaches it, the
# presets row unfocuses behind it, and Right moves one chip along.
key Tab 1
shot opt_anchor
assert_pixel "Tab reaches the anchor row" opt_anchor 180 185 "$ACCENT"
assert_pixel "and the presets row is unfocused behind it" opt_anchor 700 90 "$FIELD_BG"
key Right 1
shot opt_anchor_step
assert_pixel "Right moves the anchor one chip along" opt_anchor_step 240 185 "$ACCENT"
assert_pixel "the chip it left is unselected" opt_anchor_step 180 185 "$FIELD_BG"
assert_pixel "and so is the one it did not reach" opt_anchor_step 295 185 "$FIELD_BG"
assert_region_pixels "the presets row above did not move" \
	opt_anchor opt_anchor_step 1220x100+176+50 same

# Left on the first anchor chip is that row's boundary too, and it stops the
# same way: the second press above left auto behind, so one Left returns to it
# and a second changes nothing.
key Left 1
shot opt_anchor_back
key Left 1
shot opt_anchor_first
assert_pixel "Left walks the anchor back to auto" opt_anchor_back 180 185 "$ACCENT"
assert_pixels "and Left on it again stops at the boundary" \
	opt_anchor_back opt_anchor_first same
assert_pixel "with auto still selected" opt_anchor_first 180 185 "$ACCENT"

# ----------------------------------------------------------- the number rows ---
# The rows that hold a number instead of a fixed set of choices. The arrow keys
# step the value, the input shows it, and Enter moves the keyboard into that
# input so a value can be typed instead.
#
# One row carries all of it: `spacing`, which is 0..=24 with 4 on a clean
# config. It is reached from the anchor row above, and the keyed rows render in
# `FIELDS` order (see `tab_order`), so `spacing` is the 20th keyed row and
# `anchor` the 2nd: 18 Tabs. Re-derive everything below after any layout
# change, with the focused row parked by the reveal:
#   magick num_4.png -crop 1210x1+176+397 -depth 8 txt:-   # one row's runs
#   magick num_4.png -crop 80x24+1270+394 -filter point -resize 400% png:-
# In that state the row's band is y 375..416: the slider's track runs x 176..1265
# with its handle on it, the input box is x 1273..1345 (border at x 1274) and the
# reset button x 1354..1379. One step of `spacing` is 1/24 of the track, about
# 45 px, so the handle is unmistakable at either end of a press: it sits at
# x 362 for the value 4, 407 for 5, 317 for 3, 182 for 0, 1258 for 24 and 1079
# for a typed 20. The probe line is y 397, one pixel ABOVE the 5 px track, which
# is why the handle is the only accent there — everywhere else on that line is
# the row's own focus fill (#54596b).
#
# Two rectangles carry the claims, and each comparison below is over one of them
# rather than over the window: the track alone (`SLIDER`) for where the handle
# is, and the digits alone (`INPUT_DIGITS`) for what the input says. Both are
# cropped tight on purpose. The row band that holds them both also holds the
# input's border, and that border changes for two reasons that have nothing to
# do with the number: its rounded antialiasing wobbles by one in a channel
# whenever the row re-renders, and a focused input paints a different border
# colour than an idle one. An "these are identical" claim over the box would
# then be a claim about the focus state wearing a number's clothes, so no such
# comparison is made over it. The strip below the row — the `overall` row's
# handle at (1258, 619) — is the yardstick for "the value moved, the page did
# not".
SLIDER='1089x16+176+395'
INPUT_BOX='74x22+1273+395'
INPUT_DIGITS='13x9+1280+401'
# Wider than INPUT_DIGITS, so an appended or cleared digit is inside the crop.
# It includes the caret, so only compare two shots that both have one — use
# INPUT_DIGITS for "unchanged" claims, which must not notice the caret blinking.
INPUT_FIELD='44x14+1276+398'
LEFT_END='30x16+176+395'
RIGHT_END='30x16+1236+395'
YARDSTICK='30x20+1244+610'
# The two colours this section reads beyond the shared palette: the handle on
# the focused row, and the border a text input wears while it holds typing.
HANDLE='#6b75ff'
INPUT_FOCUSED='#6770f2'
INPUT_IDLE='#40424a'

tab 18
shot num_4

# The handle is where the value 4 of 24 puts it. The number beside it is not
# probed here — it is what the steps below are read off, one press at a time.
assert_pixel "the row's slider handle sits where the value 4 of 24 puts it" \
	num_4 362 397 "$HANDLE"

# Right steps up by one. The handle leaves the spot it was on and arrives one
# step along, the input's own box changes (the number), and the row below does
# not move at all — a whole-window diff cannot say which of the three happened.
key Right 1
shot num_5
assert_pixel "Right moved the handle one step along the track" num_5 407 397 "$HANDLE"
assert_pixel "and off the spot it was at" num_5 362 397 "$FOCUS_FILL"
assert_region_pixels "the number in the row's input changed" num_4 num_5 "$INPUT_BOX" differs
assert_region_pixels "the row below did not move" num_4 num_5 "$YARDSTICK" same

# Left walks it back, and the row comes back to exactly where it stood: the
# handle and the digits are both byte-identical to the state before the step,
# which is what says it returned to the value 4 and not to some other one.
key Left 1
shot num_4_back
assert_region_pixels "Left put the handle back where it was" num_4 num_4_back "$SLIDER" same
assert_region_pixels "and the number with it" num_4 num_4_back "$INPUT_DIGITS" same

# Up and Down are the same step: Down is the direction Left went, and Up the one
# Right went, so both return the row to its own pixels.
key Down 1
shot num_3
assert_pixel "Down moved the handle the other way" num_3 317 397 "$HANDLE"
assert_pixel "and left the spot Right had reached" num_3 407 397 "$FOCUS_FILL"
key Up 1
shot num_up_back
assert_region_pixels "Up put the handle back where Right did" num_4 num_up_back "$SLIDER" same
assert_region_pixels "and the number with it" num_4 num_up_back "$INPUT_DIGITS" same

# The bounds. `spacing` is 0..=24, so four steps reach the minimum and the handle
# has to reach the left end of the track with them. The step that would leave the
# range sends nothing at all: the window is byte-identical, so no pixel of it —
# handle, number or anything else — moved.
walk Left 4
shot num_0
assert_pixel "four steps down put the handle at the left end of the track" \
	num_0 182 397 "$HANDLE"
assert_region_pixels "and the number with it" num_4 num_0 "$INPUT_BOX" differs
key Left 1
shot num_0_stop
assert_pixels "a step off the minimum changes nothing at all" num_0 num_0_stop same

# The other end: the steps up reach the maximum, where the handle is at the right
# end of the track, and a further step stops the same way.
walk Right 30
shot num_24
assert_pixel "the steps up put the handle at the right end of the track" \
	num_24 1258 397 "$HANDLE"
assert_region_pixels "and the number with it" num_0 num_24 "$INPUT_BOX" differs
key Right 1
shot num_24_stop
assert_pixels "a step off the maximum changes nothing at all" num_24 num_24_stop same

# Enter hands the keyboard to the row's own input, which is the one widget here
# that can take real focus — the input paints a different border while it holds
# typing, and that border is the evidence. Its pixels come from an effect, so the
# key needs its usual settle before anything is typed into it.
key Enter 1
shot num_enter
assert_pixel "Enter gave the row's input the keyboard" num_enter 1274 400 "$INPUT_FOCUSED"
assert_pixel "and an unfocused input does not wear that border" num_4 1274 400 "$INPUT_IDLE"
# The caret is drawn inside a focused input and never inside an unfocused one, so
# the bright pixels of the box grow by the caret alone: the border pixels are
# counted in both states and the digits have not moved.
CARET_BEFORE=$(ink num_4 "$INPUT_BOX")
CARET_AFTER=$(ink num_enter "$INPUT_BOX")
if [ "$CARET_AFTER" -gt "$CARET_BEFORE" ]; then
	pass "Enter draws a caret in the input: $CARET_BEFORE -> $CARET_AFTER bright px"
else
	fail "Enter left the input without a caret: $CARET_BEFORE -> $CARET_AFTER bright px"
fi

# What is typed applies, and the row shows it: select the value away and type
# another one in its place. "20" is inside 0..=24, so the handle has to leave the
# right end of the track and arrive where 20 of 24 puts it, and the input has to
# show two digits where it showed one.
key ctrl+a 1
typ 20 1
shot num_typed
assert_pixel "the typed value put the handle at 20 of 24" num_typed 1079 397 "$HANDLE"
assert_region_pixels "and it left the end of the track the 24 was on" \
	num_24 num_typed "$RIGHT_END" differs
assert_region_pixels "and the input now shows the typed number" \
	num_24 num_typed "$INPUT_BOX" differs

# The commit itself: the value was applied as it was typed, so what Enter has
# left to do is hand the keyboard back to the row — which is what lets the next
# Enter go into the input again.
key Enter 1
shot num_committed
assert_pixel "Enter handed the keyboard back to the row" num_committed 1274 400 "$INPUT_IDLE"
assert_region_pixels "and left the committed number on the row" \
	num_typed num_committed "$INPUT_DIGITS" same
assert_region_pixels "with the handle where it was applied" \
	num_typed num_committed "$SLIDER" same

# A value the bounds refuse is kept as typed and applied to nothing. The row
# applies every value the bounds allow the moment it is typed, so the 99 below
# is typed over a committed 9 and stops being a value on its second digit — and
# the 9 is committed first, so the rendering the row has to put back after Escape
# exists on disk as an idle input rather than as an assumption.
key Enter 1
key ctrl+a 1
typ 9 1
key Enter 1
shot num_nine
assert_pixel "the 9 committed and the keyboard went back to the row" num_nine 1274 400 "$INPUT_IDLE"
assert_region_pixels "and the row shows the 9 it committed" num_typed num_nine "$INPUT_DIGITS" differs

key Enter 1
key ctrl+a 1
typ 99 1
shot num_refused_text
assert_pixel "the refused text sits in the input, which still holds the keyboard" \
	num_refused_text 1274 400 "$INPUT_FOCUSED"
assert_region_pixels "and it is a digit wider than the value it was typed over" \
	num_nine num_refused_text "$INPUT_DIGITS" differs
assert_region_pixels "while the value it refused moved nothing" \
	num_nine num_refused_text "$SLIDER" same

# Enter on that refuses it: the text stays, the caret stays, and the value is
# exactly what it was before the refusal.
key Enter 1
shot num_refused
assert_region_pixels "Enter refused it and kept the text" \
	num_refused_text num_refused "$INPUT_DIGITS" same
assert_region_pixels "and applied nothing on top of what the row had" \
	num_nine num_refused "$SLIDER" same
assert_pixel "with the input still holding the keyboard" num_refused 1274 400 "$INPUT_FOCUSED"

# Escape hands the keyboard back to the row and *keeps* what was typed, the same
# as Tab and Enter: nothing in a number row discards typed text. The draft here
# is `99`, which `spacing` (0..=24) refuses, so it cannot become the row's value
# and the slider must not move — but it is still the user's half-typed number,
# and it stays in the box for them to finish.
key Escape 1
shot num_escape
assert_pixel "Escape took the keyboard off the input" num_escape 1274 400 "$INPUT_IDLE"
assert_region_pixels "Escape kept the refused draft rather than discarding it" \
	num_refused_text num_escape "$INPUT_DIGITS" same
assert_region_pixels "and a value the bounds refuse still cannot land" \
	num_nine num_escape "$SLIDER" same

# Escape kept a draft the bounds refuse, so the row is now holding unapplicable
# text: Tab means "commit", and committing this refuses, so Tab stays put and says
# why. That is the escape hatch working — Escape got you out of the input, and the
# row asks to be fixed rather than silently losing what you typed.
key Tab 1
shot num_tab_refused
assert_pixel "Tab stays on the row while its text is still not a value" \
	num_tab_refused 900 390 "$FOCUS_FILL"

# Fixing the text needs the keyboard back in the input first: Escape parked the
# draft but handed typing to the row, and the box still renders the draft, so Enter
# resumes exactly where the user left off rather than starting over.
key Enter 1
shot num_resumed
assert_pixel "Enter took the keyboard back into the input" \
	num_resumed 1274 400 "$INPUT_FOCUSED"
assert_region_pixels "with the parked draft still in the box" \
	num_escape num_resumed "$INPUT_DIGITS" same

# Typing appends, so typing `8` over `99` gives `998` — still out of bounds, and
# still refused. That is the input behaving like an input, so fixing a mistyped
# number means clearing it first, exactly as it would with the mouse.
typ 8 1
shot num_appended
assert_region_pixels "an appended digit makes the text worse, not better" \
	num_resumed num_appended "$INPUT_FIELD" differs
assert_region_pixels "so the refused value still cannot land" \
	num_nine num_appended "$SLIDER" same

# Four backspaces clears `998` and one more does nothing, which is cheaper than
# counting the digits: an over-long backspace run is not an error in a text field.
for _ in 1 2 3 4; do key BackSpace 0.4; done
typ 8 1
shot num_fixed
assert_region_pixels "clearing and retyping applies the number" \
	num_appended num_fixed "$INPUT_FIELD" differs
assert_region_pixels "and the handle moves to match" \
	num_nine num_fixed "$SLIDER" differs

# Tab out of a row whose text has all been committed is an ordinary move: the
# ring leaves this row for the next one, and the keyboard goes with it.
key Tab 1
shot num_tab
assert_pixel "Tab took the ring off the number row" num_tab 900 390 "$PANEL_BG"
assert_pixel "and put it on the next row" num_tab 900 443 "$FOCUS_FILL"

# ------------------------------------------------------ R restores the field ---
# Plain R restores the setting the ring is on to its default and nothing else.
# Every check here is a claim about the VALUE, not about movement: each one
# compares the row against the state it stood in before the row was edited,
# so "the value came back to its default" and "the value moved" cannot both be
# satisfied by the same pixels. The four row kinds are checked separately
# because a flag, an option select, a number and a colour are four renderers,
# and one probe would stand for none of the other three.
#
# No daemon runs, so nothing here proves the daemon was told: the command and
# its wire text are pinned at the Command seam instead, by
# r_sends_the_reset_command_for_the_focused_key and
# a_per_key_reset_names_the_key_own_group in src/gui/.
#
# The rows are reached by Tab from `num_tab`, where the ring is on `max name
# length`. Keyed rows run in FIELDS order — corner preset, anchor, right-to-left,
# the four offset rows, monitor, visible, auto-save, show-over-fullscreen,
# dim-on-hover, talking-only, show-own-user, roster-order, width, scale,
# avatar-size, text-size, spacing, max-name-length, max-rows, the five opacities,
# then the three colours — so `spacing` is one Shift+Tab back, `talking-only`
# seven after that, the `anchor` chips eleven more, and `username background
# color` twenty-eight Tab forward from the anchor row.
#
# One rectangle per row carries the value claim, cropped on that row's own
# control and clear of the row's fill and its border (the same reason the number
# section crops two separate rectangles). Re-derive after any layout change,
# with the ring parked on the row:
#   magick <shot>.png -crop 1216x1+176+<row y> -depth 8 txt:-   # every run
# A toggler's knob rests at x 178..189 with its flat interior at x 181..188; the
# crop is that interior and nothing else, because the knob's antialiased rim
# wobbles by one in a channel whenever the row re-renders and an "identical"
# claim over the rim would be a claim about the repaint. An anchor chip row
# spans x 177..370, and a hex input's interior spans x 211..319.
FLAG_VALUE='8x11+181+77'
CHIP_VALUE='200x24+176+76'
HEX_VALUE='109x20+211+598'
# The client id's own text band, reused from the credential section above.
CRED_ID_TEXT='400x16+182+718'

# The number row first, and it reuses the geometry the section above derived.
# Shift+Tab lands the ring back on `spacing`, whose value is 8 (typed above), so
# the arrow below moves it further off its default and R brings it all the way
# back. `num_4` is the state this row stood in at its default earlier in the
# run — the same ring, the same page offset — so comparing against it is a
# claim about the VALUE and not about "something moved".
key shift+Tab 1
shot r_num_before
key Right 1
shot r_num_moved
assert_region_pixels "the arrow moved the number off its default" \
	num_4 r_num_moved "$SLIDER" differs
key r 1
shot r_num_reset
assert_pixel "R put the handle back where the default 4 puts it" \
	r_num_reset 362 397 "$HANDLE"
assert_pixel "and off the spot the arrow had reached" r_num_reset 407 397 "$FOCUS_FILL"
assert_region_pixels "R put the number back to its default, not merely somewhere else" \
	num_4 r_num_reset "$SLIDER" same
assert_region_pixels "and the digits with it" num_4 r_num_reset "$INPUT_DIGITS" same
assert_region_pixels "R changed the row and not the page" \
	r_num_moved r_num_reset "$YARDSTICK" same

# The flag: `talking-only` is off on a clean config, so the toggle starts at the
# default and Space flips it. The knob slides across the track and the track
# changes colour with it, which is what the rectangle above watches.
walk shift+Tab 7
shot r_flag_before
key space 1
shot r_flag_on
assert_region_pixels "the toggle flipped away from its default" \
	r_flag_before r_flag_on "$FLAG_VALUE" differs

# The search box holds the keyboard here, and a query containing `r` must not
# reset the row behind it. iced's keyboard listener delivers only "ignored"
# events, so a key typed into a focused input never reaches the shortcut
# dispatcher at all; this is that property end to end, on a row whose value is
# demonstrably off its default, so a reset would show in the toggle.
key ctrl+f 1
typ rr 1
shot r_flag_query
assert_region_pixels "the query landed in the search box" \
	r_flag_on r_flag_query "$SEARCH_BOX" differs
key Escape 1
shot r_flag_escaped
assert_region_pixels "and the row behind the search box was never reset" \
	r_flag_on r_flag_escaped "$FLAG_VALUE" same
key r 1
shot r_flag_reset
assert_region_pixels "R put the flag back to its default" \
	r_flag_before r_flag_reset "$FLAG_VALUE" same
assert_region_pixels "and the flip is what changed, not the reset" \
	r_flag_on r_flag_reset "$FLAG_VALUE" differs

# The option select: `anchor` is `auto` on a clean config, Right steps it to
# `top`, and R must bring the selection back to `auto` — the same walk the
# arrows take, backwards onto the default. The probes sit inside the two chip
# bodies and clear of their glyphs.
walk shift+Tab 11
shot r_chip_before
key Right 1
shot r_chip_moved
assert_pixel "the arrow moved the selection off its default" r_chip_moved 240 82 "$ACCENT"
key r 1
shot r_chip_reset
assert_pixel "R put the selection back on its default chip" r_chip_reset 180 82 "$ACCENT"
assert_pixel "and off the chip the arrow had reached" r_chip_reset 240 82 "$FIELD_BG"
assert_region_pixels "R put the whole chip row back to its default" \
	r_chip_before r_chip_reset "$CHIP_VALUE" same
assert_region_pixels "the step is what changed it, not the reset" \
	r_chip_moved r_chip_reset "$CHIP_VALUE" differs

# The colour row: `username background color` is #0d0d0f on a clean config.
# A colour has no keyboard path to a new value yet (Enter on a colour row rings
# it and stops — see `activate_focus`), so the mouse edits the hex input and
# Escape hands typing back to the row, which is what leaves R reachable at all.
# The hex field is the value: its text IS the hex the daemon would be told, so
# the claim is read off the digits rather than off the swatch beside them.
walk Tab 28
shot r_hex_focused
# The baseline is taken AFTER a click-and-Escape round trip into the hex input,
# because that round trip repaints the glyphs: comparing a post-edit shot with
# one taken before the first click would be a claim about the repaint rather
# than about the value. So the default is photographed in exactly the state the
# edit below leaves the input in, and R has to put the field back to *that*.
click_hex() {
	printf '  click %-12s sleep 1\n' 'hex input'
	xdotool mousemove $((263 + WIN_X)) $((605 + WIN_Y)) click 1
	sleep 1
}
click_hex
key Escape 1
shot r_hex_before
click_hex
key ctrl+a 1
typ '#f00f0f' 1
key Escape 1
shot r_hex_typed
assert_region_pixels "the hex field took the typed colour" \
	r_hex_before r_hex_typed "$HEX_VALUE" differs
key r 1
shot r_hex_reset
assert_region_pixels "R put the hex field back to its default" \
	r_hex_before r_hex_reset "$HEX_VALUE" same
assert_region_pixels "and the typed colour is what changed, not the reset" \
	r_hex_typed r_hex_reset "$HEX_VALUE" differs

# The credentials: a credential row edits no config key, so R resets nothing —
# and the keystroke is the input's, which is the stronger claim. Preloading made
# the client id a fixed string, so the glyphs in its text band are the witness
# that the letter went into the field instead of reaching a reset.
key Tab 1
shot r_cred_before
key r 1
shot r_cred_after
assert_region_pixels "R on a credential row types into the credential" \
	r_cred_before r_cred_after "$CRED_ID_TEXT" differs
assert_region_pixels "and the secret below it is untouched" \
	r_cred_before r_cred_after '400x16+182+771' same

# No field at all. A search that filters out the focused row drops the ring —
# there is nothing left to ring — and clearing the search leaves it dropped, so
# R now reaches a window with no focused field and must change nothing.
key ctrl+f 1
typ color 1
key Escape 1
shot r_none
key r 1
shot r_none_after
assert_pixels "R with no field focused changes nothing at all" r_none r_none_after same

# ------------------------------------------------------------------- summary ---
cat <<SUMMARY

  $PASS passed, $FAIL failed

  Not exercised by this harness:
    - No daemon runs, so only the GUI's own rendering is compared. That a
      keystroke reached the daemon (Save, Reset, Ctrl+R) is unproven here —
      and that includes every step's own command: what the arrow keys send is
      asserted at the Command seam instead, by
      an_option_step_sends_the_neighbouring_choice,
      an_option_step_at_either_end_sends_nothing,
      a_number_step_sends_the_neighbouring_value,
      a_number_step_stays_inside_the_shared_bounds and
      a_held_number_step_comes_to_rest_and_stops_there in src/gui/commands.rs.
      What R sends is asserted the same way, by
      r_sends_the_reset_command_for_the_focused_key in src/gui/update.rs and
      a_per_key_reset_names_the_key_own_group in src/gui/commands.rs: the
      socket half of a reset is UNPROVEN here, in this harness and in every
      other test in it.
    - Editing a colour from the keyboard. A colour row offers no way to type a
      new value (Enter on one rings it and stops — see `activate_focus`), so
      the hex field above is driven with the mouse and the reset is what is
      under test. Every other row kind is driven with the keyboard alone.
    - The status-bar refusal. A refused value answers with the daemon's own
      wording ("error: spacing <0-24>"), but the 2 s status probe overwrites the
      status line with its own answer before a screenshot can be certain of
      catching it, so the refusal's text is asserted by
      a_refused_commit_answers_the_error_and_keeps_the_caret in
      src/gui/update.rs and not here. What the harness does check is the rest of
      it: the value stands and the caret stays.
    - The overlay daemon is layer-shell only (ADR-004), so it cannot run on
      X11 at all and this harness never sees a surface.
    - Modifier delivery on a Wayland seat: the gap this harness exists for.
    - Space. Enter steps the option selects the same step the arrows do (see
      activate_focus in src/gui/update.rs), and on a number row it is the way
      into the input, so nothing here presses Space itself.
    - The roster-order and monitor chip rows. They are the same code path as
      the two rows above; the roster-order row is 14 Tabs further down the
      page and the monitor row has only one chip here, because this seat
      reports no outputs.
    - Every number row but the spacing row. It stands for all of them: they share
      one row renderer, one step and one bounds table, so what it shows is what
      the offsets, the widths and the sizes do. The two rows named "offset
      slider minimum" and "offset slider maximum" render no slider at all, so
      there is no handle on those two rows to watch.
    - A pixel diff says "something changed", never "the right thing changed":
      an assertion that reads the drawn colour (assert_pixel, assert_label_lift)
      is the only kind that can.

  screenshots: $SHOTS
  app log:     $ROOT/gui.log
SUMMARY

[ "$FAIL" -eq 0 ]