//! DNG/TIFF tag entries for IFD0, preview, raw, EXIF and GPS IFDs.

use super::{
    CameraProfile, Compression, Entry, IfdBuilder, LevelPolicy, Metadata, RawMode, SplitIdField,
    SplitSeqField, TagType, WriteError,
};

pub(super) const TAG_NEW_SUBFILE_TYPE: u16 = 254;
pub(super) const TAG_IMAGE_WIDTH: u16 = 256;
pub(super) const TAG_IMAGE_LENGTH: u16 = 257;
pub(super) const TAG_BITS_PER_SAMPLE: u16 = 258;
pub(super) const TAG_COMPRESSION: u16 = 259;
pub(super) const TAG_PHOTOMETRIC: u16 = 262;
pub(super) const TAG_IMAGE_DESCRIPTION: u16 = 270;
pub(super) const TAG_MAKERNOTE: u16 = 37500;
pub(super) const TAG_MAKERNOTE_SAFETY: u16 = 50741;
pub(super) const TAG_XMP: u16 = 700;
pub(super) const TAG_MAKE: u16 = 271;
pub(super) const TAG_MODEL: u16 = 272;
pub(super) const TAG_STRIP_OFFSETS: u16 = 273;
pub(super) const TAG_ORIENTATION: u16 = 274;
pub(super) const TAG_SAMPLES_PER_PIXEL: u16 = 277;
pub(super) const TAG_ROWS_PER_STRIP: u16 = 278;
pub(super) const TAG_STRIP_BYTE_COUNTS: u16 = 279;
pub(super) const TAG_X_RESOLUTION: u16 = 282;
pub(super) const TAG_Y_RESOLUTION: u16 = 283;
pub(super) const TAG_PLANAR_CONFIGURATION: u16 = 284;
pub(super) const TAG_PREDICTOR: u16 = 317;
pub(super) const TAG_RESOLUTION_UNIT: u16 = 296;
pub(super) const TAG_PAGE_NUMBER: u16 = 297;
pub(super) const TAG_SOFTWARE: u16 = 305;
pub(super) const TAG_DATE_TIME: u16 = 306;
pub(super) const TAG_TILE_WIDTH: u16 = 322;
pub(super) const TAG_TILE_LENGTH: u16 = 323;
pub(super) const TAG_TILE_OFFSETS: u16 = 324;
pub(super) const TAG_TILE_BYTE_COUNTS: u16 = 325;
pub(super) const TAG_SUB_IFDS: u16 = 330;
pub(super) const TAG_EXIF_IFD: u16 = 34665;
pub(super) const TAG_GPS_IFD: u16 = 34853;
pub(super) const TAG_GPS_VERSION: u16 = 0;
pub(super) const TAG_GPS_LAT_REF: u16 = 1;
pub(super) const TAG_GPS_LAT: u16 = 2;
pub(super) const TAG_GPS_LON_REF: u16 = 3;
pub(super) const TAG_GPS_LON: u16 = 4;
pub(super) const TAG_GPS_ALT_REF: u16 = 5;
pub(super) const TAG_GPS_ALT: u16 = 6;
pub(super) const TAG_GPS_TIME: u16 = 7;
pub(super) const TAG_GPS_DOP: u16 = 11;
pub(super) const TAG_GPS_PROCESSING: u16 = 27;
pub(super) const TAG_GPS_DATE: u16 = 29;
pub(super) const TAG_EXPOSURE_TIME: u16 = 33434;
pub(super) const TAG_FNUMBER: u16 = 33437;
pub(super) const TAG_EXPOSURE_PROGRAM: u16 = 34850;
pub(super) const TAG_ISO: u16 = 34855;
pub(super) const TAG_SENSITIVITY_TYPE: u16 = 34864;
pub(super) const TAG_STANDARD_OUTPUT_SENSITIVITY: u16 = 34865;
pub(super) const TAG_EXIF_VERSION: u16 = 36864;
pub(super) const TAG_DATETIME_ORIGINAL: u16 = 36867;
pub(super) const TAG_DATETIME_DIGITIZED: u16 = 36868;
pub(super) const TAG_OFFSET_TIME: u16 = 36880;
pub(super) const TAG_OFFSET_TIME_ORIGINAL: u16 = 36881;
pub(super) const TAG_OFFSET_TIME_DIGITIZED: u16 = 36882;
pub(super) const TAG_SHUTTER_SPEED: u16 = 37377;
pub(super) const TAG_APERTURE: u16 = 37378;
pub(super) const TAG_BRIGHTNESS: u16 = 37379;
pub(super) const TAG_EXPOSURE_BIAS: u16 = 37380;
pub(super) const TAG_MAX_APERTURE: u16 = 37381;
pub(super) const TAG_SUBJECT_DISTANCE: u16 = 37382;
pub(super) const TAG_METERING: u16 = 37383;
pub(super) const TAG_LIGHT_SOURCE: u16 = 37384;
pub(super) const TAG_FLASH: u16 = 37385;
pub(super) const TAG_FOCAL_LENGTH: u16 = 37386;
pub(super) const TAG_SUBSEC_TIME: u16 = 37520;
pub(super) const TAG_SUBSEC_TIME_ORIGINAL: u16 = 37521;
pub(super) const TAG_SUBSEC_TIME_DIGITIZED: u16 = 37522;
pub(super) const TAG_COLOR_SPACE: u16 = 40961;
pub(super) const TAG_PIXEL_X_DIMENSION: u16 = 40962;
pub(super) const TAG_PIXEL_Y_DIMENSION: u16 = 40963;
pub(super) const TAG_FOCAL_PLANE_X_RESOLUTION: u16 = 41486;
pub(super) const TAG_FOCAL_PLANE_Y_RESOLUTION: u16 = 41487;
pub(super) const TAG_FOCAL_PLANE_RESOLUTION_UNIT: u16 = 41488;
pub(super) const TAG_SENSING_METHOD: u16 = 41495;
pub(super) const TAG_FILE_SOURCE: u16 = 41728;
pub(super) const TAG_SCENE_TYPE: u16 = 41729;
pub(super) const TAG_CFA_PATTERN_EXIF: u16 = 41730;
pub(super) const TAG_CUSTOM_RENDERED: u16 = 41985;
pub(super) const TAG_EXPOSURE_MODE: u16 = 41986;
pub(super) const TAG_WHITE_BALANCE: u16 = 41987;
pub(super) const TAG_DIGITAL_ZOOM_RATIO: u16 = 41988;
pub(super) const TAG_FOCAL_35MM: u16 = 41989;
pub(super) const TAG_SCENE_CAPTURE_TYPE: u16 = 41990;
pub(super) const TAG_SUBJECT_DISTANCE_RANGE: u16 = 41996;
pub(super) const TAG_BODY_SERIAL: u16 = 42033;
pub(super) const TAG_LENS_SPEC: u16 = 42034;
pub(super) const TAG_LENS_MAKE: u16 = 42035;
pub(super) const TAG_LENS_MODEL: u16 = 42036;
pub(super) const TAG_LENS_SERIAL: u16 = 42037;
pub(super) const TAG_AS_SHOT_NEUTRAL: u16 = 50728;
pub(super) const TAG_ANALOG_BALANCE: u16 = 50727;
pub(super) const TAG_LENS_INFO: u16 = 50736;
pub(super) const TAG_BASELINE_EXPOSURE: u16 = 50730;
pub(super) const TAG_NOISE_PROFILE: u16 = 51041;
pub(super) const TAG_IMAGE_UNIQUE_ID: u16 = 42016;
pub(super) const TAG_IMAGE_NUMBER: u16 = 37393;
pub(super) const TAG_CFA_REPEAT_PATTERN_DIM: u16 = 33_421;
pub(super) const TAG_CFA_PATTERN: u16 = 33_422;
pub(super) const TAG_REFERENCE_BLACK_WHITE: u16 = 532;
pub(super) const TAG_YCBCR_SUBSAMPLING: u16 = 530;
pub(super) const TAG_DNG_VERSION: u16 = 50706;
pub(super) const TAG_ADOBE_CALIBRATION_SIGNATURE: u16 = 50932;
pub(super) const TAG_ADOBE_NAME: u16 = 50936;
pub(super) const TAG_ADOBE_HUE_SAT_DIMS: u16 = 50937;
pub(super) const TAG_ADOBE_HUE_SAT_DATA1: u16 = 50938;
pub(super) const TAG_ADOBE_HUE_SAT_DATA2: u16 = 50939;
pub(super) const TAG_ADOBE_EMBED_POLICY: u16 = 50941;
pub(super) const TAG_ADOBE_COPYRIGHT: u16 = 50942;
pub(super) const TAG_ADOBE_FORWARD1: u16 = 50964;
pub(super) const TAG_ADOBE_FORWARD2: u16 = 50965;
pub(super) const TAG_ADOBE_LOOK_DIMS: u16 = 50981;
pub(super) const TAG_ADOBE_LOOK_DATA: u16 = 50982;
pub(super) const TAG_YCBCR_COEFFICIENTS: u16 = 529;
pub(super) const TAG_YCBCR_POSITIONING: u16 = 531;
pub(super) const TAG_DNG_BACKWARD_VERSION: u16 = 50707;
pub(super) const TAG_UNIQUE_CAMERA_MODEL: u16 = 50708;
pub(super) const TAG_CFA_PLANE_COLOR: u16 = 50710;
pub(super) const TAG_CFA_LAYOUT: u16 = 50711;
pub(super) const TAG_BLACK_LEVEL_REPEAT_DIM: u16 = 50713;
pub(super) const TAG_BLACK_LEVEL: u16 = 50714;
pub(super) const TAG_WHITE_LEVEL: u16 = 50717;
pub(super) const TAG_DEFAULT_SCALE: u16 = 50718;
pub(super) const TAG_DEFAULT_CROP_ORIGIN: u16 = 50719;
pub(super) const TAG_DEFAULT_CROP_SIZE: u16 = 50720;
pub(super) const TAG_ACTIVE_AREA: u16 = 50829;
pub(super) const TAG_DEFAULT_USER_CROP: u16 = 51125;
pub(super) const TAG_COLOR_MATRIX1: u16 = 50721;
pub(super) const TAG_COLOR_MATRIX2: u16 = 50722;
pub(super) const TAG_BASELINE_NOISE: u16 = 50731;
pub(super) const TAG_BASELINE_SHARPNESS: u16 = 50732;
pub(super) const TAG_BAYER_GREEN_SPLIT: u16 = 50733;
pub(super) const TAG_LINEAR_RESPONSE_LIMIT: u16 = 50734;
pub(super) const TAG_CAMERA_SERIAL: u16 = 50735;
pub(super) const TAG_ANTI_ALIAS_STRENGTH: u16 = 50738;
pub(super) const TAG_SHADOW_SCALE: u16 = 50739;
pub(super) const TAG_CALIBRATION_ILLUMINANT1: u16 = 50778;
pub(super) const TAG_CALIBRATION_ILLUMINANT2: u16 = 50779;
pub(super) const TAG_BEST_QUALITY_SCALE: u16 = 50780;
pub(super) const TAG_RAW_DATA_UNIQUE_ID: u16 = 50781;
pub(super) const TAG_NEW_RAW_IMAGE_DIGEST: u16 = 51111;
pub(super) const TAG_OPCODE_LIST2: u16 = 51009;
pub(super) const TAG_OPCODE_LIST3: u16 = 51022;

