//! `encode` command implementation.

use super::{
    CameraProfile, Compression, CompressionArg, Context, EncodeArgs, ImageTime, LevelPolicy,
    MetadataRandomizer, PathBuf, PreviewSource, ProgressGuard, RawMode, RecommendOptions,
    RecommendedConfig, Result, Rng64, SizingMode, SplitIdArg, SplitSeqArg, WriterOptions, bail,
    chunk_bounds, chunk_path, format_kb, parse_size, profiles, progress_bar, recommend,
    risk_warnings, split_album_dir, validate_split_id, write_dng_from_file,
    write_dng_from_file_with_progress,
};
use crate::commands::{debug_file_inventory, print_write};
use std::time::SystemTime;

pub fn encode(args: &EncodeArgs) -> Result<()> {
    let progress = progress_bar(args.progress, args.no_progress, "encoding DNG", 0);
    let (profile, subprofile) = profiles::resolve(&args.camera_profile)?;
    let preview = preview_source(&args.thumbnail)?;
    if args.auto_size && (args.width.is_some() || args.height.is_some() || subprofile.is_some()) {
        bail!("--auto-size cannot be combined with --width/--height/--camera-profile subprofiles");
    }
    let image_time = resolve_image_time(args.image_time.as_deref(), &args.input)?;
    rawoxide_core::geo::validate_radius(args.gps_radius)?;
    let input_len = std::fs::metadata(&args.input)
        .with_context(|| format!("cannot read {}", args.input.display()))?
        .len();
    let output = args
        .output
        .clone()
        .unwrap_or_else(|| PathBuf::from(format!("{}.dng", args.input.display())));
    let split_size = args.split_file.as_deref().map(parse_size).transpose()?;
    let payload_len = split_size.unwrap_or(input_len);
    let config = resolve_config(args, profile, subprofile, payload_len)?;
    print_auto_config(args, &config, payload_len, input_len);
    let date_anchor = chrono::Local::now().naive_local();

    if let Some(split_size) = split_size {
        return encode_split(
            args,
            &args.input,
            input_len,
            &output,
            &config,
            profile,
            image_time.as_ref(),
            date_anchor,
            &progress,
            preview,
            split_size,
        );
    }

    let outcome = write_one(
        args,
        &args.input,
        0,
        input_len,
        &output,
        &config,
        profile,
        image_time.as_ref(),
        date_anchor,
        &progress,
        preview,
        None,
        None,
        None,
        None,
        input_len,
        None,
        None,
    )?;
    print_write(&outcome.info);
    print_file_lines(args, &outcome.info);
    print_profile_meta(args, profile, &outcome.metadata);
    Ok(())
}

fn resolve_image_time(source: Option<&str>, input: &std::path::Path) -> Result<Option<ImageTime>> {
    let source = source.unwrap_or("random");
    match source {
        "random" => Ok(None),
        "now" => Ok(Some(ImageTime::local(chrono::Local::now().naive_local()))),
        "input-modified" => {
            let modified = std::fs::metadata(input)
                .and_then(|metadata| metadata.modified())
                .with_context(|| format!("cannot read mtime for {}", input.display()))?;
            Ok(Some(image_time_from_system(modified)))
        }
        "input-created" => {
            let metadata = std::fs::metadata(input)
                .with_context(|| format!("cannot read metadata for {}", input.display()))?;
            let created = file_created_timestamp(&metadata);
            Ok(Some(image_time_from_system(created)))
        }
        text => {
            if let Ok(value) = chrono::DateTime::parse_from_rfc3339(text) {
                return Ok(Some(ImageTime::from_aware(&value)));
            }
            if let Ok(value) = chrono::NaiveDateTime::parse_from_str(text, "%Y:%m:%d %H:%M:%S") {
                return Ok(Some(ImageTime::local(value)));
            }
            if let Ok(value) = chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S") {
                return Ok(Some(ImageTime::local(value)));
            }
            bail!(
                "unknown image-time source {text:?}; choose random, input-created, \
                 input-modified, now, or a YYYY:MM:DD HH:MM:SS / RFC 3339 value"
            )
        }
    }
}

