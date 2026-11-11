//! Criterion benchmarks for the LSB codec.
//!
//! Run with `cargo bench -p rawoxide-core` or `just bench`.

use criterion::{BatchSize, Criterion, Throughput, criterion_group, criterion_main};
use rawoxide_core::{LsbCodec, PayloadFrame};

const WIDTH: usize = 2048;
const HEIGHT: usize = 1536;
const CHANNELS: usize = 3;

fn payload() -> Vec<u8> {
    // Deterministic, incompressible-enough 1 MiB payload.
    (0_u32..(1 << 20))
        .map(|index| (index.wrapping_mul(2_654_435_761) >> 13) as u8)
        .collect()
}

fn bench_codec(c: &mut Criterion) {
    let frame = PayloadFrame::pack(&payload(), None);
    let cover = vec![0_u16; WIDTH * HEIGHT * CHANNELS];
    let codec = LsbCodec::new(1).unwrap();

    let mut group = c.benchmark_group("lsb_codec");
    group.throughput(Throughput::Bytes(frame.len() as u64));
    group.bench_function("embed_1MiB_1plane_u16", |bencher| {
        bencher.iter_batched(
            || cover.clone(),
            |mut target| codec.embed(&mut target, &frame).unwrap(),
            BatchSize::LargeInput,
        );
    });

    let mut embedded = cover.clone();
    codec.embed(&mut embedded, &frame).unwrap();
    group.bench_function("extract_1MiB_1plane_u16", |bencher| {
        bencher.iter(|| codec.extract(&embedded, frame.len()).unwrap());
    });
    group.finish();
}

criterion_group!(benches, bench_codec);
criterion_main!(benches);
