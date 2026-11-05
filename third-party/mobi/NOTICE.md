# Vendored MOBI parsing components

This directory vendors `mobi` 0.8.0 (MIT, https://github.com/vv9k/mobi-rs)
with local fixes for record-range bounds, PalmDOC LZ77 decoding, trailing
multibyte/TBS regions, and bounded decompression behavior.

`src/compression/huffcdic.rs` is adapted from `kindling-mobi` 0.45.6 (MIT,
https://github.com/ciscoriordan/kindling). Its original license text is in
`LICENSE-kindling`.

The upstream license for `mobi` is in `LICENSE`.
The HUFF/CDIC regression fixtures under 	ests/fixtures are copied from
kindling-mobi 0.45.6 and remain under its MIT license.
