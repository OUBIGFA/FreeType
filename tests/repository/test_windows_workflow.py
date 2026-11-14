"""Guard workflow configuration; the hosted run validates the actual Windows build."""

import os
from pathlib import Path
import re
import subprocess
import unittest


ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github/workflows/windows-build.yml"
PREPARE = ROOT / "windows/prepare-ci.ps1"


class WindowsWorkflowTests(unittest.TestCase):
    def setUp(self):
        self.assertTrue(WORKFLOW.is_file(), "Windows installer workflow is missing")
        self.assertTrue(PREPARE.is_file(), "CI input preparation is missing")
        self.workflow = WORKFLOW.read_text(encoding="utf-8")

    def test_build_is_explicit_and_read_only(self):
        self.assertIn("workflow_dispatch:", self.workflow)
        self.assertIn("- '0.1.0'", self.workflow)
        self.assertIn("- 'freetype-*'", self.workflow)
        self.assertIn("runs-on: windows-2025", self.workflow)
        self.assertIn("contents: read", self.workflow)
        self.assertIn("persist-credentials: false", self.workflow)
        self.assertNotIn("contents: write", self.workflow)
        self.assertNotIn("pull_request_target", self.workflow)
        self.assertNotIn("gh release", self.workflow)

    def test_actions_and_toolchains_are_pinned(self):
        actions = re.findall(r"uses: ([^\s@]+)@([^\s]+)", self.workflow)
        self.assertGreaterEqual(len(actions), 4)
        for action, revision in actions:
            with self.subTest(action=action):
                self.assertRegex(revision, r"^[0-9a-f]{40}$")
        self.assertIn("go-version: '1.27.4'", self.workflow)
        self.assertIn("python-version: '3.11.9'", self.workflow)
        prepare = PREPARE.read_text(encoding="utf-8")
        self.assertIn("1.99.1", prepare)
        self.assertIn("Pillow==12.2.0", prepare)
        self.assertIn("12bbd48dfd2c86f55419bf92b196dd3aedd6983116b5ce10c85d27a5a76c2baa", prepare)

    def test_existing_builder_and_artifact_whitelist_are_preserved(self):
        self.assertIn("tools/make_default_data.py", self.workflow)
        self.assertIn("windows/build.ps1", self.workflow)
        self.assertIn("ReleaseTag", self.workflow)
        self.assertNotIn("SkipTests", self.workflow)
        self.assertIn("FreeType-${{ steps.version.outputs.version }}-Setup.exe", self.workflow)
        self.assertIn("FreeType-${{ steps.version.outputs.version }}-Setup.exe.sha256", self.workflow)
        self.assertIn("build-info.json", self.workflow)
        self.assertIn("if-no-files-found: error", self.workflow)
        self.assertNotIn("output/**", self.workflow)

    @unittest.skipUnless(os.name == "nt", "PowerShell parser is required")
    def test_prepare_script_parses_without_running_installers(self):
        command = (
            "$tokens=$null; $errors=$null; "
            "[System.Management.Automation.Language.Parser]::ParseFile($env:FT_CI_SCRIPT,"
            "[ref]$tokens,[ref]$errors) | Out-Null; "
            "if ($errors.Count) { $errors | Out-String | Write-Output; exit 1 }"
        )
        result = subprocess.run(
            ["powershell", "-NoProfile", "-NonInteractive", "-Command", command],
            env={**os.environ, "FT_CI_SCRIPT": str(PREPARE)},
            capture_output=True, timeout=20,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
