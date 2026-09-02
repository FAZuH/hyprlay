//! Windows resource build script: embeds the app icon into the `.exe`
//! resource so Explorer and the taskbar show the hyprlay mark.
//!
//! `winresource` shells out to the Windows Resource Compiler (`rc.exe`),
//! which exists only on a Windows host. Build scripts compile for the host
//! and target-specific build-dependencies resolve against the host, so the
//! whole block is gated `#[cfg(windows)]`: on a non-Windows host the crate is
//! neither built nor referenced, which keeps Linux cross-checks to
//! `windows-*` triplets green without needing `rc.exe`.
fn main() {
    #[cfg(windows)]
    {
        // Only meaningful when the target is also Windows (the resource is an
        // .exe icon). `rc.exe` is present on the Windows CI MSVC runner.
        let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
        if target_os == "windows" {
            if let Err(err) = winresource::WindowsResource::new()
                .set_icon("assets/hyprlay.ico")
                .compile()
            {
                panic!("failed to embed app icon resource: {err}");
            }
        }
    }
}
