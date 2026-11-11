# On-disk format

This document describes what RawOxide writes and reads: the payload frame, the
LSB mapping into raw samples, the surrounding TIFF/DNG structure and the
metadata used for split sets. It is the reference for anyone implementing a
compatible reader or writer.

## Payload frame

The payload is wrapped in a fixed 18-byte header before it is embedded:

| offset | size | field |
|---|---|---|
| 0 | 4 | magic `DNGS` |
| 4 | 1 | frame version, currently `1` |
| 5 | 1 | flags; bit 0 means the frame is keyed |
| 6 | 8 | plaintext payload length, big-endian `u64` |
| 14 | 4 | CRC32 of the plaintext payload, big-endian `u32` |
| 18 | N | payload bytes |

The CRC always covers the plaintext payload. With a key, both the header and the
payload are XOR-masked with a SHA-256 counter keystream:

```text
keystream[i] = SHA-256(key || counter_be_u64)[i mod 32]
counter      = byte_offset / 32
```

The header is masked from keystream offset 0. Each payload chunk is masked from
its own chunk offset; chunks are 4 MiB, so a stream is split at 4 MiB
boundaries. Decode unmasks the header, validates the magic and version, checks
the declared length against the input, unmasks the body and verifies the CRC.
An empty key behaves like no key.

An empty payload is valid and produces an 18-byte frame consisting only of the
header.

## LSB mapping

The frame is treated as a bit stream. Bit `i` (most-significant bit first
within each byte: bit 0 is `0x80`, bit 7 is `0x01`) is written to plane
`i % planes` of sample `i / planes`:

```text
sample_index = bit_index / planes
plane        = bit_index % planes
sample      ^= (bit << plane)   // only the selected bit changes
```

Samples are visited in raster order across all raw frames. For `linear` output
there are three samples per pixel (R, G, B); for `cfa` there is one. A sample is
a `u8` for 8-bit output and a `u16` container for 10/12/14/16-bit output; only
planes below the sample depth are meaningful.

Capacity for one frame is:

```text
capacity = samples × planes / 8 − 18
```

The header cost is charged once for the whole payload, not once per frame.

## DNG container

RawOxide writes a classic TIFF or BigTIFF DNG with the following structure:

```text
IFD0
├── preview image (JPEG, YCbCr 4:2:0)
├── SubIFDs (1–8)
│   └── raw page: tiled LinearRaw or CFA
├── EXIF IFD (optional)
└── GPS IFD (optional)
```

- The raw pages are tiled. Tile geometry comes from the selected camera
  profile, defaulting to 128×96.
- Sample depths 8, 10, 12, 14 and 16 are supported. Packed 10/12/14-bit samples
  are stored without padding; 8- and 16-bit samples use `u8`/`u16` containers.
- Raw tiles are uncompressed by default. `adobe_deflate` uses Deflate with
  horizontal predictor 2 and is valid for 8- and 16-bit output only.
- The preview is a single baseline JPEG strip at quality 75, sized according to
  the profile's fidelity settings.
- DNG colour tags (`AsShotNeutral`, `BaselineExposure`, `NoiseProfile`,
  calibration matrices, `ActiveArea`, `DefaultUserCrop`, `CFAPattern`, opcode
  lists and Adobe profile tags) are emitted according to the selected camera
  profile.
- EXIF carries the camera/lens/exposure block, timestamps, optional maker note
  and split markers. GPS is emitted only for profiles that request it.
- `NewRawImageDigest` and `RawDataUniqueID` are written for profiles that ask
  for raw digests; both include the profile's gain-map fingerprint where
  applicable.

Classic TIFF is used while offsets fit. When the raw data would exceed classic
limits, or when `--bigtiff` is given, RawOxide writes a BigTIFF with 64-bit
offsets and `IFD8` pointers.

## Split sets

A split payload is written as a set of DNG chunks named with a four-digit
sequence before the extension:

```text
out.dng  ->  out0001.dng, out0002.dng, ...
```

The set is described by two metadata fields selected on the command line.

Shared set ID (`--split-file-metadata-id`):

| value | location | meaning |
|---|---|---|
| `ImageUniqueID` | EXIF tag 42016 | UUID string shared by every chunk |
| `ImageDescription` | IFD0 tag 270 | shared string |
| `none` | — | no ID written |

Chunk sequence (`--split-file-metadata-seq`):

| value | location | meaning |
|---|---|---|
| `ImageNumber` | EXIF tag 37393 | scalar chunk number, no total |
| `PageNumber` | IFD0 tag 297 | `(current, total)` |
| `ImageDescription` | IFD0 tag 270 | `"NNNN/MMMM"` |
| `none` | — | no sequence written |

The ID and sequence fields must differ; selecting `ImageDescription` for both is
an error. `PageNumber` is fail-closed: decode rejects a set with a missing or
extra chunk. `ImageNumber` cannot detect gaps. Decode also accepts chunks in
filename order when no usable sequence field is present.

A set ID must be non-empty ASCII. Four-digit names limit a set to 9999 chunks.

## Determinism

Random draw streams are explicit and named:

| stream | drives |
|---|---|
| cover | synthetic raw samples and the synthetic/noise preview fallback |
| metadata | EXIF fields and serials; initialises separate location and maker-note streams |
| location | GPS sampling, shared across a split set when present |
| split chunk | per-chunk cover and metadata draws |

The cover and metadata streams are seeded from `--seed`. With `--no-randomize`
metadata uses seed 0, and split chunks use their zero-based sequence offset.
Without either flag, unseeded split chunks draw fresh entropy per chunk. GPS
locations for a split set are sampled once and shared.

The selected cover and metadata seeds, the resolved image time and the split
markers are recorded in the debug sidecar so a run can be reproduced.

## Reader scope

Decode understands the layouts RawOxide writes: tiled or stripped raw pages,
uncompressed or Deflate data, predictor 2, packed 10/12/14-bit samples, multiple
frames, classic TIFF and BigTIFF. Other DNG writers may use different
compressions, sample layouts or EXIF pointer conventions; those are outside the
current reader scope.

The reader bounds IFD offsets and counts against the file size, limits declared
payloads with `--max-bytes` before allocation, and validates the frame CRC
before publishing output.
