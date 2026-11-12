"""Record Cargo.lock packages so the repository check notices lock changes.

Run from the repository root after changing Cargo.lock. The output lists every
locked package with its version and registry checksum.
"""

from pathlib import Path
import hashlib
import json
import tomllib


def main():
    # Git stores text with LF line endings (.gitattributes), so hash the same bytes.
    lock = Path("Cargo.lock").read_bytes().replace(b"\r\n", b"\n")
    packages = [
        {"name": item["name"], "version": item["version"], "checksum": item.get("checksum", "")}
        for item in tomllib.loads(lock.decode("utf-8"))["package"]
    ]
    packages.sort(key=lambda item: (item["name"], item["version"]))
    record = {"lockSha256": hashlib.sha256(lock).hexdigest(), "packages": packages}
    target = Path("third_party/manifests/cargo-lock.json")
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8", newline="\n")
    print(f"Recorded {len(packages)} packages in {target.as_posix()}")


if __name__ == "__main__":
    main()
