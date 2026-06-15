# Vendored ICC profiles

## `sRGB-IEC61966-2.1.icc`

The canonical **sRGB IEC 61966-2.1** colour profile (ICC v2, monitor class
`mntr`, RGB → XYZ, 3144 bytes). Embedded by the `pdf-a` feature as the
`OutputIntent` `DestOutputProfile` so a PDF/A-2b document carries an unambiguous
definition of its DeviceRGB colours (`crate::emit::pdfa`).

This is the standard reference sRGB profile (the Hewlett-Packard / IEC
characterization, internal description `IEC sRGB`). It is the same profile
shipped as the system `sRGB Profile.icc` on macOS and is freely redistributable
as the reference characterization of the sRGB colour space. It is included here
only so the archival colour space travels inside the binary with no runtime file
dependency.

Validated end-to-end: a `--features pdf-a` document built with this profile
passes veraPDF `--flavour 2b` (see `tests/pdf_a.rs`).
