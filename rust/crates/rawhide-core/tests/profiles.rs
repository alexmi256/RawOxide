//! Profile registry parity tests against the embedded Python registry.

use rawhide_core::profile::CameraProfile;
use rawhide_core::profiles;

#[test]
fn registry_embeds_every_profile_plus_reference() {
    assert_eq!(profiles::count(), 308);
    let slugs = profiles::list_profiles();
    assert!(slugs.contains(&"gfx_100"));
    assert!(slugs.contains(&"canon_eos_80d"));
    assert!(slugs.contains(&"custom_pixel_xl"));
}

#[test]
fn harvested_profile_fields_match_python() {
    let profile = profiles::get_profile("canon_eos_80d").unwrap();
    assert_eq!(profile.make, "Canon");
    assert_eq!(profile.model, "Canon EOS 80D");
    assert_eq!(profile.slug, "canon_eos_80d");
    assert!((profile.crop_factor - 1.6).abs() < 1e-9);
    assert_eq!(profile.suggested_bit_depth, 14);
    assert_eq!(profile.native_width, Some(6024));
    assert_eq!(profile.native_height, Some(4020));
    assert_eq!(profile.serial_prefixes, ["02"]);
    assert_eq!(
        profile.subprofiles.get("native"),
        Some(&(6024_usize, 4020_usize))
    );
    assert_eq!(
        profile.lenses[0],
        rawhide_core::profile::Lens {
            model: "EF100-400mm f/4.5-5.6L IS II USM".to_owned(),
            min_mm: 100.0,
            max_mm: 400.0,
            max_aperture: 4.5,
            make: None,
        }
    );
    assert_eq!(profile.exposure_choices[0], (1, 1000));
    assert_eq!(profile.fnumber_choices[1], (200, 100));
    assert_eq!(profile.iso_choices[..3], [50, 64, 100]);
    assert_eq!(profile.metering_choices, [2, 3, 5]);
}

#[test]
fn depth_helpers_match_python() {
    let profile = profiles::get_profile("canon_eos_80d").unwrap();
    assert_eq!(profile.black_level(16), 256);
    assert_eq!(profile.black_level(14), 64);
    assert_eq!(CameraProfile::white_level(12), 4095);
    assert_eq!(CameraProfile::top_planes(12), 12);
    assert_eq!(CameraProfile::container_bits(8), 8);
    assert_eq!(CameraProfile::container_bits(14), 16);
}

#[test]
fn fidelity_overlay_is_parsed() {
    let profile = profiles::get_profile("custom_pixel_xl").unwrap();
    let fidelity = profile.fidelity.as_ref().unwrap();
    assert!(fidelity.emit_gps);
    assert!(!fidelity.emit_xmp);
    assert!(!fidelity.emit_lens);
    assert!(!fidelity.emit_serials);
    assert_eq!(fidelity.orientations, [6, 3]);
    assert_eq!(fidelity.subsec_digits, 6);
    assert_eq!(fidelity.date_window_days, Some(365));
    assert_eq!(fidelity.suggested_mode.as_deref(), Some("cfa"));
    assert_eq!(fidelity.tag_conventions.as_deref(), Some("adobe"));
    assert!(fidelity.raw_digests);
    assert_eq!(fidelity.asn_r, Some((4453, 6460)));
    assert_eq!(fidelity.asn_b, Some((3780, 6410)));
    assert_eq!(fidelity.baseline_exposure, Some((1300, 9400)));
    assert_eq!(fidelity.subject_distance[0], (4_294_967_295, 1));
    assert_eq!(fidelity.exposure_bias, [0]);
    assert_eq!(fidelity.flash, Some(16));
    assert_eq!(fidelity.white_level, Some(16_368));
    assert_eq!(fidelity.max_planes, Some(16));
    assert_eq!(fidelity.level_policy, "capacity");
    assert_eq!(fidelity.tile, Some((256, 256)));
    assert_eq!(fidelity.black_levels.as_ref().unwrap().len(), 4);
    assert_eq!(profile.effective_max_planes(16, "profile"), 16);
    assert_eq!(profile.effective_white(16, "profile"), 65_535);
}

#[test]
fn resolve_supports_longest_slug_and_subprofiles() {
    let (profile, subprofile) = profiles::resolve("gfx_100-pixelshift").unwrap();
    assert_eq!(profile.slug, "gfx_100");
    assert_eq!(subprofile, Some("pixelshift"));
    assert!(profiles::resolve("gfx_100-nope").is_err());
    assert!(profiles::resolve("not_a_camera").is_err());
}

#[test]
fn reference_profile_stays_default() {
    let profile = profiles::get_profile("gfx_100").unwrap();
    assert_eq!(*profile, CameraProfile::default());
}

