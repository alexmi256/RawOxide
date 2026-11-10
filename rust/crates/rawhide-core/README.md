# rawhide-core

Library for RawHide: payload framing, the LSB codec, TIFF/DNG reading and
writing, embedded camera profiles, seeded metadata, GPS sampling and preview
generation.

The crate has no CLI and no global network policy. Its only network access is
the optional Wikimedia Commons preview source (`PreviewSource::Random`); core
otherwise performs no I/O beyond files the caller passes in.

See the workspace [README](../../README.md),
[port plan](../../docs/port-plan.md) and
[porting matrix](../../docs/porting-matrix.md).
