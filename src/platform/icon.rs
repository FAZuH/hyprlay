//! Bundled app icon: the brand mark (black rounded square, bold white "H")
//! as a scalable SVG plus the PNG/ICO raster sizes, all committed under
//! `assets/hyprlay.{svg,png,ico}`. This module exposes the iced window icon
//! built from the embedded 256px PNG (used by the settings GUI and the
//! Windows/macOS overlay surface), so no front re-decodes it and no
//! cross-front import is needed. The XDG theme install reuses the same
//! bundled bytes directly (see `service::systemd`).

use iced::window::icon::Icon;
use iced::window::icon::from_rgba;

/// The 256px raster side of the app icon, embedded at compile time.
const APP_ICON_PNG: &[u8] = include_bytes!("../../assets/hyprlay-256.png");

/// Build the iced window icon from the embedded 256px PNG. Panics on a bad
/// embedded asset or a decode failure — both are build bugs, not runtime
/// conditions (the asset is a committed, known-valid PNG).
pub fn window_icon() -> Icon {
    let img = image::load_from_memory(APP_ICON_PNG)
        .expect("embedded app icon is a valid PNG")
        .into_rgba8();
    let (width, height) = img.dimensions();
    from_rgba(img.into_raw(), width, height)
        .expect("embedded app icon decodes to valid RGBA dimensions")
}
