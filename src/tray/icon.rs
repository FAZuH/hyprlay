use image::GenericImageView;

/// Decode an embedded PNG (RGBA) into an ARGB32 [`ksni::Icon`], per the
/// ksni documentation's conversion (rgba → argb, network byte order).
pub fn load_icon(bytes: &[u8]) -> ksni::Icon {
    let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .expect("embedded tray icon must be a valid PNG");
    let (width, height) = img.dimensions();
    let mut data = img.into_rgba8().into_vec();
    for pixel in data.as_chunks_mut::<4>().0 {
        pixel.rotate_right(1); // rgba -> argb
    }
    ksni::Icon {
        width: width as i32,
        height: height as i32,
        data,
    }
}
