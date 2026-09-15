"""Verify vendored files against pinned upstream digests; no third-party packages."""
import hashlib
import pathlib
import re

root = pathlib.Path(__file__).resolve().parents[1] / "assets" / "original-crtsim"
entries = re.findall(r"^([0-9a-f]{64}) (\S+)$", (root / "SOURCES.md").read_text(), re.M)
assert len(entries) == 11, "missing asset digests"
for digest, name in entries:
    actual = hashlib.sha256((root / name).read_bytes()).hexdigest()
    assert actual == digest, f"{name}: digest mismatch"
print(f"Verified {len(entries)} original upstream files")
