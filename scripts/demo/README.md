# GUI Demo Runbook

Run `./dev.sh demo-preflight` before a shoot. Run `./dev.sh demo` to build
the feature-gated demo binaries, switch to workspace 99, start a deterministic
four-person roster, and record the scene clips. The previous workspace is
restored when the command exits.

Use `HYPRLAY_DEMO_ROSTER=6` to change the roster size. Raw clips go to
`~/Videos/hyprlay-demo/raw` and delivery files go to `~/Videos/hyprlay-demo/out`.
Run `./dev.sh demo-gif` after reviewing the raw clips.
