# Bundled NES LUTs

The 38 PNGs in this directory are preserved byte-for-byte from the user's
`luts_png.zip` attachment. `SHA256SUMS` records SHA-256 digests of those original
files, including their original names. Run `python tools/verify_assets.py` to
verify their integrity and dimensions before packaging.

Upstream collection:
https://github.com/mamedev/mame-goodies/tree/b0555f8d1b8b8686a38f7aa8052232670e48a825/bgfx/lut/nes

Discovery/update discussion:
https://www.reddit.com/r/emulation/comments/1oopf1i/updated_nes_luts_for_mame/

The pinned upstream commit identifies the NES LUT update. The attachment is the
vendored byte source; these digests attest to its integrity, not an independent
download comparison against every upstream file. Upstream's root README and
disclaimer are preserved verbatim in `UPSTREAM_README.md`. See the repository
`THIRD_PARTY_NOTICES.md` and application credits for creator attribution.

## Layout and decoding

All originals are RGB PNGs of 4096 × 64 pixels, encoding 64 × 64 × 64 samples.
Red varies across each 64-pixel tile, green down the rows, and blue across the
64 tiles: `pixel_x = red + 64 * blue`, `pixel_y = green`.

This ordering is verified against MAME's `apply_lut` implementation:
https://github.com/mamedev/mame/blob/4bfae9c5364143474d2cd4c21f24443716c4d74b/hlsl/color.fx

`crtsim-core/src/nes_luts.rs` decodes the embedded PNG on demand into the existing
64³ `Lut` representation, in red-fastest `.cube` order. It preserves every sample
without resampling or gamma/profile conversion. The original compressed PNGs
are embedded in the application so the gallery works offline. The converted
`.cube` archive is unnecessary for the bundled gallery.
