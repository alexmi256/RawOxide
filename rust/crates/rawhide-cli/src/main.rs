//! `rawhide` command-line interface.
//!
//! The CLI is intentionally thin: it parses arguments, resolves defaults and
//! delegates to `rawhide-core`. `capacity`, `encode` and `decode` are
//! functional; `gen-tiff` is still scaffolded.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use rawhide_core::{
    CameraProfile, Compression, DecodeOptions, ImageTime, LevelPolicy, MetadataRandomizer,
    PreviewSource, RawMode, RecommendOptions, RecommendedConfig, SizingMode, SplitIdField,
    SplitSeqField, WriterOptions, capacity_bytes, chunk_bounds, chunk_path, decode_files,
    format_kb, parse_size, profiles, recommend, risk_warnings, split_album_dir, write_cover_tiff,
    write_dng,
};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(
    name = "rawhide",
    version,
    about = "Hide arbitrary data in DNG raw sensor data",
    long_about = "RawHide embeds a framed payload in the low bit planes of DNG raw samples. \
                  This Rust port is under active development; see rust/docs/port-plan.md."
)]
struct Cli {
    /// Increase log verbosity (-v, -vv).
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Print the sample count and payload capacity for a geometry.
    Capacity(CapacityArgs),
    /// Embed a payload into a new DNG.
    Encode(EncodeArgs),
    /// Extract a payload from one or more DNG files.
    Decode(DecodeArgs),
    /// Generate a plain TIFF cover.
    GenTiff(GenTiffArgs),
}

#[derive(Debug, Args)]
struct CapacityArgs {
    /// Raw frame width in pixels.
    #[arg(long, default_value_t = 2048, value_parser = clap::value_parser!(u32).range(1..))]
    width: u32,

    /// Raw frame height in pixels.
    #[arg(long, default_value_t = 1536, value_parser = clap::value_parser!(u32).range(1..))]
    height: u32,

    /// Raw layout: demosaiced RGB or single-channel CFA.
    #[arg(long, value_enum, default_value_t = Mode::Linear)]
    mode: Mode,

    /// LSB planes per sample (1-16).
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u8).range(1..=16))]
    lsb_planes: u8,

    /// Full-resolution raw frames sharing the file (1-8).
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u8).range(1..=8))]
    raw_frames: u8,
}

#[derive(Debug, Args)]
struct EncodeArgs {
    /// Payload file to embed.
    #[arg(short, long)]
    input: PathBuf,

    /// Output DNG path (defaults to <input>.dng).
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Seed for cover pixels, thumbnails and metadata.
    #[arg(long)]
    seed: Option<u64>,

    /// Preview source: synthetic, random (offline noise), or an image path.
    #[arg(long, default_value = "synthetic")]
    thumbnail: String,

    /// Raw frame width in pixels (auto-selected when omitted).
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
    width: Option<u32>,

    /// Raw frame height in pixels (auto-selected when omitted).
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
    height: Option<u32>,

    /// Compute minimal dimensions instead of using the preset geometries.
    #[arg(long)]
    auto_size: bool,

    /// LSB planes per sample (1-16; smallest fitting count when omitted).
    #[arg(long, value_parser = clap::value_parser!(u8).range(1..=16))]
    lsb_planes: Option<u8>,

    /// Raw layout (defaults to the profile's suggestion, then linear).
    #[arg(long, value_enum)]
    mode: Option<EncodeMode>,

    /// Packed `BitsPerSample` (8/10/12/14/16; defaults to 16).
    #[arg(long, value_parser = clap::value_parser!(u8).range(8..=16))]
    bit_depth: Option<u8>,

    /// Full-resolution raw frames in one file (1-8).
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u8).range(1..=8))]
    raw_frames: u8,

    /// Camera profile slug (optionally `slug-subprofile`).
    #[arg(long, default_value = "gfx_100")]
    camera_profile: String,

    /// GPS sampling radius in km (0-25).
    #[arg(long, default_value_t = rawhide_core::geo::MAX_RADIUS_KM)]
    gps_radius: f64,

    /// Fidelity level policy.
    #[arg(long, value_enum, default_value_t = LevelPolicyArg::Profile)]
    level_policy: LevelPolicyArg,

    /// Raw tile compression (deflate only with bit depth 8/16).
    #[arg(long, value_enum, default_value_t = CompressionArg::None)]
    compression: CompressionArg,

    /// Force `BigTIFF` output even for small files.
    #[arg(long)]
    bigtiff: bool,

    /// Disable randomized metadata; use deterministic seed 0.
    #[arg(long)]
    no_randomize: bool,

    /// Capture time: random, now, input-created, input-modified,
    /// `YYYY:MM:DD HH:MM:SS`, or RFC 3339.
    #[arg(long)]
    image_time: Option<String>,

    /// Optional passphrase for the XOR keystream.
    #[arg(long)]
    key: Option<String>,

    /// Write a JSON metadata sidecar next to each output.
    #[arg(long)]
    debug_metadata_output: bool,

    /// Split the payload into chunks of at most SIZE per DNG.
    #[arg(long, value_name = "SIZE")]
    split_file: Option<String>,

    /// Metadata field holding the shared chunk-set UUID.
    #[arg(long, value_enum, default_value = "ImageUniqueID")]
    split_file_metadata_id: SplitIdArg,

    /// Metadata field holding the chunk sequence number.
    #[arg(long, value_enum, default_value = "ImageNumber")]
    split_file_metadata_seq: SplitSeqArg,

    /// Override the generated chunk-set UUID.
    #[arg(long)]
    split_file_id: Option<String>,

    /// Write chunks beside the output instead of an input-stem album directory.
    #[arg(long)]
    no_split_file_album: bool,
}