/// Static XMP packet (12 KiB, space-padded).
pub(super) fn xmp_packet() -> Vec<u8> {
    const CORE: &[u8] = b"<?xpacket begin=\"\xef\xbb\xbf\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n\
<x:xmpmeta xmlns:x=\"adobe:ns:meta/\" x:xmptk=\"Adobe XMP Core 5.6-c140 79.160451, 2017/05/06-01:08:21\">\n\
<rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n\
<rdf:Description rdf:about=\"\" xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\">\n\
<xmp:Rating>0</xmp:Rating>\n\
</rdf:Description>\n</rdf:RDF>\n</x:xmpmeta>\n\
<?xpacket end=\"w\"?>\n";
    let mut packet = CORE.to_vec();
    packet.resize(12_288, b' ');
    packet
}

pub(super) fn split_entries(
    split_id: Option<&str>,
    split_seq: Option<u32>,
    split_total: Option<u32>,
    id_field: SplitIdField,
    seq_field: SplitSeqField,
) -> Result<(Vec<Entry>, Vec<Entry>), WriteError> {
    let mut ifd0 = Vec::new();
    let mut exif = Vec::new();

    if let Some(id) = split_id {
        match id_field {
            SplitIdField::ImageUniqueId => exif.push(Entry::ascii(TAG_IMAGE_UNIQUE_ID, id)),
            SplitIdField::ImageDescription => {
                ifd0.push(Entry::ascii(TAG_IMAGE_DESCRIPTION, id));
            }
            SplitIdField::None => {}
        }
    }

    if let Some(seq) = split_seq {
        match seq_field {
            SplitSeqField::ImageNumber => {
                exif.push(Entry::longs(TAG_IMAGE_NUMBER, &[seq]));
            }
            SplitSeqField::PageNumber => {
                let total = split_total.ok_or_else(|| {
                    WriteError::InvalidSplit("PageNumber needs a total".to_owned())
                })?;
                ifd0.push(Entry::shorts(
                    TAG_PAGE_NUMBER,
                    &[
                        u16::try_from(seq).unwrap_or(u16::MAX),
                        u16::try_from(total).unwrap_or(u16::MAX),
                    ],
                ));
            }
            SplitSeqField::ImageDescription => {
                let total = split_total.ok_or_else(|| {
                    WriteError::InvalidSplit("ImageDescription sequence needs a total".to_owned())
                })?;
                ifd0.push(Entry::ascii(
                    TAG_IMAGE_DESCRIPTION,
                    &format!("{seq:04}/{total:04}"),
                ));
            }
            SplitSeqField::None => {}
        }
    }

    Ok((ifd0, exif))
}

