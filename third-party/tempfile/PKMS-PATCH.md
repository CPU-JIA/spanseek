# PKMS Windows long-path patch

Source: crates.io `tempfile` 3.27.0, https://crates.io/crates/tempfile/3.27.0
Crate SHA-256: `32497e9a4c7b38532efcdebeef879707aa9f794296a4f0244f6f69e9bc8574bd`
Upstream: https://github.com/Stebalien/tempfile
Original MIT / Apache-2.0 license files are preserved.

Windows `keep` and `persist` called `SetFileAttributesW` / `MoveFileExW`
with ordinary paths. With LongPathsEnabled=0, real Lance local writes failed
after Rust std had successfully created the temporary file.

The local patch resolves ordinary paths with `std::path::absolute`, adds the
Windows extended-length prefix (including UNC), preserves verbatim/device paths,
preserves the NT `\??\` prefix before absolute resolution, and rejects embedded
NUL before calling Win32. Both source and destination are
converted. No registry change or machine-wide configuration is required.
The patch raises this copy's declared MSRV from 1.63 to 1.79 for `absolute`;
PKMS itself requires Rust 1.98. `Cargo.toml.orig` remains the upstream provenance.

Regression tests cover path encoding, long Unicode paths, persist overwrite,
noclobber failure recovery, and keep. UNC encoding is tested without claiming
access to a real network share. Remove this patch when a verified upstream
release provides equivalent support, rerunning the real Lance long-path test.