#[derive(Debug, Args)]
struct DecodeArgs {
    /// One DNG, or several chunk files to concatenate.
    #[arg(short, long, required = true, num_args = 1..)]
    input: Vec<PathBuf>,

    /// Recovered payload path.
    #[arg(short, long)]
    output: PathBuf,

    /// Passphrase used at encode time.
    #[arg(long)]
    key: Option<String>,

    /// LSB planes used at encode time (auto-detect when omitted).
    #[arg(long, value_parser = clap::value_parser!(u8).range(1..=16))]
    lsb_planes: Option<u8>,

    /// Safety cap on the declared payload per file.
    #[arg(long, value_name = "SIZE")]
    max_bytes: Option<String>,

    /// Metadata field holding the shared chunk-set UUID.
    #[arg(long, value_enum, default_value = "ImageUniqueID")]
    split_file_metadata_id: SplitIdArg,

    /// Metadata field holding the chunk sequence number.
    #[arg(long, value_enum, default_value = "ImageNumber")]
    split_file_metadata_seq: SplitSeqArg,
}

#[derive(Debug, Args)]
struct GenTiffArgs {
    /// Output TIFF path.
    #[arg(short, long)]
    output: PathBuf,

    /// Image width.
    #[arg(long, default_value_t = 2048, value_parser = clap::value_parser!(u32).range(1..))]
    width: u32,

    /// Image height.
    #[arg(long, default_value_t = 1536, value_parser = clap::value_parser!(u32).range(1..))]
    height: u32,

