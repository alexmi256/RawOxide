//! `rawhide` command-line interface.
//!
//! The CLI is intentionally thin: it parses arguments, resolves defaults and
//! delegates to `rawhide-core`. `capacity`, `encode`, `decode` and `gen-tiff`
//! are all implemented.

use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use rawhide_core::{
    CameraProfile, Compression, DecodeOptions, ImageTime, LevelPolicy, MetadataRandomizer,
    PreviewSource, RawMode, RecommendOptions, RecommendedConfig, Rng64, SizingMode, SplitIdField,
    SplitSeqField, WriterOptions, capacity_bytes, chunk_bounds, chunk_path, decode_files_to_writer,
    decode_files_to_writer_with_progress, format_kb, parse_size, profiles, recommend,
    risk_warnings, split_album_dir, validate_split_id, write_cover_tiff, write_dng_from_file,
    write_dng_from_file_with_progress,
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

    /// Preview source: synthetic, random (Wikimedia Commons, offline-noise
    /// fallback), or an image path.
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

    /// Force progress output.
    #[arg(long, conflicts_with = "no_progress")]
    progress: bool,

    /// Disable progress output.
    #[arg(long)]
    no_progress: bool,

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

    /// Group split chunks in an input-stem album directory (default).
    #[arg(long, conflicts_with = "no_split_file_album")]
    split_file_album: bool,

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

    /// Force progress output.
    #[arg(long, conflicts_with = "no_progress")]
    progress: bool,

    /// Disable progress output.
    #[arg(long)]
    no_progress: bool,
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

struct ProgressGuard(Option<ProgressBar>);

impl ProgressGuard {
    const fn is_enabled(&self) -> bool {
        self.0.is_some()
    }

    fn set(&self, position: u64, length: u64, message: &str) {
        if let Some(bar) = &self.0 {
            if bar.length() != Some(length) {
                // Reset before changing the length so a shorter next chunk
                // never renders at a position beyond its own total.
                bar.set_position(0);
                bar.set_length(length);
            }
            bar.set_position(position);
            bar.set_message(message.to_owned());
        }
    }
}

impl Drop for ProgressGuard {
    fn drop(&mut self) {
        if let Some(bar) = self.0.take() {
            bar.finish_and_clear();
        }
    }
}

#[allow(clippy::literal_string_with_formatting_args)]
fn progress_bar(force: bool, disabled: bool, message: &str, length: u64) -> ProgressGuard {
    if disabled || (!force && !std::io::stderr().is_terminal()) {
        return ProgressGuard(None);
    }
    let draw_target = if force {
        ProgressDrawTarget::term_like_with_hz(Box::new(console::Term::buffered_stderr()), 20)
    } else {
        ProgressDrawTarget::stderr()
    };
    let bar = ProgressBar::with_draw_target(Some(length.max(1)), draw_target);
    bar.set_style(
        ProgressStyle::with_template("{spinner} {msg} [{bar:28}] {percent:>3}% ({pos}/{len})")
            .unwrap_or_else(|_| ProgressStyle::default_bar()),
    );
    bar.set_message(message.to_owned());
    bar.enable_steady_tick(Duration::from_millis(100));
    ProgressGuard(Some(bar))
}

mod commands;
mod encode;
mod sidecar;

use commands::{capacity, decode, gen_tiff};
use encode::encode;
