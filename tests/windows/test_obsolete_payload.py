"""Recycle only unchanged, manifest-owned payload files after a successful upgrade."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


@unittest.skipUnless(os.name == "nt", "Windows recycle-bin test")
class ObsoletePayloadTests(unittest.TestCase):
    def test_manifest_owned_sources_are_recycled_and_modified_or_unlisted_files_survive(self):
        parent = Path(os.environ.get("FREETYPE_TEST_TEMP", ROOT / "_temp/obsolete-payload"))
        parent.mkdir(parents=True, exist_ok=True)
        scratch = Path(tempfile.mkdtemp(dir=parent)).resolve()
        installed = scratch / "installed"
        previous = scratch / "previous"
        previous.mkdir()
        files = {"data/default@1/source/original.zip": b"source", "data/default@1/source/edited.zip": b"original"}
        for name, content in files.items():
            target = installed / "third-party" / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(content)
        edited = installed / "third-party/data/default@1/source/edited.zip"
        edited.write_bytes(b"user edit")
        unrelated = installed / "third-party/data/default@1/source/personal.txt"
        unrelated.write_text("keep", encoding="utf-8")
        (installed / "data").mkdir()
        (installed / "data/lexicon.tsv").write_bytes(b"dictionary")
        (previous / "third-party.json").write_text(json.dumps({"components": [{"sourceFiles": list(files)}],
            "files": {name: hashlib.sha256(content).hexdigest() for name, content in files.items()}}), encoding="utf-8")
        (previous / "data.json").write_text(json.dumps({"files": {"lexicon.tsv": {
            "sha256": hashlib.sha256(b"dictionary").hexdigest()}}}), encoding="utf-8")
        script = ROOT / "windows/app/cleanup-install.ps1"
        self.assertTrue(script.is_file(), "Upgrade cleanup is missing")
        result = subprocess.run(["powershell", "-NoProfile", "-NonInteractive", "-File", str(script)],
            env={**os.environ, "FREETYPE_INSTALL_DIR": str(installed), "FREETYPE_PREVIOUS_MANIFESTS": str(previous)},
            capture_output=True, timeout=30, creationflags=subprocess.CREATE_NO_WINDOW)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertFalse((installed / "third-party/data/default@1/source/original.zip").exists())
        self.assertFalse((installed / "data/lexicon.tsv").exists())
        self.assertEqual(edited.read_bytes(), b"user edit")
        self.assertEqual(unrelated.read_text(encoding="utf-8"), "keep")