pub(super) const fn select_bigtiff(raw_bytes: u64, preview: u64, force: bool) -> bool {
    force || raw_bytes.saturating_add(preview) > 3_500_000_000
}

pub(super) fn build_ifd(
    builder: &IfdBuilder,
    offset: u64,
    big: bool,
) -> Result<Vec<u8>, WriteError> {
    if big {
        Ok(builder.build_big(offset)?)
    } else {
        Ok(builder.build(u32::try_from(offset).map_err(|_| WriteError::TooLarge)?)?)
    }
}

pub(super) fn metadata_entries(
    profile: Option<&CameraProfile>,
    metadata: Option<&Metadata>,
    mode: RawMode,
    preview_width: usize,
    preview_height: usize,
) -> (Vec<Entry>, Vec<Entry>) {
    let Some(metadata) = metadata else {
        return (Vec::new(), Vec::new());
    };
    let fidelity = profile.and_then(|profile| profile.fidelity.as_ref());
    let allow = fidelity.and_then(|fidelity| fidelity.exif_allow.as_ref());
    let base_enabled = |tag: u16| allow.is_none_or(|list| list.contains(&u32::from(tag)));
    let extra_enabled = |tag: u16| allow.is_some_and(|list| list.contains(&u32::from(tag)));
    let emit_lens = fidelity.is_none_or(|fidelity| fidelity.emit_lens);
    let emit_serials = fidelity.is_none_or(|fidelity| fidelity.emit_serials);

    let mut ifd0 = Vec::new();
    if emit_serials {
        ifd0.push(Entry::ascii(TAG_CAMERA_SERIAL, &metadata.camera_serial));
    }
    let analog =
        if fidelity.and_then(|fidelity| fidelity.tag_conventions.as_deref()) == Some("adobe") {
            (1_000_000_u32, 1_000_000_u32)
        } else {
            (10_000_u32, 10_000_u32)
        };
    ifd0.push(Entry::rationals(
        TAG_ANALOG_BALANCE,
        &[analog, analog, analog],
    ));
    if emit_lens {
        ifd0.push(Entry::rationals(TAG_LENS_INFO, &metadata.lens_spec));
    }
    if let Some(noise) = metadata.noise_profile {
        ifd0.push(Entry::doubles(TAG_NOISE_PROFILE, &noise));
    }
    let mut exif = Vec::new();

    let fnumber = if metadata.fnumber.1 == 0 {
        0.0
    } else {
        f64::from(metadata.fnumber.0) / f64::from(metadata.fnumber.1)
    };
    let exposure_seconds = if metadata.exposure_time.1 == 0 {
        0.0
    } else {
        f64::from(metadata.exposure_time.0) / f64::from(metadata.exposure_time.1)
    };
    let shutter = if exposure_seconds > 0.0 {
        (-exposure_seconds.log2() * 100.0).round() as i32
    } else {
        0
    };
    let aperture = if fnumber > 0.0 {
        (2.0 * fnumber.log2() * 100.0).round() as u32
    } else {
        0
    };

    macro_rules! push_base {
        ($tag:expr, $entry:expr) => {
            if base_enabled($tag) {
                exif.push($entry);
            }
        };
    }

    push_base!(
        TAG_EXPOSURE_TIME,
        Entry::rationals(TAG_EXPOSURE_TIME, &[metadata.exposure_time])
    );
    push_base!(
        TAG_FNUMBER,
        Entry::rationals(TAG_FNUMBER, &[metadata.fnumber])
    );
    push_base!(
        TAG_EXPOSURE_PROGRAM,
        Entry::shorts(TAG_EXPOSURE_PROGRAM, &[metadata.exposure_program])
    );
    push_base!(
        TAG_ISO,
        Entry::shorts(TAG_ISO, &[u16::try_from(metadata.iso).unwrap_or(u16::MAX)])
    );
    push_base!(
        TAG_SENSITIVITY_TYPE,
        Entry::shorts(TAG_SENSITIVITY_TYPE, &[1])
    );
    push_base!(
        TAG_STANDARD_OUTPUT_SENSITIVITY,
        Entry::longs(TAG_STANDARD_OUTPUT_SENSITIVITY, &[metadata.iso])
    );
    push_base!(
        TAG_EXIF_VERSION,
        Entry::bytes(TAG_EXIF_VERSION, TagType::Undefined, b"0230".to_vec())
    );
    push_base!(
        TAG_DATETIME_ORIGINAL,
        Entry::ascii(TAG_DATETIME_ORIGINAL, &metadata.datetime)
    );
    push_base!(
        TAG_DATETIME_DIGITIZED,
        Entry::ascii(TAG_DATETIME_DIGITIZED, &metadata.datetime)
    );
    push_base!(
        TAG_OFFSET_TIME,
        Entry::ascii(TAG_OFFSET_TIME, &metadata.offset)
    );
    push_base!(
        TAG_OFFSET_TIME_ORIGINAL,
        Entry::ascii(TAG_OFFSET_TIME_ORIGINAL, &metadata.offset)
    );
    push_base!(
        TAG_OFFSET_TIME_DIGITIZED,
        Entry::ascii(TAG_OFFSET_TIME_DIGITIZED, &metadata.offset)
    );
    push_base!(
        TAG_SHUTTER_SPEED,
        Entry::srationals(TAG_SHUTTER_SPEED, &[(shutter, 100)])
    );
    push_base!(
        TAG_APERTURE,
        Entry::rationals(TAG_APERTURE, &[(aperture, 100)])
    );
    push_base!(
        TAG_BRIGHTNESS,
        Entry::srationals(
            TAG_BRIGHTNESS,
            &[(
                metadata.brightness.0 as i32,
                i32::try_from(metadata.brightness.1).unwrap_or(100),
            )]
        )
    );
    push_base!(
        TAG_EXPOSURE_BIAS,
        Entry::srationals(
            TAG_EXPOSURE_BIAS,
            &[(
                metadata.exposure_bias.0,
                i32::try_from(metadata.exposure_bias.1).unwrap_or(100),
            )]
        )
    );
    push_base!(
        TAG_MAX_APERTURE,
        Entry::rationals(TAG_MAX_APERTURE, &[metadata.max_aperture])
    );
    if extra_enabled(TAG_DIGITAL_ZOOM_RATIO) {
        exif.push(Entry::rationals(TAG_DIGITAL_ZOOM_RATIO, &[(0, 1)]));
    }
    if extra_enabled(TAG_CFA_PATTERN_EXIF) && mode == RawMode::Cfa {
        exif.push(Entry::bytes(
            TAG_CFA_PATTERN_EXIF,
            TagType::Undefined,
            vec![2, 0, 2, 0, 2, 1, 1, 0],
        ));
    }
    if extra_enabled(TAG_SUBJECT_DISTANCE) && metadata.subject_distance.1 != 0 {
        exif.push(Entry::rationals(
            TAG_SUBJECT_DISTANCE,
            &[metadata.subject_distance],
        ));
    }
    push_base!(
        TAG_METERING,
        Entry::shorts(TAG_METERING, &[metadata.metering])
    );
    push_base!(TAG_LIGHT_SOURCE, Entry::shorts(TAG_LIGHT_SOURCE, &[0]));
    push_base!(TAG_FLASH, Entry::shorts(TAG_FLASH, &[metadata.flash]));
    push_base!(
        TAG_FOCAL_LENGTH,
        Entry::rationals(TAG_FOCAL_LENGTH, &[metadata.focal_rational])
    );
    push_base!(
        TAG_SUBSEC_TIME,
        Entry::ascii(TAG_SUBSEC_TIME, &metadata.subsec)
    );
    push_base!(
        TAG_SUBSEC_TIME_ORIGINAL,
        Entry::ascii(TAG_SUBSEC_TIME_ORIGINAL, &metadata.subsec)
    );
    push_base!(
        TAG_SUBSEC_TIME_DIGITIZED,
        Entry::ascii(TAG_SUBSEC_TIME_DIGITIZED, &metadata.subsec)
    );
    push_base!(TAG_COLOR_SPACE, Entry::shorts(TAG_COLOR_SPACE, &[1]));
    push_base!(
        TAG_PIXEL_X_DIMENSION,
        Entry::longs(
            TAG_PIXEL_X_DIMENSION,
            &[u32::try_from(preview_width).unwrap_or(u32::MAX)]
        )
    );
    push_base!(
        TAG_PIXEL_Y_DIMENSION,
        Entry::longs(
            TAG_PIXEL_Y_DIMENSION,
            &[u32::try_from(preview_height).unwrap_or(u32::MAX)]
        )
    );
    push_base!(
        TAG_FOCAL_PLANE_X_RESOLUTION,
        Entry::rationals(TAG_FOCAL_PLANE_X_RESOLUTION, &[(913, 1)])
    );
    push_base!(
        TAG_FOCAL_PLANE_Y_RESOLUTION,
        Entry::rationals(TAG_FOCAL_PLANE_Y_RESOLUTION, &[(913, 1)])
    );
    push_base!(
        TAG_FOCAL_PLANE_RESOLUTION_UNIT,
        Entry::shorts(TAG_FOCAL_PLANE_RESOLUTION_UNIT, &[3])
    );
    push_base!(TAG_SENSING_METHOD, Entry::shorts(TAG_SENSING_METHOD, &[2]));
    push_base!(
        TAG_FILE_SOURCE,
        Entry::bytes(TAG_FILE_SOURCE, TagType::Undefined, vec![3])
    );
    push_base!(
        TAG_SCENE_TYPE,
        Entry::bytes(TAG_SCENE_TYPE, TagType::Undefined, vec![1])
    );
    push_base!(
        TAG_CUSTOM_RENDERED,
        Entry::shorts(TAG_CUSTOM_RENDERED, &[0])
    );
    push_base!(TAG_EXPOSURE_MODE, Entry::shorts(TAG_EXPOSURE_MODE, &[0]));
    push_base!(TAG_WHITE_BALANCE, Entry::shorts(TAG_WHITE_BALANCE, &[0]));
    push_base!(
        TAG_SCENE_CAPTURE_TYPE,
        Entry::shorts(TAG_SCENE_CAPTURE_TYPE, &[0])
    );
    push_base!(
        TAG_FOCAL_35MM,
        Entry::shorts(
            TAG_FOCAL_35MM,
            &[u16::try_from(metadata.focal_35mm).unwrap_or(u16::MAX)]
        )
    );
    if emit_serials {
        push_base!(
            TAG_BODY_SERIAL,
            Entry::ascii(TAG_BODY_SERIAL, &metadata.body_serial)
        );
    }
    if emit_lens {
        push_base!(
            TAG_LENS_SPEC,
            Entry::rationals(TAG_LENS_SPEC, &metadata.lens_spec)
        );
        push_base!(
            TAG_LENS_MAKE,
            Entry::ascii(TAG_LENS_MAKE, &metadata.lens_make)
        );
        push_base!(TAG_LENS_MODEL, Entry::ascii(TAG_LENS_MODEL, &metadata.lens));
        if emit_serials {
            push_base!(
                TAG_LENS_SERIAL,
                Entry::ascii(TAG_LENS_SERIAL, &metadata.lens_serial)
            );
        }
    }
    if extra_enabled(TAG_SUBJECT_DISTANCE_RANGE) && metadata.subject_distance_range != 0 {
        exif.push(Entry::shorts(
            TAG_SUBJECT_DISTANCE_RANGE,
            &[metadata.subject_distance_range],
        ));
    }
    if let Some(makernote) = &metadata.makernote {
        if extra_enabled(TAG_MAKERNOTE) {
            exif.push(Entry::bytes(
                TAG_MAKERNOTE,
                TagType::Undefined,
                makernote.clone(),
            ));
        }
        ifd0.push(Entry::shorts(TAG_MAKERNOTE_SAFETY, &[1]));
    }
    (ifd0, exif)
}

