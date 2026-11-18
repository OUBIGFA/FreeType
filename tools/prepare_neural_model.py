"""Fetch and verify the exact WowType neural model shipped on Windows."""

import argparse
import hashlib
import json
import lzma
import os
from pathlib import Path

from make_default_data import ROOT, checked_download, sha256


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    spec = json.loads((ROOT / "third_party/manifests/neural-model.json").read_text(encoding="utf-8"))
    path = ROOT / ".local/data/nnlm.bin"
    if path.is_file() and path.stat().st_size == spec["bytes"] and sha256(path) == spec["sha256"]:
        print(f"NNLM {spec['version']}: {spec['sha256']}")
        return
    if args.check:
        parser.exit(1, "Invalid or missing nnlm.bin; run python -X utf8 tools/prepare_neural_model.py\n")
    archive = checked_download(ROOT / ".local/data-src/archives/nnlm.bin.xz", spec["url"], spec["archiveSha256"])
    with lzma.open(archive, "rb") as source:
        data = source.read(spec["bytes"] + 1)
    if len(data) != spec["bytes"] or hashlib.sha256(data).hexdigest() != spec["sha256"]:
        parser.exit(1, "Decoded nnlm.bin does not match its pinned manifest\n")
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(".bin.part")
    temporary.write_bytes(data)
    os.replace(temporary, path)
    print(f"NNLM {spec['version']}: {spec['sha256']}")


if __name__ == "__main__":
    main()
