//! Remaining command implementations and debug inventory.

use super::{
    CapacityArgs, Context, DecodeArgs, DecodeOptions, GenTiffArgs, Result, capacity_bytes,
    decode_files, format_kb, parse_size, progress_bar, write_cover_tiff,
};

pub fn gen_tiff(args: &GenTiffArgs) -> Result<()> {
    let width = usize::try_from(args.width).map_err(|_| anyhow::anyhow!("width is too large"))?;
    let height =
        usize::try_from(args.height).map_err(|_| anyhow::anyhow!("height is too large"))?;
    let size = write_cover_tiff(&args.output, width, height, args.seed, "RawHide Rust cover")?;
    println!("wrote {} ({})", args.output.display(), format_kb(size));
    Ok(())
}

pub fn print_write(info: &rawhide_core::WriteInfo) {
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

pub fn debug_file_inventory(path: &std::path::Path) -> Result<serde_json::Value> {
    let reader = rawhide_core::DngReader::open(path)?;
    let mut ifds = vec![ifd_inventory("ifd0", reader.ifd0()?)];
    for (index, ifd) in reader.raw_ifds()?.iter().enumerate() {
        ifds.push(ifd_inventory(&format!("raw{index}"), ifd));
    }
    if let Some(exif) = reader.exif_ifd()? {
        ifds.push(ifd_inventory("exif", &exif));
    }
    if let Some(gps) = reader.gps_ifd()? {
        ifds.push(ifd_inventory("gps", &gps));
    }
    Ok(serde_json::json!({
        "bytes": std::fs::metadata(path)?.len(),
        "ifds": ifds,
    }))
}

fn ifd_inventory(name: &str, ifd: &tiff_reader::Ifd) -> serde_json::Value {
    let tags: Vec<_> = ifd.tags().iter().map(tag_inventory).collect();
    serde_json::json!({"name": name, "tag_count": tags.len(), "tags": tags})
}

fn tag_inventory(tag: &tiff_reader::Tag) -> serde_json::Value {
    let value = if tag.count as usize <= 64 {
        match &tag.value {
            tiff_reader::TagValue::Ascii(text) => serde_json::Value::String(text.clone()),
            tiff_reader::TagValue::Byte(values) | tiff_reader::TagValue::Undefined(values) => {
                serde_json::json!(values)
            }
            tiff_reader::TagValue::Short(values) => serde_json::json!(values),
            tiff_reader::TagValue::Long(values) => serde_json::json!(values),
            tiff_reader::TagValue::Rational(values) => serde_json::json!(values),
            tiff_reader::TagValue::SByte(values) => serde_json::json!(values),
            tiff_reader::TagValue::SShort(values) => serde_json::json!(values),
            tiff_reader::TagValue::SLong(values) => serde_json::json!(values),
            tiff_reader::TagValue::SRational(values) => serde_json::json!(values),
            tiff_reader::TagValue::Float(values) => serde_json::json!(values),
            tiff_reader::TagValue::Double(values) => serde_json::json!(values),
            tiff_reader::TagValue::Long8(values) => serde_json::json!(values),
            tiff_reader::TagValue::SLong8(values) => serde_json::json!(values),
        }
    } else {
        let encoded = tag.value.encode(tiff_reader::ByteOrder::LittleEndian);
        serde_json::json!({
            "bytes": tag.value.encoded_len(),
            "crc32": format!("{:08x}", crc32fast::hash(&encoded)),
        })
    };
    serde_json::json!({
        "tag": tag.code,
        "type": format!("{:?}", tag.tag_type),
        "count": tag.count,
        "value": value,
    })
}

pub fn decode(args: &DecodeArgs) -> Result<()> {
    let _progress = progress_bar(args.progress, args.no_progress, "decoding DNG");
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
