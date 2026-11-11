//! Preview source preparation.

use rawoxide_core::thumbnail::{PreviewSource, noise_rgb, rgb};

#[test]
fn file_thumbnail_is_cropped_and_resized() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.png");
    let image = image::RgbImage::from_fn(40, 10, |x, y| {
        image::Rgb([(x * 6) as u8, (y * 20) as u8, 128])
    });
    image.save(&path).unwrap();

    let pixels = rgb(PreviewSource::File(&path), 16, 16, None).unwrap();
    assert_eq!(pixels.len(), 16 * 16 * 3);
    assert_eq!(
        pixels,
        rgb(PreviewSource::File(&path), 16, 16, None).unwrap()
    );
}

#[test]
fn synthetic_and_noise_sources_are_deterministic() {
    assert_eq!(
        rgb(PreviewSource::Synthetic, 8, 6, Some(9)).unwrap(),
        rgb(PreviewSource::Synthetic, 8, 6, Some(9)).unwrap()
    );
    assert_eq!(
        noise_rgb(8, 6, Some(9)).unwrap(),
        noise_rgb(8, 6, Some(9)).unwrap()
    );
    assert_ne!(
        noise_rgb(8, 6, Some(9)).unwrap(),
        noise_rgb(8, 6, Some(10)).unwrap()
    );
}

#[test]
fn unknown_file_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        rgb(
            PreviewSource::File(&dir.path().join("missing.png")),
            8,
            8,
            None
        )
        .is_err()
    );
}
