//! `encode` command implementation.

use super::{
    CameraProfile, Compression, CompressionArg, Context, EncodeArgs, ImageTime, LevelPolicy,
    MetadataRandomizer, PathBuf, PreviewSource, RawMode, RecommendOptions, RecommendedConfig,
    Result, SizingMode, SplitIdArg, WriterOptions, bail, chunk_bounds, chunk_path, format_kb,
    parse_size, profiles, progress_bar, recommend, risk_warnings, split_album_dir,
    write_dng_from_file,
};
use crate::commands::{debug_file_inventory, print_write};

pub fn encode(args: &EncodeArgs) -> Result<()> {
    let _progress = progress_bar(args.progress, args.no_progress, "encoding DNG");
    let (profile, subprofile) = profiles::resolve(&args.camera_profile)?;
    let preview = preview_source(&args.thumbnail)?;
    if args.auto_size && (args.width.is_some() || args.height.is_some() || subprofile.is_some()) {
        bail!("--auto-size cannot be combined with --width/--height/--camera-profile subprofiles");
    }
    let image_time = resolve_image_time(args.image_time.as_deref(), &args.input)?;
    rawhide_core::geo::validate_radius(args.gps_radius)?;
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

    if let Some(split_size) = split_size {
        return encode_split(
            args,
            &args.input,
            input_len,
            &output,
            &config,
            profile,
            image_time.as_ref(),
            preview,
            split_size,
        );
    }

    let info = write_one(
        args,
        &args.input,
        0,
        input_len,
        &output,
        &config,
        profile,
        image_time.as_ref(),
        preview,
        0,
        None,
        None,
        None,
        None,
    )?;
    print_write(&info);
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
            let created = metadata.created().or_else(|_| metadata.modified())?;
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

fn image_time_from_system(value: std::time::SystemTime) -> ImageTime {
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
    preview: PreviewSource<'_>,
    seed_offset: u64,
    split_id: Option<&str>,
    split_seq: Option<u32>,
    split_total: Option<u32>,
    shared_location: Option<&rawhide_core::Location>,
) -> Result<rawhide_core::WriteInfo> {
    let seed = args.seed.map(|seed| seed.wrapping_add(seed_offset));
    let metadata_seed = if args.no_randomize {
        if split_seq.is_some() {
            seed.or(Some(seed_offset))
        } else {
            Some(0)
        }
    } else {
        seed
    };
    let metadata = MetadataRandomizer::new(metadata_seed, Some(profile))
        .with_gps_radius(args.gps_radius)?
        .randomize_with_location(image_time, shared_location);
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
        seed: seed.unwrap_or(seed_offset),
        split_id,
        split_seq,
        split_total,
        split_id_field: args.split_file_metadata_id.into(),
        split_seq_field: args.split_file_metadata_seq.into(),
        tile_width: 128,
        tile_height: 96,
    };
    let info = write_dng_from_file(output, payload_path, payload_offset, payload_len, &options)?;
    if args.debug_metadata_output {
        write_debug_sidecar(
            args,
            &info,
            config,
            profile,
            &metadata,
            split_seq,
            split_total,
        )?;
    }
    Ok(info)
}

fn write_debug_sidecar(
    args: &EncodeArgs,
    info: &rawhide_core::WriteInfo,
    config: &RecommendedConfig,
    profile: &CameraProfile,
    metadata: &rawhide_core::Metadata,
    split_seq: Option<u32>,
    split_total: Option<u32>,
) -> Result<()> {
    let file_inventory = debug_file_inventory(&info.path)?;
    let document = serde_json::json!({
        "schema": "stegodng-debug-metadata",
        "schema_version": 1,
        "generated_at": chrono::Utc::now().to_rfc3339(),
        "tool": {"name": "rawhide", "version": env!("CARGO_PKG_VERSION")},
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
            "thumbnail_source": args.thumbnail,
            "payload_bytes": info.payload_len,
            "chunk_seq": split_seq,
            "chunk_total": split_total,
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
                "thumbnail": args.thumbnail,
                "seed": args.seed,
                "keyed": args.key.is_some(),
                "no_randomize": args.no_randomize,
            },
            "capacity": {"bytes": info.capacity},
        },
        "profile": {
            "slug": profile.slug,
            "display_name": profile.display_name,
            "make": profile.make,
            "model": profile.model,
            "unique_camera_model": profile.unique_camera_model,
            "software": profile.software,
        },
        "metadata": metadata,
        "file": file_inventory,
    });
    let mut path = info.path.with_extension("json");
    if path == info.path {
        path = info.path.with_extension("dng.json");
    }
    std::fs::write(&path, serde_json::to_vec_pretty(&document)?)
        .with_context(|| format!("cannot write {}", path.display()))?;
    Ok(())
}

fn encode_split(
    args: &EncodeArgs,
    payload_path: &std::path::Path,
    payload_len: u64,
    output: &std::path::Path,
    config: &RecommendedConfig,
    profile: &CameraProfile,
    image_time: Option<&ImageTime>,
    preview: PreviewSource<'_>,
    split_size: u64,
) -> Result<()> {
    let bounds = chunk_bounds(
        usize::try_from(payload_len).map_err(|_| anyhow::anyhow!("payload is too large"))?,
        split_size,
    )?;
    if bounds.len() == 1 {
        let info = write_one(
            args,
            payload_path,
            0,
            payload_len,
            output,
            config,
            profile,
            image_time,
            preview,
            0,
            None,
            None,
            None,
            None,
        )?;
        print_write(&info);
        return Ok(());
    }

    let shared_location = if profile
        .fidelity
        .as_ref()
        .is_some_and(|fidelity| fidelity.emit_gps)
    {
        let mut rng =
            rawhide_core::Rng64::new(args.seed.or_else(|| args.no_randomize.then_some(0)));
        Some(rawhide_core::geo::sample_location(
            &mut rng,
            args.gps_radius,
            image_time.map(|image_time| image_time.datetime),
            image_time.map(|image_time| image_time.offset.as_str()),
            None,
        )?)
    } else {
        None
    };
    let split_id = args
        .split_file_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
    let base = if args.no_split_file_album {
        output.to_path_buf()
    } else {
        let album = split_album_dir(output, &args.input);
        std::fs::create_dir_all(&album)
            .with_context(|| format!("cannot create {}", album.display()))?;
        output
            .file_name()
            .map_or_else(|| album.clone(), |name| album.join(name))
    };

    for (seq, total, start, end) in bounds {
        let path = chunk_path(&base, seq);
        let info = write_one(
            args,
            payload_path,
            u64::try_from(start).unwrap_or(u64::MAX),
            u64::try_from(end - start).unwrap_or(u64::MAX),
            &path,
            config,
            profile,
            image_time,
            preview,
            u64::from(seq - 1),
            Some(&split_id),
            Some(seq),
            Some(total),
            shared_location.as_ref(),
        )?;
        println!(
            "wrote {} (chunk {seq}/{total}, {} of {})",
            info.path.display(),
            format_kb(info.payload_len as u64),
            format_kb(payload_len)
        );
    }
    if args.split_file_metadata_id == SplitIdArg::None {
        println!(
            "note: no shared chunk-set UUID written (--split-file-metadata-id none); \
             keep the 0001-style filenames so decode can order the chunks."
        );
    } else {
        println!(
            "note: chunks share {}={split_id} (set --split-file-metadata-id none to avoid it)",
            split_id_name(args.split_file_metadata_id)
        );
    }
    Ok(())
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
