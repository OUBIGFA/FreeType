"""Build provenance must describe reviewed working inputs without staging user files."""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
SNAPSHOT = ROOT / "tools" / "capture_build_source.py"


class BuildSourceTests(unittest.TestCase):
    def setUp(self):
        scratch = Path(os.environ.get("FREETYPE_TEST_TEMP", ROOT / "_temp" / "repository-tests"))
        scratch.mkdir(parents=True, exist_ok=True)
        self.repo = Path(tempfile.mkdtemp(prefix="source-", dir=scratch)).resolve()
        self.sequence = 0
        self.git("init", "-q")
        self.git("config", "core.autocrlf", "false")
        self.git("config", "user.name", "Fixture Author")
        self.git("config", "user.email", "fixture@example.invalid")
        self.write(".gitignore", "**/_temp/\n")
        self.write("core/input.rs", "original source\n")
        self.git("add", ".")
        self.git("commit", "-q", "-m", "fixture")

    def git(self, *args):
        return subprocess.check_output(["git", "-C", str(self.repo), *args], encoding="utf-8").strip()

    def write(self, name, text):
        path = self.repo / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8", newline="\n")

    def capture(self, *args, index=None):
        self.sequence += 1
        index = index or self.repo / "_temp" / f"index-{self.sequence}"
        return subprocess.run(
            [sys.executable, "-X", "utf8", str(SNAPSHOT), "--index", str(index), *args],
            cwd=self.repo, capture_output=True, text=True, encoding="utf-8",
        )

    def test_captures_working_sources_and_staged_additions_without_touching_index(self):
        self.write("core/input.rs", "reviewed working source\n")
        self.write("windows/new.rs", "new staged source\n")
        self.git("add", "windows/new.rs")
        self.write("skills/local.txt", "unrelated user work\n")
        index_before = (self.repo / ".git" / "index").read_bytes()
        result = self.capture()
        self.assertEqual(result.returncode, 0, result.stderr)
        snapshot = json.loads(result.stdout)
        self.assertEqual(snapshot["baseCommit"], self.git("rev-parse", "HEAD"))
        self.assertEqual(self.git("show", snapshot["tree"] + ":core/input.rs"), "reviewed working source")
        self.assertEqual(self.git("show", snapshot["tree"] + ":windows/new.rs"), "new staged source")
        self.assertNotIn("skills/local.txt", self.git("ls-tree", "-r", "--name-only", snapshot["tree"]))
        self.assertEqual((self.repo / ".git" / "index").read_bytes(), index_before)
        self.assertTrue((self.repo / "skills/local.txt").is_file())

    def test_untracked_build_input_is_rejected_instead_of_omitted(self):
        self.write("windows/app/unstaged.go", "package main\n")
        result = self.capture()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("windows/app/unstaged.go", result.stderr)
        self.assertEqual(self.git("diff", "--cached", "--name-only"), "")

    def test_changed_source_fails_expected_snapshot_check(self):
        before = self.capture()
        self.assertEqual(before.returncode, 0, before.stderr)
        snapshot = json.loads(before.stdout)
        self.write("core/input.rs", "changed while building\n")
        after = self.capture("--expect-tree", snapshot["tree"], "--expect-commit", snapshot["baseCommit"])
        self.assertNotEqual(after.returncode, 0)
        self.assertIn("源码", after.stderr)

    def test_unchanged_source_passes_expected_snapshot_check(self):
        before = self.capture()
        self.assertEqual(before.returncode, 0, before.stderr)
        snapshot = json.loads(before.stdout)
        after = self.capture("--expect-tree", snapshot["tree"], "--expect-commit", snapshot["baseCommit"])
        self.assertEqual(after.returncode, 0, after.stderr)
        self.assertEqual(json.loads(after.stdout), snapshot)

    def test_real_index_cannot_be_used_as_scratch(self):
        index = self.repo / ".git" / "index"
        before = index.read_bytes()
        result = self.capture(index=index)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("_temp", result.stderr)
        self.assertEqual(index.read_bytes(), before)


if __name__ == "__main__":
    unittest.main()
