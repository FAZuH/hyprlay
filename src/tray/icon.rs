use image::GenericImageView;

/// A decoded tray icon as a pure RGBA8 buffer plus dimensions. This is the
/// only shape the tray front carries; each backend adapter converts it to its
/// native icon type (`ksni::Icon`, `tray_icon::Icon`).
#[derive(Debug, Clone)]
pub struct IconData {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// RGBA8 pixels (`[r, g, b, a]` per pixel).
    pub rgba: Vec<u8>,
}

impl IconData {
    /// ARGB32 in network byte order (`[a, r, g, b]` per pixel) — what `ksni`
    /// expects (rgba → argb, per the ksni documentation).
    pub fn argb32(&self) -> Vec<u8> {
        let mut data = self.rgba.clone();
        for pixel in data.as_chunks_mut::<4>().0 {
            pixel.rotate_right(1); // rgba -> argb
        }
        data
    }
}

/// Decode an embedded PNG (RGBA) into a pure [`IconData`]. Fails loudly on an
/// invalid embedded asset — the input is a compile-time bundled file, so a
/// decode error is a build bug, not a runtime condition.
pub fn load_icon(bytes: &[u8]) -> IconData {
    let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .expect("embedded tray icon must be a valid PNG");
    let (width, height) = img.dimensions();
    IconData {
        width,
        height,
        rgba: img.into_rgba8().into_vec(),
    }
}