pub(super) fn gps_entries(metadata: Option<&Metadata>) -> Option<Vec<Entry>> {
    let gps = metadata?.gps.as_ref()?;
    Some(vec![
        Entry::bytes(TAG_GPS_VERSION, TagType::Byte, vec![2, 2, 0, 0]),
        Entry::ascii(TAG_GPS_LAT_REF, &gps.lat_ref.to_string()),
        Entry::rationals(TAG_GPS_LAT, &[(gps.lat.0, 1), (gps.lat.1, 1), gps.lat.2]),
        Entry::ascii(TAG_GPS_LON_REF, &gps.lon_ref.to_string()),
        Entry::rationals(TAG_GPS_LON, &[(gps.lon.0, 1), (gps.lon.1, 1), gps.lon.2]),
        Entry::bytes(
            TAG_GPS_ALT_REF,
            TagType::Byte,
            vec![u8::try_from(gps.altitude_ref).unwrap_or(0)],
        ),
        Entry::rationals(TAG_GPS_ALT, &[(gps.altitude_mm, 1000)]),
        Entry::rationals(
            TAG_GPS_TIME,
            &[(gps.time.0, 1), (gps.time.1, 1), (gps.time.2, 1)],
        ),
        Entry::rationals(TAG_GPS_DOP, &[(gps.dop, 1000)]),
        Entry::bytes(
            TAG_GPS_PROCESSING,
            TagType::Undefined,
            b"ASCII\0\0\0fused".to_vec(),
        ),
        Entry::ascii(TAG_GPS_DATE, &gps.date),
    ])
}

