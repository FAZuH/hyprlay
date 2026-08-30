#!/usr/bin/env bash

demo_key "CTRL+2"
demo_record "s4-layout" 5
demo_focus_gui
demo_key "CTRL+F"
demo_type "opacity"
demo_record "s4-search" 4
demo_focus_gui
demo_key "ESC"
demo_key "CTRL+2"
"$DEMO_TARGET_DIR/release/hyprlay" set dim-on-hover >/dev/null 2>&1 || true
demo_cursor 1450 100
demo_cursor 1400 420
demo_cursor 1450 100
demo_record "s4-hover" 5