fn file_created_timestamp(metadata: &std::fs::Metadata) -> SystemTime {
    if let Ok(created) = metadata.created() {
        return created;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        use std::time::{Duration, UNIX_EPOCH};
        let seconds = metadata.ctime();
        let nanos = u32::try_from(metadata.ctime_nsec()).unwrap_or(0);
        let total = i128::from(seconds) * 1_000_000_000 + i128::from(nanos);
        let magnitude = u64::try_from(total.unsigned_abs()).unwrap_or(u64::MAX);
        if total >= 0 {
            UNIX_EPOCH
                .checked_add(Duration::from_nanos(magnitude))
                .unwrap_or(UNIX_EPOCH)
        } else {
            UNIX_EPOCH
                .checked_sub(Duration::from_nanos(magnitude))
                .unwrap_or(UNIX_EPOCH)
        }
    }
    #[cfg(not(unix))]
    {
        metadata.modified().unwrap_or(std::time::UNIX_EPOCH)
    }
}

fn random_seed() -> u64 {
    Rng64::new(None).next_u64()
}

fn image_time_from_system(value: SystemTime) -> ImageTime {
    let local = chrono::DateTime::<chrono::Local>::from(value);
    ImageTime {
        datetime: local.naive_local(),
        offset: format!(
            "{}{:02}:{:02}",
            if local.offset().local_minus_utc() >= 0 {
                '+'
            } else {
                '-'
            },
            local.offset().local_minus_utc().unsigned_abs() / 3600,
            (local.offset().local_minus_utc().unsigned_abs() % 3600) / 60,
        ),
    }
}

fn preview_source(value: &str) -> Result<PreviewSource<'_>> {
    match value {
        "synthetic" => Ok(PreviewSource::Synthetic),
        "random" => Ok(PreviewSource::Random),
        path => {
            let path = std::path::Path::new(path);
            if !path.is_file() {
                bail!(
                    "unknown thumbnail source {value:?}: expected 'synthetic', 'random', \
                     or a path to an image file"
                );
            }
            Ok(PreviewSource::File(path))
        }
    }
}

fn resolve_config(
    args: &EncodeArgs,
    profile: &CameraProfile,
    subprofile: Option<&str>,
    payload_len: u64,
) -> Result<RecommendedConfig> {
    let mut width = args
        .width
        .map(usize::try_from)
        .transpose()
        .map_err(|_| anyhow::anyhow!("width is too large"))?;
    let mut height = args
        .height
        .map(usize::try_from)
        .transpose()
        .map_err(|_| anyhow::anyhow!("height is too large"))?;
    if let Some(subprofile) = subprofile {
        if width.is_some() || height.is_some() {
            bail!(
                "--width/--height cannot be combined with a --camera-profile \
                 subprofile (it pins the geometry)"
            );
        }
        let (pinned_width, pinned_height) = profile
            .subprofiles
            .get(subprofile)
            .copied()
            .ok_or_else(|| anyhow::anyhow!("unknown sub-profile {subprofile:?}"))?;
        width = Some(pinned_width);
        height = Some(pinned_height);
    }
    let bit_depth = args.bit_depth.unwrap_or(16);
    let suggested_mode = profile
        .fidelity
        .as_ref()
        .and_then(|fidelity| fidelity.suggested_mode.as_deref());
    let mode = match (args.mode, suggested_mode) {
        (Some(mode), _) => Some(SizingMode::from(mode)),
        (None, Some("cfa")) => Some(SizingMode::Cfa),
        (None, Some("linear")) => Some(SizingMode::Linear),
        (None, _) => None,
    };
    let policy = LevelPolicy::from(args.level_policy);
    let max_planes = profile.effective_max_planes(bit_depth, policy.as_str());
    let sensor_sizes = profile
        .fidelity
        .as_ref()
        .filter(|fidelity| fidelity.prefer_native)
        .map_or(&[][..], |fidelity| fidelity.sensor_sizes.as_slice());
    let prefer_max_planes = profile
        .fidelity
        .as_ref()
        .is_some_and(|fidelity| fidelity.prefer_native);
    let config = recommend(
        payload_len,
        &RecommendOptions {
            width,
            height,
            mode,
            bit_depth: args.bit_depth,
            lsb_planes: args.lsb_planes,
            auto_size: args.auto_size,
            frames: args.raw_frames,
            max_planes: Some(max_planes),
            sensor_sizes,
            prefer_max_planes,
        },
    )?;
    Ok(config)
}

