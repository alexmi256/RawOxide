//! Sizing parity tests against the Python `AutoSizer` oracle.

use rawhide_core::profile::top_planes;
use rawhide_core::sizing::{RecommendOptions, SizingError, SizingMode, recommend};
use rawhide_core::{max_config, risk_warnings};

#[test]
fn preset_demo_geometry_matches_python() {
    let config = recommend(3000, &RecommendOptions::default()).unwrap();
    assert_eq!(config.width, 2048);
    assert_eq!(config.height, 1536);
    assert_eq!(config.lsb_planes, 1);
    assert_eq!(config.mode, SizingMode::Linear);
    assert_eq!(config.bit_depth, 16);
    assert_eq!(config.frames, 1);
    assert_eq!(config.capacity, 1_179_630);
    assert_eq!(config.preset, Some("demo"));
    assert_eq!(
        config.auto,
        ["mode", "bit_depth", "width", "height", "lsb_planes"]
    );
}

#[test]
fn cfa_capacity_is_third_of_linear() {
    let config = recommend(
        3000,
        &RecommendOptions {
            mode: Some(SizingMode::Cfa),
            ..RecommendOptions::default()
        },
    )
    .unwrap();
    assert_eq!(config.capacity, 393_198);
    assert_eq!(config.auto, ["bit_depth", "width", "height", "lsb_planes"]);
}

#[test]
fn auto_size_computes_minimal_frame() {
    let config = recommend(
        1_000_000,
        &RecommendOptions {
            auto_size: true,
            ..RecommendOptions::default()
        },
    )
    .unwrap();
    assert_eq!(config.width, 1886);
    assert_eq!(config.height, 1416);
    assert_eq!(config.lsb_planes, 1);
    assert_eq!(config.capacity, 1_001_448);
    assert_eq!(config.preset, Some("auto-size"));
}

#[test]
fn auto_size_scales_planes_before_geometry() {
    let config = recommend(
        2_000_000_000,
        &RecommendOptions {
            auto_size: true,
            ..RecommendOptions::default()
        },
    )
    .unwrap();
    assert_eq!(config.width, 22_538);
    assert_eq!(config.height, 16_904);
    assert_eq!(config.lsb_planes, 14);
    assert_eq!(config.preset, Some("auto-size"));
}

#[test]
fn one_sided_geometry_completes_to_four_thirds() {
    let config = recommend(
        1_000_000,
        &RecommendOptions {
            width: Some(4000),
            ..RecommendOptions::default()
        },
    )
    .unwrap();
    assert_eq!(config.width, 4000);
    assert_eq!(config.height, 3000);
    assert_eq!(config.auto, ["mode", "bit_depth", "height", "lsb_planes"]);
}

#[test]
fn explicit_geometry_keeps_requested_planes() {
    let config = recommend(
        3000,
        &RecommendOptions {
            width: Some(128),
            height: Some(96),
            ..RecommendOptions::default()
        },
    )
    .unwrap();
    assert_eq!(config.lsb_planes, 1);
    assert_eq!(config.capacity, 4590);
}

#[test]
fn explicit_geometry_that_cannot_fit_mentions_kb_and_remedies() {
    let error = recommend(
        1_000_000,
        &RecommendOptions {
            width: Some(128),
            height: Some(96),
            ..RecommendOptions::default()
        },
    )
    .unwrap_err();
    let text = error.to_string();
    assert!(
        text.contains("payload 976.6 KB exceeds capacity 71.9 KB"),
        "{text}"
    );
    assert!(text.contains("largest supported configuration"), "{text}");
    assert!(
        text.contains("split the input across multiple DNGs"),
        "{text}"
    );
}

#[test]
fn planes_above_depth_fail_closed() {
    let error = recommend(
        10,
        &RecommendOptions {
            bit_depth: Some(12),
            lsb_planes: Some(13),
            ..RecommendOptions::default()
        },
    )
    .unwrap_err();
    assert!(matches!(error, SizingError::PlanesExceedDepth { .. }));
}

#[test]
fn profile_plane_ceiling_is_enforced() {
    let error = recommend(
        10,
        &RecommendOptions {
            max_planes: Some(4),
            lsb_planes: Some(5),
            ..RecommendOptions::default()
        },
    )
    .unwrap_err();
    assert!(text_contains_profile(&error), "{error}");
}

const fn text_contains_profile(error: &SizingError) -> bool {
    matches!(error, SizingError::PlanesExceedProfile { top: 4, .. })
}

#[test]
fn auto_size_conflicts_with_explicit_dimensions() {
    let error = recommend(
        10,
        &RecommendOptions {
            width: Some(100),
            height: Some(100),
            auto_size: true,
            ..RecommendOptions::default()
        },
    )
    .unwrap_err();
    assert!(matches!(error, SizingError::AutoSizeConflict));
}

#[test]
fn risk_warnings_cover_planes_frames_depth_and_key() {
    let config = recommend(
        3000,
        &RecommendOptions {
            bit_depth: Some(12),
            lsb_planes: Some(10),
            frames: 2,
            ..RecommendOptions::default()
        },
    )
    .unwrap();
    let warnings = risk_warnings(&config, false, None);
    assert_eq!(warnings.len(), 4);
    assert!(warnings[0].contains("destroy most of the 12-bit cover"));
    assert!(warnings[1].contains("2 full-resolution raw frames"));
    assert!(warnings[2].contains("12-bit is below every"));
    assert!(warnings[3].contains("without --key"));
}

#[test]
fn sixteen_planes_reports_no_cover() {
    let config = recommend(
        3000,
        &RecommendOptions {
            lsb_planes: Some(16),
            ..RecommendOptions::default()
        },
    )
    .unwrap();
    let warnings = risk_warnings(&config, true, Some("Sony A7R V"));
    assert!(warnings[0].contains("leave no cover"));
    assert_eq!(warnings.len(), 1);
}

#[test]
fn max_config_description_matches_python() {
    assert_eq!(
        max_config(SizingMode::Linear, 16, 1),
        "23296x17472 linear 16-bit, 16 LSB planes"
    );
    assert_eq!(
        max_config(SizingMode::Cfa, 12, 3),
        "23296x17472 cfa 12-bit, 12 LSB planes x 3 frames"
    );
    assert_eq!(top_planes(8), 8);
    assert_eq!(top_planes(16), 16);
}