#[test]
fn strict_policy_caps_planes_by_white_headroom() {
    let mut profile = profiles::get_profile("canon_eos_80d").unwrap().clone();
    profile.fidelity = Some(rawhide_core::FidelityConfig {
        white_level: Some(16_368),
        max_planes: Some(16),
        level_policy: "strict".to_owned(),
        ..rawhide_core::FidelityConfig::default()
    });
    assert_eq!(profile.effective_white(16, "strict"), 16_368);
    assert_eq!(profile.effective_max_planes(16, "strict"), 13);
    assert_eq!(profile.effective_max_planes(16, "capacity"), 16);
}

#[test]
fn makernote_envelope_is_parsed() {
    let profile = profiles::get_profile("custom_pixel_xl").unwrap();
    let makernote = profile
        .fidelity
        .as_ref()
        .unwrap()
        .makernote
        .as_ref()
        .expect("makernote config");
    assert_eq!(makernote.magic, b"HDRP");
    assert_eq!(makernote.length, (2640, 2757));
    assert_eq!(
        makernote.prefix,
        vec![
            0x02, 0xef, 0x64, 0x35, 0x6d, 0x5e, 0x70, 0x1e, 0x2c, 0xea, 0xe3,
        ]
    );
}

#[test]
fn makernote_defaults_and_whitespace_match_python() {
    let value = serde_json::json!({"prefix_hex": "02 ef 64"});
    let config = rawhide_core::profile::MakernoteConfig::from_json(&value).unwrap();
    assert_eq!(config.magic, b"HDRP");
    assert_eq!(config.prefix, vec![0x02, 0xef, 0x64]);
    assert_eq!(config.length, (2640, 2757));
}

#[test]
fn gainmap_grid_validation_matches_python() {
    let plane = serde_json::json!([[1.0, 1.0], [1.0, 1.0]]);
    let valid = serde_json::json!({
        "grid": [2, 2],
        "planes": [plane.clone(), plane.clone(), plane.clone(), plane],
    });
    assert!(rawhide_core::profile::GainMapConfig::from_json(&valid).is_ok());

    let mismatch = serde_json::json!({
        "grid": [3, 2],
        "planes": [plane.clone(), plane.clone(), plane.clone(), plane],
    });
    assert!(rawhide_core::profile::GainMapConfig::from_json(&mismatch).is_err());

    let bad_clip = serde_json::json!({
        "grid": [2, 2],
        "clip": [4.0, 1.0],
        "planes": [plane.clone(), plane.clone(), plane.clone(), plane],
    });
    assert!(rawhide_core::profile::GainMapConfig::from_json(&bad_clip).is_err());

    let bad_jitter = serde_json::json!({
        "grid": [2, 2],
        "jitter": -1.0,
        "planes": [plane.clone(), plane.clone(), plane.clone(), plane],
    });
    assert!(rawhide_core::profile::GainMapConfig::from_json(&bad_jitter).is_err());
}

#[test]
fn adobe_profile_block_is_parsed() {
    let profile = profiles::get_profile("custom_pixel_xl").unwrap();
    let adobe = profile
        .fidelity
        .as_ref()
        .unwrap()
        .adobe
        .as_ref()
        .expect("adobe profile");
    assert!(adobe.forward_matrix1.is_some());
    assert!(adobe.name.as_deref().is_some_and(|name| !name.is_empty()));
    let dims = adobe.hue_sat_map_dims.expect("hue dims");
    assert_eq!(
        adobe.hue_sat_map_data1.as_ref().unwrap().len(),
        dims[0] as usize * dims[1] as usize * dims[2] as usize * 3
    );
    let look = adobe.look_table_dims.expect("look dims");
    assert_eq!(
        adobe.look_table_data.as_ref().unwrap().len(),
        look[0] as usize * look[1] as usize * look[2] as usize * 3
    );
}

#[test]
fn adobe_profile_values_match_assets() {
    let profile = profiles::get_profile("custom_pixel_xl").unwrap();
    let adobe = profile.fidelity.as_ref().unwrap().adobe.as_ref().unwrap();
    assert_eq!(adobe.name.as_deref(), Some("Adobe Standard"));
    assert_eq!(adobe.forward_matrix1.unwrap()[0], (4471, 10_000));
    assert_eq!(
        &adobe.hue_sat_map_data1.as_ref().unwrap()[..4],
        &[2.0, 1.0, 1.0, 2.0]
    );
    assert_eq!(
        &adobe.look_table_data.as_ref().unwrap()[..4],
        &[0.0, 1.0, 1.0, 0.0]
    );
}
