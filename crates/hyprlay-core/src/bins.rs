pub const DAEMON_BIN: &str = "hyprlayd";
pub const CLI_BIN: &str = "hyprlay";
/// Wayland application ID for the settings window. Must match the
/// `application_id` set in `iced::window::Settings::platform_specific` and the
/// class used by `hyprctl dispatch focuswindow`.
pub const GUI_APP_ID: &str = "hyprlay-gui";
/// Lock name for the GUI single-instance guard (`XDG_RUNTIME_DIR/<name>.lock`).
pub const GUI_LOCK: &str = "hyprlay-gui";
