"""Keep license copying lossless and reject incomplete source disclosures."""

import importlib
import io
import json
import os
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools"))


class ThirdPartyNoticeTests(unittest.TestCase):
    def setUp(self):
        self.assertTrue((ROOT / "tools" / "make_third_party_notices.py").is_file(),
                        "The notice collector has not been implemented")
        self.collector = importlib.import_module("make_third_party_notices")
        scratch = Path(os.environ.get("FREETYPE_TEST_TEMP", ROOT / "_temp" / "notice-tests"))
        scratch.mkdir(parents=True, exist_ok=True)
        self.directory = Path(tempfile.mkdtemp(prefix="notices-", dir=scratch)).resolve()
        self.source = self.directory / "upstream source"
        self.source.mkdir()
        self.output = self.directory / "output"

    def test_preserves_nested_notices_without_copying_source_named_license(self):
        content = "Copyright 原作者\r\nPermission text.\r\n".encode("utf-8")
        (self.source / "LICENSE").write_bytes(content)
        nested = self.source / "vendor"
        nested.mkdir()
        (nested / "NOTICE.md").write_text("Extra attribution", encoding="utf-8")
        (self.source / "license.go").write_text("package example", encoding="utf-8")
        component = self.collector.copy_component(self.output, "go:example.org/sample", "v1.2.3", self.source)
        self.assertEqual(len(component["licenseFiles"]), 2)
        copied = [self.output / name for name in component["licenseFiles"]]
        self.assertIn(content, [path.read_bytes() for path in copied])
        self.assertEqual({path.name for path in copied}, {"LICENSE", "NOTICE.md"})
        self.assertNotIn(str(self.directory), json.dumps(component))

    def test_missing_license_stops_collection(self):
        with self.assertRaisesRegex(ValueError, "license|许可"):
            self.collector.copy_component(self.output, "go:example.org/missing", "v1.0.0", self.source)

    def test_mpl_notice_requires_and_preserves_corresponding_source(self):
        (self.source / "LICENSE").write_text("Mozilla Public License Version 2.0", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "source|源码"):
            self.collector.copy_component(self.output, "go:example.org/mpl", "v1.0.0", self.source)
        archive = self.directory / "v1.0.0.zip"
        archive.write_bytes(b"exact upstream source archive")
        component = self.collector.copy_component(self.output, "go:example.org/mpl", "v1.0.0",
                                                  self.source, source_archive=archive)
        self.assertEqual(len(component["sourceFiles"]), 1)
        self.assertEqual((self.output / component["sourceFiles"][0]).read_bytes(), archive.read_bytes())

    def test_json_stream_reads_multiple_module_records(self):
        values = list(self.collector.json_stream('  {"Module":{"Path":"first"}}\n{}\n{"Module":{"Path":"second"}}'))
        self.assertEqual(values, [{"Module": {"Path": "first"}}, {}, {"Module": {"Path": "second"}}])

    def test_explicit_mpl_text_is_not_rejected_for_its_filename(self):
        (self.source / "MPL-2.0.txt").write_text("Mozilla Public License Version 2.0", encoding="utf-8")
        archive = self.directory / "public_suffix_list.dat"
        archive.write_bytes(b"// Public Suffix List source")
        component = self.collector.copy_component(self.output, "data:public-suffix-list", "revision",
                                                  self.source, ["MPL-2.0.txt"], archive)
        self.assertEqual(len(component["licenseFiles"]), 1)
        self.assertEqual(len(component["sourceFiles"]), 1)

    def test_nsis_notices_come_from_source_without_an_installed_compiler(self):
        self.assertTrue(hasattr(self.collector, "collect_nsis"), "NSIS source collection is missing")
        archive = self.directory / "nsis-3.13-src.tar.bz2"
        contents = {"COPYING": b"NSIS licenses\r\n", "Contrib/Modern UI 2/License.txt": b"Modern UI license\r\n"}
        with tarfile.open(archive, "w:bz2") as snapshot:
            for name, content in contents.items():
                member = tarfile.TarInfo("nsis-3.13-src/" + name)
                member.size = len(content)
                snapshot.addfile(member, io.BytesIO(content))
        component = self.collector.collect_nsis(self.output, archive, "3.13")
        self.assertEqual(len(component["licenseFiles"]), 2)
        for relative, content in contents.items():
            self.assertEqual((self.output / "installer/nsis@3.13" / relative).read_bytes(), content)
        self.assertEqual((self.output / component["sourceFiles"][0]).read_bytes(), archive.read_bytes())

    def test_go_runtime_preserves_vendored_notices_but_not_compiler_dependencies(self):
        self.assertTrue(hasattr(self.collector, "collect_go_runtime"), "Go runtime collection is missing")
        (self.source / "LICENSE").write_bytes(b"Go license")
        runtime = self.source / "src/vendor/golang.org/x/crypto"
        runtime.mkdir(parents=True)
        (runtime / "LICENSE").write_bytes(b"Vendored crypto license")
        compiler = self.source / "src/cmd/vendor/example"
        compiler.mkdir(parents=True)
        (compiler / "LICENSE").write_bytes(b"Build-only license")
        component = self.collector.collect_go_runtime(self.output, self.source, "go1.27.4")
        self.assertEqual(len(component["licenseFiles"]), 2)
        copied = [self.output / name for name in component["licenseFiles"]]
        self.assertEqual({path.read_bytes() for path in copied}, {b"Go license", b"Vendored crypto license"})
