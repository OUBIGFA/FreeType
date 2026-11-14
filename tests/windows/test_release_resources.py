"""Read the actual build outputs without installing or loading the application."""

import hashlib
import json
import os
from pathlib import Path
import subprocess
import unittest


ROOT = Path(__file__).resolve().parents[2]
APP = ROOT / "windows" / "app"


@unittest.skipUnless(os.name == "nt", "Windows resource test")
class ReleaseResourceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.config = json.loads((APP / "mygo.json").read_text(encoding="utf-8"))
        cls.stage = APP / "build" / "windows-amd64"
        installers = list((APP / "build").rglob(f"freetype-setup-{cls.config['version']}-*.exe"))
        if len(installers) != 1:
            raise RuntimeError("Run windows/build.ps1 to produce exactly one test installer")
        cls.artifacts = [cls.stage / "FreeType.exe", cls.stage / "bin" / "FreeTypeServer.exe",
                         cls.stage / "bin" / "freetype_tsf.dll",
                         cls.stage / "bin" / "x86" / "freetype_tsf.dll", installers[0]]

    def test_all_components_expose_matching_product_and_file_versions(self):
        script = (
            "[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false); "
            "$paths = $env:FREETYPE_RESOURCE_TEST_PATHS | ConvertFrom-Json; "
            "@($paths | ForEach-Object { $file = Get-Item -LiteralPath $_; "
            "[pscustomobject]@{Path=$file.FullName;Product=$file.VersionInfo.ProductName;"
            "ProductVersion=$file.VersionInfo.ProductVersion;FileVersion=$file.VersionInfo.FileVersion;"
            "NumericVersion=@($file.VersionInfo.FileMajorPart,$file.VersionInfo.FileMinorPart,"
            "$file.VersionInfo.FileBuildPart,$file.VersionInfo.FilePrivatePart);"
            "Description=$file.VersionInfo.FileDescription;Original=$file.VersionInfo.OriginalFilename}"
            " }) | ConvertTo-Json -Compress"
        )
        result = subprocess.run(
            ["powershell", "-NoProfile", "-NonInteractive", "-Command", script],
            env={**os.environ, "FREETYPE_RESOURCE_TEST_PATHS": json.dumps(list(map(str, self.artifacts)))},
            check=True, capture_output=True, text=True, encoding="utf-8", timeout=30,
            creationflags=subprocess.CREATE_NO_WINDOW,
        )
        versions = json.loads(result.stdout)
        self.assertEqual(len(versions), 5)
        for version in versions:
            with self.subTest(path=version["Path"]):
                self.assertEqual(version["Product"], "FreeType")
                self.assertEqual(version["ProductVersion"], self.config["version"])
                self.assertEqual(version["FileVersion"], self.config["version"])
                self.assertEqual(version["NumericVersion"], [int(part) for part in self.config["version"].split(".")] + [0])
                self.assertTrue(version["Description"])
                self.assertTrue(version["Original"])

    def test_installer_payload_contains_notices_and_hashed_license_files(self):
        self.assertTrue((self.stage / "THIRD-PARTY-NOTICES.txt").is_file())
        self.assertEqual((self.stage / "THIRD-PARTY-NOTICES.txt").read_bytes(),
                         (ROOT / "third_party" / "NOTICE.md").read_bytes())
        directory = self.stage / "third-party"
        manifest = json.loads((directory / "manifest.json").read_text(encoding="utf-8"))
        identifiers = {component["id"] for component in manifest["components"]}
        self.assertTrue({"go:github.com/egoist/mygo", "rust:windows", "toolchain:go",
                         "toolchain:rust", "installer:nsis", "data:public-suffix-list",
                         "data:opencc", "asset:octicons"}.issubset(identifiers))
        self.assertIn("WowType", {entry["name"] for entry in manifest["unresolved"]})
        for relative, expected in manifest["files"].items():
            with self.subTest(file=relative):
                path = directory / relative
                self.assertTrue(path.resolve().is_relative_to(directory.resolve()))
                self.assertEqual(hashlib.sha256(path.read_bytes()).hexdigest(), expected)
        self.assertNotIn(str(ROOT), json.dumps(manifest, ensure_ascii=False))