fn print_auto_config(
    args: &EncodeArgs,
    config: &RecommendedConfig,
    payload_len: u64,
    input_len: u64,
) {
    let scope = if args.split_file.is_some() {
        format!(
            "per {} chunk (from {} input)",
            format_kb(payload_len),
            format_kb(input_len)
        )
    } else {
        format!("for {} input", format_kb(input_len))
    };
    if !config.auto.is_empty() {
        println!(
            "auto-config ({}): {}x{} {} {}-bit, {} LSB plane(s), capacity {} {scope}",
            config.auto.join(", "),
            config.width,
            config.height,
            config.mode.as_str(),
            config.bit_depth,
            config.lsb_planes,
            format_kb(config.capacity)
        );
    }
    if config.preset == Some("auto-size") {
        println!(
            "warning: --auto-size chose non-standard dimensions {}x{} (no camera \
             preset matches); the file is smaller, but unusual dimensions may look \
             suspicious under inspection.",
            config.width, config.height
        );
    }
    let display_name = profiles::resolve(&args.camera_profile)
        .ok()
        .map(|(profile, _)| profile.display_name.as_str());
    for line in risk_warnings(config, args.key.is_some(), display_name) {
        println!("{line}");
    }
}

#[allow(clippy::too_many_arguments)]
fn write_one(
    args: &EncodeArgs,
    payload_path: &std::path::Path,
    payload_offset: u64,
    payload_len: u64,
    output: &std::path::Path,
    config: &RecommendedConfig,
    profile: &CameraProfile,
    image_time: Option<&ImageTime>,
    date_anchor: chrono::NaiveDateTime,
    progress: &ProgressGuard,
    preview: PreviewSource<'_>,
    chunk_seed: Option<u64>,
    split_id: Option<&str>,
    split_seq: Option<u32>,
    split_total: Option<u32>,
    total_payload_bytes: u64,
    location_seed: Option<u64>,
    shared_location: Option<&rawoxide_core::Location>,
) -> Result<WriteOutcome> {
    // Split chunks receive an explicit per-chunk seed. A plain encode keeps
    // The cover uses seed 0 unless --seed was given, while the metadata
    // stream draws its own seed when --seed is absent.
    let cover_seed = chunk_seed.unwrap_or_else(|| args.seed.unwrap_or(0));
    let metadata_seed = if args.no_randomize {
        Some(chunk_seed.unwrap_or(0))
    } else {
        Some(chunk_seed.unwrap_or_else(|| args.seed.unwrap_or_else(random_seed)))
    };
    let metadata = MetadataRandomizer::new(metadata_seed, Some(profile))
        .with_gps_radius(args.gps_radius)?
        .with_date_anchor(date_anchor)
        .randomize_with_location(image_time, shared_location);
    let (tile_width, tile_height) = profile.raw_tile_wh().unwrap_or((128, 96));
    let options = WriterOptions {
        width: config.width,
        height: config.height,
        mode: match config.mode {
            SizingMode::Linear => RawMode::Linear,
            SizingMode::Cfa => RawMode::Cfa,
        },
        bit_depth: u16::from(config.bit_depth),
        lsb_planes: config.lsb_planes,
        key: args.key.as_deref().map(str::as_bytes),
        profile: Some(profile),
        metadata: Some(&metadata),
        level_policy: LevelPolicy::from(args.level_policy),
        compression: Compression::from(args.compression),
        preview,
        force_bigtiff: args.bigtiff,
        frames: config.frames,
        seed: cover_seed,
        split_id,
        split_seq,
        split_total,
        split_id_field: args.split_file_metadata_id.into(),
        split_seq_field: args.split_file_metadata_seq.into(),
        tile_width,
        tile_height,
        debug_stats: args.debug_metadata_output,
    };
    let message = split_seq.map_or_else(
        || "encoding DNG".to_owned(),
        |seq| format!("encoding chunk {seq}/{}", split_total.unwrap_or(0)),
    );
    let total_rows =
        u64::from(config.frames).saturating_mul(u64::try_from(config.height).unwrap_or(u64::MAX));
    let info = if progress.is_enabled() {
        progress.set(0, total_rows, &message);
        write_dng_from_file_with_progress(
            output,
            payload_path,
            payload_offset,
            payload_len,
            &options,
            |done, total| progress.set(done, total, &message),
        )?
    } else {
        write_dng_from_file(output, payload_path, payload_offset, payload_len, &options)?
    };
    if args.debug_metadata_output {
        write_debug_sidecar(
            args,
            &info,
            config,
            profile,
            &metadata,
            &SidecarContext {
                metadata_seed: metadata_seed.unwrap_or(0),
                cover_seed,
                total_payload_bytes,
                location_seed,
                image_time,
                split_id,
                split_seq,
                split_total,
            },
        )?;
    }
    Ok(WriteOutcome { info, metadata })
}