#[allow(clippy::too_many_arguments)]
pub(super) fn preview_entries(
    width: usize,
    height: usize,
    jpeg_len: usize,
    jpeg_offset: u64,
    sub_ifd_offsets: &[u64],
    exif_offset: Option<u64>,
    gps_offset: Option<u64>,
    profile: Option<&CameraProfile>,
    metadata: Option<&Metadata>,
    extra: &[Entry],
    big: bool,
) -> Vec<Entry> {
    let make = profile.map_or("RawOxide", |profile| profile.make.as_str());
    let model = profile.map_or("Rust Spike DNG", |profile| profile.model.as_str());
    let software = metadata
        .map(|metadata| metadata.software.as_str())
        .filter(|software| !software.is_empty())
        .or_else(|| {
            profile
                .map(|profile| profile.software.as_str())
                .filter(|software| !software.is_empty())
        });
    let adobe = profile.is_some_and(|profile| {
        profile
            .fidelity
            .as_ref()
            .and_then(|fidelity| fidelity.tag_conventions.as_deref())
            == Some("adobe")
    });
    let datetime = metadata.map_or("2026:11:08 12:00:00", |metadata| metadata.datetime.as_str());
    let orientation = metadata.map_or(1, |metadata| metadata.orientation);
    let mut entries = vec![
        Entry::longs(TAG_NEW_SUBFILE_TYPE, &[1]),
        Entry::longs(TAG_IMAGE_WIDTH, &[u32::try_from(width).unwrap_or(u32::MAX)]),
        Entry::longs(
            TAG_IMAGE_LENGTH,
            &[u32::try_from(height).unwrap_or(u32::MAX)],
        ),
        Entry::shorts(TAG_BITS_PER_SAMPLE, &[8, 8, 8]),
        Entry::shorts(TAG_COMPRESSION, &[7]),
        Entry::shorts(TAG_PHOTOMETRIC, &[6]),
        Entry::ascii(TAG_MAKE, make),
        Entry::ascii(TAG_MODEL, model),
        Entry::longs(
            TAG_STRIP_OFFSETS,
            &[u32::try_from(jpeg_offset).expect("classic offsets validated before IFD build")],
        ),
        Entry::shorts(TAG_ORIENTATION, &[orientation]),
        Entry::shorts(TAG_SAMPLES_PER_PIXEL, &[3]),
        Entry::longs(
            TAG_ROWS_PER_STRIP,
            &[u32::try_from(height).unwrap_or(u32::MAX)],
        ),
        Entry::longs(
            TAG_STRIP_BYTE_COUNTS,
            &[u32::try_from(jpeg_len).unwrap_or(u32::MAX)],
        ),
        Entry::shorts(TAG_PLANAR_CONFIGURATION, &[1]),
        Entry::ascii(TAG_DATE_TIME, datetime),
        if big {
            Entry::ifds8(TAG_SUB_IFDS, sub_ifd_offsets)
        } else {
            let offsets = sub_ifd_offsets
                .iter()
                .map(|offset| {
                    u32::try_from(*offset).expect("classic offsets validated before IFD build")
                })
                .collect::<Vec<_>>();
            if adobe {
                Entry::longs(TAG_SUB_IFDS, &offsets)
            } else {
                Entry::ifds(TAG_SUB_IFDS, &offsets)
            }
        },
        Entry::shorts(TAG_YCBCR_SUBSAMPLING, &[2, 2]),
        Entry::rationals(
            TAG_REFERENCE_BLACK_WHITE,
            &[(0, 1), (255, 1), (128, 1), (255, 1), (128, 1), (255, 1)],
        ),
    ];
    if !adobe {
        entries.push(Entry::rationals(TAG_X_RESOLUTION, &[(1, 1)]));
        entries.push(Entry::rationals(TAG_Y_RESOLUTION, &[(1, 1)]));
        entries.push(Entry::shorts(TAG_RESOLUTION_UNIT, &[1]));
    }
    if let Some(software) = software {
        entries.push(Entry::ascii(TAG_SOFTWARE, software));
    }
    if let Some(profile) = profile {
        entries.extend(profile_ifd0_entries(profile));
    }
    if let Some(metadata) = metadata {
        entries.push(Entry::rationals(
            TAG_AS_SHOT_NEUTRAL,
            &[
                (
                    u32::try_from(metadata.as_shot_neutral[0].0).unwrap_or(0),
                    metadata.as_shot_neutral[0].1,
                ),
                (
                    u32::try_from(metadata.as_shot_neutral[1].0).unwrap_or(0),
                    metadata.as_shot_neutral[1].1,
                ),
                (
                    u32::try_from(metadata.as_shot_neutral[2].0).unwrap_or(0),
                    metadata.as_shot_neutral[2].1,
                ),
            ],
        ));
        entries.push(Entry::srationals(
            TAG_BASELINE_EXPOSURE,
            &[(
                metadata.baseline_exposure.0,
                i32::try_from(metadata.baseline_exposure.1).unwrap_or(10_000),
            )],
        ));
    }
    let emit_xmp = profile
        .and_then(|profile| profile.fidelity.as_ref())
        .is_none_or(|fidelity| fidelity.emit_xmp);
    if emit_xmp {
        entries.push(Entry::bytes(TAG_XMP, TagType::Byte, xmp_packet()));
    }
    if let Some(offset) = exif_offset {
        entries.push(if big {
            Entry::long8(TAG_EXIF_IFD, offset)
        } else {
            Entry::longs(
                TAG_EXIF_IFD,
                &[u32::try_from(offset).expect("classic offsets validated before IFD build")],
            )
        });
    }
    if let Some(offset) = gps_offset {
        entries.push(if big {
            Entry::long8(TAG_GPS_IFD, offset)
        } else {
            Entry::longs(
                TAG_GPS_IFD,
                &[u32::try_from(offset).expect("classic offsets validated before IFD build")],
            )
        });
    }
    entries.extend(extra.iter().cloned());
    entries
}