    /// Seed for the synthetic cover.
    #[arg(long)]
    seed: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Mode {
    /// Demosaiced RGB: three samples per pixel.
    Linear,
    /// Bayer mosaic: one sample per pixel.
    Cfa,
}

impl Mode {
    const fn channels(self) -> u64 {
        match self {
            Self::Linear => 3,
            Self::Cfa => 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum CompressionArg {
    /// Uncompressed tiles.
    None,
    /// Adobe Deflate with horizontal predictor.
    #[value(name = "adobe_deflate")]
    AdobeDeflate,
}

impl From<CompressionArg> for Compression {
    fn from(value: CompressionArg) -> Self {
        match value {
            CompressionArg::None => Self::None,
            CompressionArg::AdobeDeflate => Self::AdobeDeflate,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum LevelPolicyArg {
    /// Use the profile's `raw.level_policy`.
    Profile,
    /// Keep the full container range.
    Capacity,
    /// Honor the profile white level and plane ceiling.
    Strict,
}

impl From<LevelPolicyArg> for LevelPolicy {
    fn from(value: LevelPolicyArg) -> Self {
        match value {
            LevelPolicyArg::Profile => Self::Profile,
            LevelPolicyArg::Capacity => Self::Capacity,
            LevelPolicyArg::Strict => Self::Strict,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum EncodeMode {
    /// Demosaiced RGB: three samples per pixel.
    Linear,
    /// Bayer mosaic: one sample per pixel.
    Cfa,
}

impl From<EncodeMode> for SizingMode {
    fn from(value: EncodeMode) -> Self {
        match value {
            EncodeMode::Linear => Self::Linear,
            EncodeMode::Cfa => Self::Cfa,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum SplitIdArg {
    #[value(name = "ImageUniqueID")]
    ImageUniqueId,
    #[value(name = "ImageDescription")]
    ImageDescription,
    #[value(name = "none")]
    None,
}

impl From<SplitIdArg> for SplitIdField {
    fn from(value: SplitIdArg) -> Self {
        match value {
            SplitIdArg::ImageUniqueId => Self::ImageUniqueId,
            SplitIdArg::ImageDescription => Self::ImageDescription,
            SplitIdArg::None => Self::None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum SplitSeqArg {
    #[value(name = "ImageNumber")]
    ImageNumber,
    #[value(name = "PageNumber")]
    PageNumber,
    #[value(name = "ImageDescription")]
    ImageDescription,
    #[value(name = "none")]
    None,
}

impl From<SplitSeqArg> for SplitSeqField {
    fn from(value: SplitSeqArg) -> Self {
        match value {
            SplitSeqArg::ImageNumber => Self::ImageNumber,
            SplitSeqArg::PageNumber => Self::PageNumber,
            SplitSeqArg::ImageDescription => Self::ImageDescription,
            SplitSeqArg::None => Self::None,
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn init_tracing(verbose: u8) {
    let default = match verbose {
        0 => "warn",
        1 => "info",
        _ => "debug",
    };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}

fn run(cli: &Cli) -> Result<()> {
    match &cli.command {
        Command::Capacity(args) => {
            capacity(args);
            Ok(())
        }
        Command::Encode(args) => encode(args),
        Command::Decode(args) => decode(args),
        Command::GenTiff(args) => gen_tiff(args),
    }
}

fn encode(args: &EncodeArgs) -> Result<()> {
    let (profile, subprofile) = profiles::resolve(&args.camera_profile)?;
    let preview = preview_source(&args.thumbnail)?;
    if args.auto_size && (args.width.is_some() || args.height.is_some() || subprofile.is_some()) {
        bail!("--auto-size cannot be combined with --width/--height/--camera-profile subprofiles");
    }
    let image_time = resolve_image_time(args.image_time.as_deref(), &args.input)?;
    rawhide_core::geo::validate_radius(args.gps_radius)?;
    let payload = std::fs::read(&args.input)
        .with_context(|| format!("cannot read {}", args.input.display()))?;
    let output = args
        .output
        .clone()
        .unwrap_or_else(|| PathBuf::from(format!("{}.dng", args.input.display())));
    let split_size = args.split_file.as_deref().map(parse_size).transpose()?;
    let payload_len = split_size.unwrap_or(payload.len() as u64);
    let config = resolve_config(args, profile, subprofile, payload_len)?;
    print_auto_config(args, &config, payload_len, payload.len() as u64);

    if let Some(split_size) = split_size {
        return encode_split(
            args,
            &payload,
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
        &payload,
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
    payload: &[u8],
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
    let info = write_dng(output, payload, &options)?;
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
    payload: &[u8],
    output: &std::path::Path,
    config: &RecommendedConfig,
    profile: &CameraProfile,
    image_time: Option<&ImageTime>,
    preview: PreviewSource<'_>,
    split_size: u64,
) -> Result<()> {
    let bounds = chunk_bounds(payload.len(), split_size)?;
    if bounds.len() == 1 {
        let info = write_one(
            args, payload, output, config, profile, image_time, preview, 0, None, None, None, None,
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
            &payload[start..end],
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
            format_kb(payload.len() as u64)
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

fn gen_tiff(args: &GenTiffArgs) -> Result<()> {
    let width = usize::try_from(args.width).map_err(|_| anyhow::anyhow!("width is too large"))?;
    let height =
        usize::try_from(args.height).map_err(|_| anyhow::anyhow!("height is too large"))?;
    let size = write_cover_tiff(&args.output, width, height, args.seed, "RawHide Rust cover")?;
    println!("wrote {} ({})", args.output.display(), format_kb(size));
    Ok(())
}

fn print_write(info: &rawhide_core::WriteInfo) {
    if info.bigtiff {
        println!("warning: wrote BigTIFF (64-bit offsets); some tools cannot parse it.");
    }
    println!("wrote {} ({})", info.path.display(), format_kb(info.size));
    println!(
        "payload {} / capacity {}",
        format_kb(info.payload_len as u64),
        format_kb(info.capacity)
    );
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

fn decode(args: &DecodeArgs) -> Result<()> {
    let max_bytes = args.max_bytes.as_deref().map(parse_size).transpose()?;
    let key = args.key.as_deref().map(str::as_bytes);
    let options = DecodeOptions {
        key,
        lsb_planes: args.lsb_planes,
        max_bytes,
        split_id_field: args.split_file_metadata_id.into(),
        split_seq_field: args.split_file_metadata_seq.into(),
    };

    let outcome = decode_files(&args.input, &options)?;
    for warning in &outcome.warnings {
        eprintln!("warning: {warning}");
    }
    std::fs::write(&args.output, &outcome.payload)
        .with_context(|| format!("cannot write {}", args.output.display()))?;
    println!(
        "recovered {} -> {}",
        format_kb(outcome.payload.len() as u64),
        args.output.display()
    );
    Ok(())
}

fn capacity(args: &CapacityArgs) {
    let samples = u64::from(args.width)
        .saturating_mul(u64::from(args.height))
        .saturating_mul(args.mode.channels())
        .saturating_mul(u64::from(args.raw_frames));
    let capacity = capacity_bytes(samples, args.lsb_planes);
    let frame_suffix = if args.raw_frames > 1 {
        format!(" x {} frames", args.raw_frames)
    } else {
        String::new()
    };

    println!("samples: {samples}");
    println!(
        "capacity ({} LSB plane(s){frame_suffix}): {}",
        args.lsb_planes,
        format_kb(capacity)
    );
}