struct SidecarContext<'a> {
    metadata_seed: u64,
    cover_seed: u64,
    total_payload_bytes: u64,
    location_seed: Option<u64>,
    image_time: Option<&'a ImageTime>,
    split_id: Option<&'a str>,
    split_seq: Option<u32>,
    split_total: Option<u32>,
}

struct WriteOutcome {
    info: rawoxide_core::WriteInfo,
    metadata: rawoxide_core::Metadata,
}

fn write_debug_sidecar(
    args: &EncodeArgs,
    info: &rawoxide_core::WriteInfo,
    config: &RecommendedConfig,
    profile: &CameraProfile,
    metadata: &rawoxide_core::Metadata,
    context: &SidecarContext<'_>,
) -> Result<()> {
    let file_inventory = debug_file_inventory(&info.path)?;
    let id_field: rawoxide_core::SplitIdField = args.split_file_metadata_id.into();
    let seq_field: rawoxide_core::SplitSeqField = args.split_file_metadata_seq.into();
    let container = if info.bit_depth <= 8 {
        "uint8"
    } else {
        "uint16"
    };
    let channels = match info.mode {
        RawMode::Linear => 3_u64,
        RawMode::Cfa => 1_u64,
    };
    let samples_per_frame = u64::try_from(info.width)
        .unwrap_or(u64::MAX)
        .saturating_mul(u64::try_from(info.height).unwrap_or(u64::MAX))
        .saturating_mul(channels);
    let fill_ratio = if info.capacity == 0 {
        serde_json::Value::Null
    } else {
        serde_json::json!(fill_ratio(info.payload_len, info.capacity))
    };
    let frame_shape = if info.mode == RawMode::Linear {
        serde_json::json!([info.height, info.width, 3])
    } else {
        serde_json::json!([info.height, info.width])
    };
    let frame_records = (0..usize::from(info.frames))
        .map(|index| {
            serde_json::json!({
                "index": index,
                "seed": context.cover_seed.wrapping_add(u64::try_from(index).unwrap_or(u64::MAX)),
                "shape": frame_shape,
                "dtype": container,
                "crc32": info
                    .frame_crc32s
                    .get(index)
                    .map_or_else(String::new, |crc| format!("{crc:08x}")),
            })
        })
        .collect::<Vec<_>>();
    let split = context.split_seq.map(|seq| {
        serde_json::json!({
            "chunk_seq": seq,
            "chunk_total": context.split_total,
            "split_id": context.split_id.unwrap_or(""),
            "split_id_field": id_field.as_str(),
            "split_seq_field": seq_field.as_str(),
        })
    });
    let image_time = context
        .image_time
        .map(|time| time.datetime.format("%Y-%m-%dT%H:%M:%S").to_string());
    let mut metadata_json = serde_json::to_value(metadata)?;
    if let Some(fidelity) = profile.fidelity.as_ref() {
        metadata_json["fidelity_layout"] = serde_json::json!(false);
        let mut allow = fidelity.exif_allow.clone();
        if info.mode != RawMode::Cfa
            && let Some(tags) = allow.as_mut()
        {
            tags.retain(|tag| *tag != 41_730);
        }
        metadata_json["exif_allow"] = serde_json::json!(allow);
        metadata_json["emit_serials"] = serde_json::json!(fidelity.emit_serials);
        metadata_json["emit_lens"] = serde_json::json!(fidelity.emit_lens);
        if info.mode == RawMode::Cfa
            && profile
                .cfa_pattern
                .as_deref()
                .is_some_and(|pattern| pattern.eq_ignore_ascii_case("BGGR"))
        {
            metadata_json["photo_cfa_pattern"] =
                crate::sidecar::bytes_json(b"\x02\x00\x02\x00\x02\x01\x01\x00");
        }
        if let Some(makernote) = &metadata.makernote {
            metadata_json["makernote"] = crate::sidecar::bytes_json(makernote);
        }
    }
    let mut document = serde_json::json!({
        "schema": "stegodng-debug-metadata",
        "schema_version": 1,
        "generated_at": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "tool": {
            "name": "rawoxide",
            "version": env!("CARGO_PKG_VERSION"),
            "target_os": std::env::consts::OS,
            "target_arch": std::env::consts::ARCH,
        },
        "invocation": {
            "input": args.input.display().to_string(),
            "camera_profile_requested": &args.camera_profile,
            "image_time_source": args.image_time.as_deref().unwrap_or("random"),
            "auto_config": {
                "preset": config.preset,
                "auto": &config.auto,
            },
        },
        "summary": {
            "output": info.path.display().to_string(),
            "output_bytes": info.size,
            "width": info.width,
            "height": info.height,
            "mode": info.mode.as_str(),
            "bit_depth": info.bit_depth,
            "lsb_planes": info.lsb_planes,
            "frames": info.frames,
            "compression": args.compression_name(),
            "bigtiff": info.bigtiff,
            "thumbnail_source": &info.thumbnail_label,
            "payload_bytes": info.payload_len,
        },
        "encode": {
            "options": {
                "width": config.width,
                "height": config.height,
                "mode": config.mode.as_str(),
                "bit_depth": config.bit_depth,
                "lsb_planes": config.lsb_planes,
                "frames": config.frames,
                "compression": args.compression_name(),
                "thumbnail": &args.thumbnail,
                "seed": args.seed,
                "keyed": args.key.is_some(),
                "no_randomize": args.no_randomize,
                "image_time": image_time,
                "gps_radius_km": args.gps_radius,
                "location_seed": context.location_seed,
                "camera_profile": &profile.slug,
                "camera_profile_name": &profile.display_name,
                "level_policy": LevelPolicy::from(args.level_policy).as_str(),
            },
            "payload": {
                "bytes": info.payload_len,
                "total_bytes": context.total_payload_bytes,
                "frame_bytes": info.payload_len.saturating_add(rawoxide_core::PayloadFrame::HEADER_LEN),
                "frame_crc32": info.payload_frame_crc32.map_or_else(String::new, |crc| format!("{crc:08x}")),
            },
            "output": {
                "path": info.path.display().to_string(),
                "bytes": info.size,
                "bigtiff": info.bigtiff,
                "container": container,
            },
            "capacity": {
                "bytes": info.capacity,
                "samples_per_frame": samples_per_frame,
                "frames": info.frames,
                "fill_ratio": fill_ratio,
            },
            "split": split,
        },
        "profile": crate::sidecar::profile_json(profile),
        "cover": {
            "generator": "synthetic gradient + noise",
            "seed": context.cover_seed,
            "metadata_seed": context.metadata_seed,
            "white_level": info.white_level,
            "white_level_mode": if info.white_level.is_some() {
                "profile-effective"
            } else {
                "full-container"
            },
            "headroom": info.headroom,
            "frame_crc32_scope": "embedded-raw",
            "frames": frame_records,
        },
        "thumbnail": {
            "requested": &args.thumbnail,
            "source": &info.thumbnail_label,
            "shape": [info.thumbnail_height, info.thumbnail_width, 3],
            "dtype": "uint8",
            "crc32": format!("{:08x}", info.thumbnail_crc32),
        },
        "metadata": metadata_json,
        "file": file_inventory,
    });
    if let Some(seq) = context.split_seq {
        document["summary"]["chunk_seq"] = serde_json::json!(seq);
        document["summary"]["chunk_total"] = serde_json::json!(context.split_total);
    }
    let mut path = info.path.with_extension("json");
    if path == info.path {
        path = info.path.with_extension("dng.json");
    }
    let mut bytes = serde_json::to_vec_pretty(&document)?;
    bytes.push(b'\n');
    if let Err(error) = std::fs::write(&path, &bytes) {
        let _ = std::fs::remove_file(&path);
        return Err(error).with_context(|| format!("cannot write {}", path.display()));
    }
    Ok(())
}

