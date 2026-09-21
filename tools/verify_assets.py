"""Verify vendored files against pinned upstream digests; no third-party packages."""
import hashlib
import pathlib
import re
import struct

root = pathlib.Path(__file__).resolve().parents[1] / "assets" / "original-crtsim"
entries = re.findall(r"^([0-9a-f]{64}) (\S+)$", (root / "SOURCES.md").read_text(), re.M)
assert len(entries) == 11, "missing asset digests"
for digest, name in entries:
    actual = hashlib.sha256((root / name).read_bytes()).hexdigest()
    assert actual == digest, f"{name}: digest mismatch"
print(f"Verified {len(entries)} original upstream files")

root = root.parent / "nes-luts"
entries = re.findall(r"^([0-9a-f]{64})  (.+\.png)$", (root / "SHA256SUMS").read_text(), re.M)
assert len(entries) == 38, "missing NES LUT asset digests"
assert len({name for _, name in entries}) == 38, "duplicate NES LUT manifest entry"
assert {name for _, name in entries} == {p.name for p in root.glob("*.png")}, "NES LUT manifest mismatch"
for digest, name in entries:
    data = (root / name).read_bytes()
    assert hashlib.sha256(data).hexdigest() == digest, f"{name}: digest mismatch"
    assert data[:8] == b"\x89PNG\r\n\x1a\n" and data[12:16] == b"IHDR", f"{name}: not a PNG"
    assert struct.unpack(">II", data[16:24]) == (4096, 64), f"{name}: unexpected LUT dimensions"
    assert data[24:26] == bytes([8, 2]), f"{name}: expected 8-bit RGB samples"
print(f"Verified {len(entries)} original NES LUT PNGs (64³ samples each)")
