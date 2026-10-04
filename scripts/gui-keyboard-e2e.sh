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

# The window's position on the root, read once: a screenshot is
# window-relative, and a click is not, so every coordinate below has to have
# the window's own offset added or the click lands somewhere else entirely.
eval "$(xdotool getwindowgeometry --shell "$W")"
WIN_X=${X:-0}
WIN_Y=${Y:-0}

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