fn round6(value: f64) -> f64 {
    (value * 1_000_000.0).round() / 1_000_000.0
}

#[allow(clippy::cast_precision_loss)]
fn fill_ratio(payload: usize, capacity: u64) -> f64 {
    round6(payload as f64 / capacity as f64)
}

#[allow(clippy::too_many_arguments)]
fn encode_split(
    args: &EncodeArgs,
    payload_path: &std::path::Path,
    payload_len: u64,
    output: &std::path::Path,
    config: &RecommendedConfig,
    profile: &CameraProfile,
    image_time: Option<&ImageTime>,
    date_anchor: chrono::NaiveDateTime,
    progress: &ProgressGuard,
    preview: PreviewSource<'_>,
    split_size: u64,
) -> Result<()> {
    let bounds = chunk_bounds(
        usize::try_from(payload_len).map_err(|_| anyhow::anyhow!("payload is too large"))?,
        split_size,
    )?;
    let split_id = args
        .split_file_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
    validate_split_id(&split_id)?;

    // The output moves into the album directory even when the payload fits a
    // single chunk. Build the album before the one-chunk
    // early return, and remove it again if encoding fails before writing.
    let use_album = args.split_file_album || !args.no_split_file_album;
    let (base, mut album_guard) = if use_album {
        let album = split_album_dir(output, &args.input);
        let created = !album.exists();
        std::fs::create_dir_all(&album)
            .with_context(|| format!("cannot create {}", album.display()))?;
        let guard = if created {
            AlbumGuard {
                path: Some(album.clone()),
            }
        } else {
            AlbumGuard::default()
        };
        let base = output
            .file_name()
            .map_or_else(|| album.clone(), |name| album.join(name));
        (base, guard)
    } else {
        (output.to_path_buf(), AlbumGuard::default())
    };

    if bounds.len() == 1 {
        let outcome = write_one(
            args,
            payload_path,
            0,
            payload_len,
            &base,
            config,
            profile,
            image_time,
            date_anchor,
            progress,
            preview,
            None,
            None,
            None,
            None,
            payload_len,
            None,
            None,
        )?;
        print_write(&outcome.info);
        print_file_lines(args, &outcome.info);
        print_profile_meta(args, profile, &outcome.metadata);
        album_guard.disarm();
        return Ok(());
    }

    let emit_gps = profile
        .fidelity
        .as_ref()
        .is_some_and(|fidelity| fidelity.emit_gps);
    let location_seed = emit_gps.then(|| {
        args.seed
            .or_else(|| args.no_randomize.then_some(0))
            .unwrap_or_else(random_seed)
    });
    let shared_location = if let Some(location_seed) = location_seed {
        let mut rng = Rng64::new(Some(location_seed));
        Some(rawoxide_core::geo::sample_location(
            &mut rng,
            args.gps_radius,
            image_time.map(|image_time| image_time.datetime),
            image_time.map(|image_time| image_time.offset.as_str()),
            None,
        )?)
    } else {
        None
    };

    let mut outcomes = Vec::with_capacity(bounds.len());
    for (seq, total, start, end) in bounds {
        let path = chunk_path(&base, seq);
        // An unseeded split set draws a fresh chunk seed, so unrelated sets
        // and chunks never share cover pixels. Seeded and --no-randomize runs
        // stay reproducible.
        let chunk_seed = if let Some(seed) = args.seed {
            Some(seed.wrapping_add(u64::from(seq - 1)))
        } else if args.no_randomize {
            Some(u64::from(seq - 1))
        } else {
            Some(random_seed())
        };
        let outcome = write_one(
            args,
            payload_path,
            u64::try_from(start).unwrap_or(u64::MAX),
            u64::try_from(end - start).unwrap_or(u64::MAX),
            &path,
            config,
            profile,
            image_time,
            date_anchor,
            progress,
            preview,
            chunk_seed,
            Some(&split_id),
            Some(seq),
            Some(total),
            payload_len,
            location_seed,
            shared_location.as_ref(),
        )?;
        outcomes.push((seq, total, outcome));
    }
    album_guard.disarm();
    if outcomes.iter().any(|(_, _, outcome)| outcome.info.bigtiff) {
        println!(
            "warning: raw data exceeds classic TIFF limits; written as 64-bit BigTIFF \
             (DNG 1.6). tifffile reads it, but some tools (e.g. exiv2) cannot parse BigTIFF."
        );
    }
    for (seq, total, outcome) in &outcomes {
        println!(
            "wrote {} (chunk {seq}/{total}, {} of {})",
            outcome.info.path.display(),
            format_kb(outcome.info.payload_len as u64),
            format_kb(payload_len)
        );
    }
    let id_none = args.split_file_metadata_id == SplitIdArg::None;
    let seq_none = args.split_file_metadata_seq == SplitSeqArg::None;
    if id_none && seq_none {
        println!(
            "note: no split markers written to metadata; keep the 0001-style \
             filenames so decode can order the chunks."
        );
    } else {
        if id_none {
            println!("note: no shared chunk-set UUID written (--split-file-metadata-id none)");
        } else {
            println!(
                "note: chunks share {}={split_id} (links the set; use \
                 --split-file-metadata-id none to avoid it)",
                split_id_name(args.split_file_metadata_id)
            );
        }
        if seq_none {
            println!("note: no sequence numbers in metadata; decode orders by filename.");
        }
    }
    for (_, _, outcome) in &outcomes {
        print_file_lines(args, &outcome.info);
    }
    if let Some((_, _, first)) = outcomes.first() {
        print_profile_meta(args, profile, &first.metadata);
    }
    Ok(())
}

