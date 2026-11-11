//! Remaining command implementations and debug inventory.

use std::io::Write as _;
use std::path::Path;

use super::{
    CapacityArgs, Context, DecodeArgs, DecodeOptions, GenTiffArgs, Result, capacity_bytes,
    decode_files_to_writer, decode_files_to_writer_with_progress, format_kb, parse_size,
    progress_bar, write_cover_tiff,
};

pub fn gen_tiff(args: &GenTiffArgs) -> Result<()> {
    let width = usize::try_from(args.width).map_err(|_| anyhow::anyhow!("width is too large"))?;
    let height =
        usize::try_from(args.height).map_err(|_| anyhow::anyhow!("height is too large"))?;
    let size = write_cover_tiff(
        &args.output,
        width,
        height,
        args.seed,
        "RawOxide Rust cover",
    )?;
    println!("wrote {} ({})", args.output.display(), format_kb(size));
    Ok(())
}

pub fn print_write(info: &rawoxide_core::WriteInfo) {
    if info.bigtiff {
        println!(
            "warning: raw data exceeds classic TIFF limits; written as 64-bit BigTIFF \
             (DNG 1.6). tifffile reads it, but some tools (e.g. exiv2) cannot parse BigTIFF."
        );
    }
    println!("wrote {} ({})", info.path.display(), format_kb(info.size));
    println!(
        "payload {} / capacity {}",
        format_kb(info.payload_len as u64),
        format_kb(info.capacity)
    );
}

pub fn debug_file_inventory(path: &std::path::Path) -> Result<serde_json::Value> {
    crate::sidecar::file_inventory(path)
}

pub fn decode(args: &DecodeArgs) -> Result<()> {
    let progress = progress_bar(args.progress, args.no_progress, "decoding DNG", 0);
    let max_bytes = args.max_bytes.as_deref().map(parse_size).transpose()?;
    let key = args.key.as_deref().map(str::as_bytes);
    let options = DecodeOptions {
        key,
        lsb_planes: args.lsb_planes,
        max_bytes,
        split_id_field: args.split_file_metadata_id.into(),
        split_seq_field: args.split_file_metadata_seq.into(),
    };

    let parent = args.output.parent().unwrap_or_else(|| Path::new("."));
    let mut builder = tempfile::Builder::new();
    builder.prefix(".rawoxide-").suffix(".bin.tmp");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        builder.permissions(std::fs::Permissions::from_mode(0o666));
    }
    let temp = builder
        .tempfile_in(parent)
        .with_context(|| format!("cannot create temporary output in {}", parent.display()))?;
    let mut writer = std::io::BufWriter::new(temp.reopen()?);
    let outcome = if progress.is_enabled() {
        decode_files_to_writer_with_progress(
            &args.input,
            &options,
            &mut writer,
            |index, count, done, total| {
                let message = if count > 1 {
                    format!("decoding chunk {}/{}", index + 1, count)
                } else {
                    "decoding DNG".to_owned()
                };
                progress.set(done, total, &message);
            },
        )?
    } else {
        decode_files_to_writer(&args.input, &options, &mut writer)?
    };
    writer.flush().with_context(|| {
        format!(
            "cannot write temporary output for {}",
            args.output.display()
        )
    })?;
    drop(writer);
    temp.persist(&args.output)
        .with_context(|| format!("cannot replace {}", args.output.display()))?;

    for warning in &outcome.warnings {
        eprintln!("warning: {warning}");
    }
    println!(
        "recovered {} -> {}",
        format_kb(outcome.payload_len),
        args.output.display()
    );
    Ok(())
}

pub fn capacity(args: &CapacityArgs) {
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
