# Local calamine patch

Vendored from crates.io `calamine 0.37.0` so spanseek's legacy BIFF `.xls`
reader can keep formula text readable.

The local change in `src/xls.rs` masks `PtgRef3d` / `PtgArea3d` BIFF8
`ColRelU` fields to their 14-bit column value before formatting. Upstream
0.37.0 shifts the whole 16-bit word, turning a valid reference such as
`明细!D2:D3` into garbage such as `明细!$BTRP$2:$BTRP$3`.

The upstream crate is MIT licensed; `LICENSE-MIT.md` is kept alongside the
source. Re-check this patch before upgrading calamine.
The vendored manifest disables upstream doctests. Their examples assume the
unpacked crate's test fixtures and are not part of spanseek's build; unit tests
for the reader still run with the workspace test suite.