pub(super) fn profile_ifd0_entries(profile: &CameraProfile) -> Vec<Entry> {
    let mut entries = Vec::new();
    if profile
        .fidelity
        .as_ref()
        .and_then(|fidelity| fidelity.tag_conventions.as_deref())
        == Some("adobe")
    {
        entries.push(Entry::rationals(
            TAG_YCBCR_COEFFICIENTS,
            &[(299, 1000), (587, 1000), (114, 1000)],
        ));
        entries.push(Entry::shorts(TAG_YCBCR_POSITIONING, &[1]));
    }
    if let Some(adobe) = profile
        .fidelity
        .as_ref()
        .and_then(|fidelity| fidelity.adobe.as_ref())
    {
        if let Some(matrix) = adobe.forward_matrix1 {
            entries.push(Entry::srationals(TAG_ADOBE_FORWARD1, &matrix));
        }
        if let Some(matrix) = adobe.forward_matrix2 {
            entries.push(Entry::srationals(TAG_ADOBE_FORWARD2, &matrix));
        }
        if let Some(name) = adobe.name.as_deref().filter(|name| !name.is_empty()) {
            entries.push(Entry::ascii(TAG_ADOBE_NAME, name));
        }
        if let Some(signature) = adobe
            .calibration_signature
            .as_deref()
            .filter(|signature| !signature.is_empty())
        {
            entries.push(Entry::ascii(TAG_ADOBE_CALIBRATION_SIGNATURE, signature));
        }
        if let Some(copyright) = adobe
            .copyright
            .as_deref()
            .filter(|copyright| !copyright.is_empty())
        {
            entries.push(Entry::ascii(TAG_ADOBE_COPYRIGHT, copyright));
        }
        if let Some(policy) = adobe.embed_policy {
            entries.push(Entry::longs(TAG_ADOBE_EMBED_POLICY, &[policy]));
        }
        if let Some(dims) = adobe.hue_sat_map_dims {
            entries.push(Entry::longs(TAG_ADOBE_HUE_SAT_DIMS, &dims));
            if let Some(data) = &adobe.hue_sat_map_data1 {
                entries.push(Entry::floats(TAG_ADOBE_HUE_SAT_DATA1, data));
            }
            if let Some(data) = &adobe.hue_sat_map_data2 {
                entries.push(Entry::floats(TAG_ADOBE_HUE_SAT_DATA2, data));
            }
        }
        if let Some(dims) = adobe.look_table_dims {
            entries.push(Entry::longs(TAG_ADOBE_LOOK_DIMS, &dims));
            if let Some(data) = &adobe.look_table_data {
                entries.push(Entry::floats(TAG_ADOBE_LOOK_DATA, data));
            }
        }
    }
    let matrix1 = profile
        .color_matrix1
        .iter()
        .map(|&(numerator, denominator)| {
            (numerator, i32::try_from(denominator).unwrap_or(i32::MAX))
        })
        .collect::<Vec<_>>();
    let matrix2 = profile
        .color_matrix2
        .iter()
        .map(|&(numerator, denominator)| {
            (numerator, i32::try_from(denominator).unwrap_or(i32::MAX))
        })
        .collect::<Vec<_>>();
    entries.extend([
        Entry::bytes(TAG_DNG_VERSION, TagType::Byte, profile.dng_version.to_vec()),
        Entry::bytes(
            TAG_DNG_BACKWARD_VERSION,
            TagType::Byte,
            profile.dng_backward.to_vec(),
        ),
        Entry::ascii(TAG_UNIQUE_CAMERA_MODEL, &profile.unique_camera_model),
        Entry::srationals(TAG_COLOR_MATRIX1, &matrix1),
        Entry::srationals(TAG_COLOR_MATRIX2, &matrix2),
        Entry::shorts(TAG_CALIBRATION_ILLUMINANT1, &[profile.calib_illuminant1]),
        Entry::shorts(TAG_CALIBRATION_ILLUMINANT2, &[profile.calib_illuminant2]),
        Entry::rationals(TAG_BASELINE_NOISE, &[profile.baseline_noise]),
        Entry::rationals(TAG_BASELINE_SHARPNESS, &[profile.baseline_sharpness]),
        Entry::rationals(TAG_LINEAR_RESPONSE_LIMIT, &[profile.linear_response_limit]),
        Entry::rationals(TAG_SHADOW_SCALE, &[profile.shadow_scale]),
    ]);
    entries
}