fn print_file_lines(args: &EncodeArgs, info: &rawoxide_core::WriteInfo) {
    println!(
        "thumbnail: {}: {}",
        info.path.display(),
        info.thumbnail_label
    );
    if args.debug_metadata_output {
        println!("debug: {}", sidecar_path(&info.path).display());
    }
}

fn print_profile_meta(
    args: &EncodeArgs,
    profile: &CameraProfile,
    metadata: &rawoxide_core::Metadata,
) {
    let display = if profile.display_name.is_empty() {
        profile.model.as_str()
    } else {
        profile.display_name.as_str()
    };
    println!("profile: {} ({display})", args.camera_profile);
    println!(
        "meta: {} ISO{} {} S/N {}",
        metadata.datetime, metadata.iso, metadata.lens, metadata.camera_serial
    );
}

fn sidecar_path(path: &std::path::Path) -> PathBuf {
    let mut sidecar = path.with_extension("json");
    if sidecar == path {
        sidecar = path.with_extension("dng.json");
    }
    sidecar
}

/// Removes an album directory created by a failed split encode.
///
/// Only empty directories are removed, so a partial chunk set is never
/// deleted behind the user's back.
#[derive(Debug, Default)]
struct AlbumGuard {
    path: Option<PathBuf>,
}

impl AlbumGuard {
    fn disarm(&mut self) {
        self.path = None;
    }
}

impl Drop for AlbumGuard {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            let _ = std::fs::remove_dir(path);
        }
    }
}

impl EncodeArgs {
    const fn compression_name(&self) -> &'static str {
        match self.compression {
            CompressionArg::None => "none",
            CompressionArg::AdobeDeflate => "adobe_deflate",
        }
    }
}

const fn split_id_name(value: SplitIdArg) -> &'static str {
    match value {
        SplitIdArg::ImageUniqueId => "ImageUniqueID",
        SplitIdArg::ImageDescription => "ImageDescription",
        SplitIdArg::None => "none",
    }
}
