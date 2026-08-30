#!/usr/bin/env bash

demo_key "CTRL+2"
demo_fullscreen
demo_record "s5-fullscreen" 6
hyprctl dispatch fullscreen 1 >/dev/null || true
sleep 1
demo_focus_gui
demo_key "CTRL+S"
demo_key "CTRL+3"
demo_record "s5-save" 5
