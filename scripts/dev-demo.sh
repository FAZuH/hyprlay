#!/usr/bin/env bash

# Local, GUI-only product demo. This module is discovered by dev.sh and is
# intentionally not part of the shared project-ops development baseline.

DEMO_MODULE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEMO_ROOT="$(dirname "$DEMO_MODULE_DIR")"

dev_desc demo "Build and record the Hyprland GUI demo on workspace 99"
dev_desc demo-preflight "Check GUI demo recording dependencies"
dev_desc demo-gif "Build the demo MP4 and hero GIF from recorded scenes"

cmd_demo_preflight() {
    local missing=0
    local command
    for command in hyprctl cargo wf-recorder ffmpeg slurp grim ydotool wtype jq; do
        if ! command -v "$command" >/dev/null 2>&1; then
            err "Missing demo dependency: $command"
            missing=1
        fi
    done
    if ! systemctl --user is-active --quiet ydotool; then
        err "ydotool user service is not active"
        missing=1
    fi
    if ! hyprctl monitors >/dev/null 2>&1; then
        err "Hyprland is not reachable"
        missing=1
    fi
    if (( missing )); then
        return 1
    fi
    scs "GUI demo preflight passed"
}

cmd_demo() {
    source "$DEMO_MODULE_DIR/demo/lib.sh"
    cmd_demo_preflight
    demo_build
    demo_stage
    trap 'demo_cleanup' EXIT INT TERM
    demo_start_daemon
    demo_run_scenes
    demo_cleanup
    trap - EXIT INT TERM
    scs "Demo scenes recorded in $DEMO_RAW_DIR"
}

cmd_demo_gif() {
    source "$DEMO_MODULE_DIR/demo/lib.sh"
    demo_build_deliverables
}
