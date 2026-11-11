//! Plain TIFF cover generation.

use rawoxide_core::write_cover_tiff;

#[test]
fn writes_an_rgb_tiff_cover() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cover.tif");
    let size = write_cover_tiff(&path, 8, 6, Some(3), "RawOxide test").unwrap();
    assert!(size > 0);
    let file = tiff_reader::TiffFile::open(&path).unwrap();
    let ifd = file.ifd(0).unwrap();
    assert_eq!(
        ifd.tag(256).map(|tag| &tag.value),
        Some(&tiff_reader::TagValue::Long(vec![8]))
    );
    assert_eq!(
        ifd.tag(257).map(|tag| &tag.value),
        Some(&tiff_reader::TagValue::Long(vec![6]))
    );
    assert_eq!(
        ifd.tag(277).map(|tag| &tag.value),
        Some(&tiff_reader::TagValue::Short(vec![3]))
    );
    let first = file.read_image_bytes(0).unwrap();
    assert_eq!(first.len(), 8 * 6 * 3);

    let second_path = dir.path().join("cover2.tif");
    write_cover_tiff(&second_path, 8, 6, Some(3), "RawOxide test").unwrap();
    let second = tiff_reader::TiffFile::open(&second_path)
        .unwrap()
        .read_image_bytes(0)
        .unwrap();
    assert_eq!(first, second);

    let other_path = dir.path().join("cover3.tif");
    write_cover_tiff(&other_path, 8, 6, Some(4), "RawOxide test").unwrap();
    let other = tiff_reader::TiffFile::open(&other_path)
        .unwrap()
        .read_image_bytes(0)
        .unwrap();
    assert_ne!(first, other);
}

#[test]
fn rejects_zero_geometry() {
    let dir = tempfile::tempdir().unwrap();
    assert!(write_cover_tiff(dir.path().join("bad.tif"), 0, 4, None, "x").is_err());
}