#[allow(clippy::too_many_arguments)]
pub(super) fn raw_entries(
    width: usize,
    height: usize,
    mode: RawMode,
    bit_depth: u16,
    tile_width: usize,
    tile_height: usize,
    tile_offsets: &[u64],
    tile_byte_counts: &[u64],
    profile: Option<&CameraProfile>,
    level_policy: LevelPolicy,
    compression: Compression,
    opcode2: Option<&[u8]>,
    big: bool,
) -> Vec<Entry> {
    let channels = mode.channels();
    let defaults = CameraProfile::default();
    let profile = profile.unwrap_or(&defaults);
    let depth = u8::try_from(bit_depth).unwrap_or(16);
    let mask = profile.effective_white(depth, level_policy.as_str());
    let black_values = profile.effective_black_levels(mode.as_str(), depth, level_policy.as_str());
    let cfa_pattern = cfa_pattern_bytes(profile.cfa_pattern.as_deref());
    let (crop_origin, crop_size) = crop_rect(profile, mode, width, height);
    let best_quality = if profile.fidelity.is_some() {
        (1, 1)
    } else {
        (100, 100)
    };
    let mut entries = vec![
        Entry::longs(TAG_NEW_SUBFILE_TYPE, &[0]),
        Entry::longs(TAG_IMAGE_WIDTH, &[u32::try_from(width).unwrap_or(u32::MAX)]),
        Entry::longs(
            TAG_IMAGE_LENGTH,
            &[u32::try_from(height).unwrap_or(u32::MAX)],
        ),
        Entry::shorts(TAG_BITS_PER_SAMPLE, &vec![bit_depth; channels]),
        Entry::shorts(
            TAG_COMPRESSION,
            &[if compression == Compression::None {
                1
            } else {
                8
            }],
        ),
        Entry::shorts(TAG_PHOTOMETRIC, &[mode.photometric()]),
        Entry::shorts(
            TAG_SAMPLES_PER_PIXEL,
            &[u16::try_from(channels).unwrap_or(u16::MAX)],
        ),
        Entry::shorts(TAG_PLANAR_CONFIGURATION, &[1]),
        Entry::longs(
            TAG_TILE_WIDTH,
            &[u32::try_from(tile_width).unwrap_or(u32::MAX)],
        ),
        Entry::longs(
            TAG_TILE_LENGTH,
            &[u32::try_from(tile_height).unwrap_or(u32::MAX)],
        ),
        if big {
            Entry::long8s(TAG_TILE_OFFSETS, tile_offsets)
        } else {
            Entry::longs(
                TAG_TILE_OFFSETS,
                &tile_offsets
                    .iter()
                    .map(|offset| {
                        u32::try_from(*offset).expect("classic offsets validated before IFD build")
                    })
                    .collect::<Vec<_>>(),
            )
        },
        if big {
            Entry::long8s(TAG_TILE_BYTE_COUNTS, tile_byte_counts)
        } else {
            Entry::longs(
                TAG_TILE_BYTE_COUNTS,
                &tile_byte_counts
                    .iter()
                    .map(|count| {
                        u32::try_from(*count).expect("classic byte counts fit classic TIFF")
                    })
                    .collect::<Vec<_>>(),
            )
        },
        Entry::longs(TAG_BLACK_LEVEL, &black_values),
        Entry::longs(TAG_WHITE_LEVEL, &vec![mask; channels]),
        Entry::rationals(TAG_DEFAULT_SCALE, &[(1, 1), (1, 1)]),
        Entry::rationals(
            TAG_DEFAULT_CROP_ORIGIN,
            &[
                (u32::try_from(crop_origin.0).unwrap_or(u32::MAX), 1),
                (u32::try_from(crop_origin.1).unwrap_or(u32::MAX), 1),
            ],
        ),
        Entry::rationals(
            TAG_DEFAULT_CROP_SIZE,
            &[
                (u32::try_from(crop_size.0).unwrap_or(u32::MAX), 1),
                (u32::try_from(crop_size.1).unwrap_or(u32::MAX), 1),
            ],
        ),
        Entry::rationals(TAG_ANTI_ALIAS_STRENGTH, &[(100, 100)]),
        Entry::rationals(TAG_BEST_QUALITY_SCALE, &[best_quality]),
    ];
    if mode == RawMode::Cfa && (crop_size.0 != width || crop_size.1 != height) {
        entries.push(Entry::longs(
            TAG_ACTIVE_AREA,
            &[
                0,
                0,
                u32::try_from(height).unwrap_or(u32::MAX),
                u32::try_from(width).unwrap_or(u32::MAX),
            ],
        ));
        entries.push(Entry::rationals(
            TAG_DEFAULT_USER_CROP,
            &[(0, 1), (0, 1), (1, 1), (1, 1)],
        ));
    }
    if let Some(opcode2) = opcode2 {
        entries.push(Entry::bytes(
            TAG_OPCODE_LIST2,
            TagType::Undefined,
            opcode2.to_vec(),
        ));
    } else {
        entries.push(Entry::bytes(
            TAG_OPCODE_LIST3,
            TagType::Undefined,
            profile.opcode_list3.clone(),
        ));
    }
    if matches!(compression, Compression::AdobeDeflate) {
        entries.push(Entry::shorts(TAG_PREDICTOR, &[2]));
    }
    if black_values.len() == 4 {
        entries.push(Entry::shorts(TAG_BLACK_LEVEL_REPEAT_DIM, &[2, 2]));
    }
    if let Some(green) = profile
        .fidelity
        .as_ref()
        .and_then(|fidelity| fidelity.bayer_green_split)
    {
        entries.push(Entry::longs(TAG_BAYER_GREEN_SPLIT, &[u32::from(green)]));
    }
    let adobe = profile
        .fidelity
        .as_ref()
        .and_then(|fidelity| fidelity.tag_conventions.as_deref())
        == Some("adobe");
    if !adobe {
        entries.push(Entry::rationals(TAG_X_RESOLUTION, &[(1, 1)]));
        entries.push(Entry::rationals(TAG_Y_RESOLUTION, &[(1, 1)]));
        entries.push(Entry::shorts(TAG_RESOLUTION_UNIT, &[1]));
    }
    if mode == RawMode::Cfa {
        entries.push(Entry::shorts(TAG_CFA_REPEAT_PATTERN_DIM, &[2, 2]));
        entries.push(Entry::bytes(TAG_CFA_PATTERN, TagType::Byte, cfa_pattern));
        entries.push(Entry::shorts(TAG_CFA_PLANE_COLOR, &[0, 1, 2]));
        entries.push(Entry::shorts(TAG_CFA_LAYOUT, &[1]));
    }
    entries
}

pub(super) fn crop_rect(
    profile: &CameraProfile,
    mode: RawMode,
    width: usize,
    height: usize,
) -> ((usize, usize), (usize, usize)) {
    if mode == RawMode::Cfa {
        profile
            .fidelity
            .as_ref()
            .and_then(|fidelity| fidelity.crop_margin)
            .map_or(((0, 0), (width, height)), |(x, y)| {
                ((x, y), (width - 2 * x, height - 2 * y))
            })
    } else {
        ((0, 0), (width, height))
    }
}

pub(super) fn cfa_pattern_bytes(pattern: Option<&str>) -> Vec<u8> {
    let color = |channel: u8| match channel {
        b'R' => 0,
        b'B' => 2,
        _ => 1,
    };
    match pattern {
        Some(pattern) if pattern.len() == 4 => pattern.bytes().map(color).collect(),
        _ => vec![0, 1, 1, 2],
    }
}
