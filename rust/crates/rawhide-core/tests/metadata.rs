//! Metadata randomizer behaviour and reproducibility.

use chrono::NaiveDate;
use rawhide_core::metadata::MetadataRandomizer;
use rawhide_core::profiles;

#[test]
fn same_seed_produces_identical_metadata() {
    let profile = profiles::get_profile("canon_eos_80d").unwrap();
    let mut first = MetadataRandomizer::new(Some(42), Some(profile));
    let mut second = MetadataRandomizer::new(Some(42), Some(profile));
    assert_eq!(first.randomize(None), second.randomize(None));
}

#[test]
fn different_seeds_change_serials_and_exposure() {
    let mut first = MetadataRandomizer::new(Some(1), None);
    let mut second = MetadataRandomizer::new(Some(2), None);
    assert_ne!(
        first.randomize(None).camera_serial,
        second.randomize(None).camera_serial
    );
}

#[test]
fn harvested_profile_pools_are_respected() {
    let profile = profiles::get_profile("canon_eos_80d").unwrap();
    let mut randomizer = MetadataRandomizer::new(Some(7), Some(profile));
    for _ in 0..20 {
        let metadata = randomizer.randomize(None);
        assert_eq!(metadata.lens_make, "Canon");
        assert!(profile.iso_choices.contains(&metadata.iso));
        assert!(profile.exposure_choices.contains(&metadata.exposure_time));
        assert!(profile.fnumber_choices.contains(&metadata.fnumber));
        assert!(profile.metering_choices.contains(&metadata.metering));
        assert!(
            profile
                .exposure_program_choices
                .contains(&metadata.exposure_program)
        );
        assert!(metadata.camera_serial.starts_with("02"));
        assert_eq!(metadata.camera_serial.len(), 8);
        assert_eq!(metadata.focal_35mm, (metadata.focal * 1.6).round() as u32);
    }
}

#[test]
fn reference_profile_keeps_legacy_serial_shapes() {
    let profile = profiles::get_profile("gfx_100").unwrap();
    let mut randomizer = MetadataRandomizer::new(Some(3), Some(profile));
    let mut bodies = std::collections::HashSet::new();
    for _ in 0..40 {
        let metadata = randomizer.randomize(None);
        assert!(
            metadata.body_serial == "33000087"
                || metadata.body_serial == "94000525"
                || metadata.body_serial.len() == 8
        );
        bodies.insert(metadata.body_serial);
    }
    assert!(bodies.len() > 2);
}

#[test]
fn fidelity_ranges_are_honoured() {
    let profile = profiles::get_profile("custom_pixel_xl").unwrap();
    let mut randomizer = MetadataRandomizer::new(Some(11), Some(profile));
    for _ in 0..20 {
        let metadata = randomizer.randomize(None);
        assert!((4453..=6460).contains(&metadata.as_shot_neutral[0].0));
        assert!((3780..=6410).contains(&metadata.as_shot_neutral[2].0));
        assert_eq!(metadata.as_shot_neutral[1], (10_000, 10_000));
        assert!((1300..=9400).contains(&metadata.baseline_exposure.0));
        assert_eq!(metadata.exposure_bias, (0, 100));
        assert_eq!(metadata.flash, 16);
        assert_eq!(metadata.subsec.len(), 6);
        assert!([6, 3].contains(&metadata.orientation));
        assert!(
            profile
                .fidelity
                .as_ref()
                .unwrap()
                .software_variants
                .contains(&metadata.software)
        );
        assert!(metadata.noise_profile.is_some());
    }
}

#[test]
fn pinned_image_time_sets_datetime_and_offset() {
    let mut randomizer = MetadataRandomizer::new(Some(5), None);
    let time = NaiveDate::from_ymd_opt(2021, 6, 5)
        .unwrap()
        .and_hms_opt(14, 30, 9)
        .unwrap();
    let image_time = rawhide_core::ImageTime {
        datetime: time,
        offset: "+09:00".to_owned(),
    };
    let metadata = randomizer.randomize(Some(&image_time));
    assert_eq!(metadata.datetime, "2021:06:05 14:30:09");
    assert_eq!(metadata.offset, "+09:00");
}

#[test]
fn date_range_profile_keeps_timestamps_inside_the_range() {
    let profile = profiles::get_profile("custom_pixel_xl").unwrap();
    // The custom profile uses `date_window_days` ending now; timestamps must be
    // between the two bounds observed at construction time.
    let bounds = profile.fidelity.as_ref().unwrap().date_bounds().unwrap();
    let mut randomizer = MetadataRandomizer::new(Some(9), Some(profile));
    for _ in 0..10 {
        let metadata = randomizer.randomize(None);
        let stamp =
            chrono::NaiveDateTime::parse_from_str(&metadata.datetime, "%Y:%m:%d %H:%M:%S").unwrap();
        assert!(
            stamp >= bounds.0 && stamp <= bounds.1,
            "{stamp} not in {bounds:?}"
        );
    }
}

#[test]
fn default_exposure_bias_pool_is_not_always_zero() {
    let mut randomizer = MetadataRandomizer::new(Some(6), None);
    let mut seen = std::collections::HashSet::new();
    for _ in 0..40 {
        let metadata = randomizer.randomize(None);
        assert!([0, -33, 33, -67, 67].contains(&metadata.exposure_bias.0));
        assert_eq!(metadata.exposure_bias.1, 100);
        seen.insert(metadata.exposure_bias.0);
    }
    assert!(seen.len() > 1);
    assert!(seen.iter().any(|bias| *bias != 0));
}
