"""Exercise the default dictionary builder with real pinned source archives."""

import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
BUILDER = ROOT / "tools" / "make_default_data.py"
CONVERTER = ROOT / "tools" / "rime2tsv.py"


class DefaultDataTests(unittest.TestCase):
    def setUp(self):
        scratch = Path(os.environ.get("FREETYPE_TEST_TEMP", ROOT / "_temp" / "data-tests"))
        scratch.mkdir(parents=True, exist_ok=True)
        self.scratch = Path(tempfile.mkdtemp(prefix="case-", dir=scratch))
        self.sources = self.scratch / "sources"
        self.archives = self.sources / "archives"
        self.archives.mkdir(parents=True)
        self.output = self.scratch / "output"
        self.model = self.scratch / "words.lm"
        self.model.write_bytes(b"fixture-language-model")
        self.model_license = self.archives / "CC-BY-SA-4.0.txt"
        self.model_license.write_text("fixture CC BY-SA 4.0", encoding="utf-8")
        self.frost_files = {
            "rime_frost.dict.yaml": "---\nname: frost\nimport_tables:\n"
            "  - cn_dicts/base\n  - cn_dicts_cell/extthot\n"
            "  - cn_dicts/41448\n  - cn_dicts/GB18030-2022\n"
            "  - cn_dicts/corrections\n...\n",
            "cn_dicts/base.dict.yaml": "...\n你好\tni hao\t100\n"
            "常用\tchang yong\t50\n吕布\tlv bu\t10\n",
            "cn_dicts_cell/exthot.dict.yaml": "...\n重跑\tchong pao\t0\n",
            "cn_dicts/41448.dict.yaml": "...\n㕈\thou\t0\n",
            "cn_dicts/GB18030-2022.dict.yaml": "...\n𠮷\tji\t0\n",
            "cn_dicts/corrections.dict.yaml": "...\n角色\tjiao se\t0\n",
            "en_dicts/en.dict.yaml": "...\nhello\thello\t100\n",
            "en_dicts/en_ext.dict.yaml": "...\nFreeType\tfreetype\t1\n",
            "LICENSE": "fixture GPL-3.0",
        }
        self.wanxiang_files = {
            "dicts/zi.dict.yaml": "...\n吕\tlǚ\t5\n",
            "dicts/jichu.dict.yaml": "...\n你好\tnǐ hǎo\t900\n"
            "吕布\tlǚ bù\t800\n向量检索\txiàng liàng jiǎn suǒ\t20\n",
            "dicts/duoyin.dict.yaml": "...\n重跑\tzhòng pǎo\t4\n",
            "dicts/diming.dict.yaml": "...\n海淀\thǎi diàn\t8\n",
            "dicts/mingren.dict.yaml": "...\n苏轼\tsū shì\t6\n",
            "dicts/yiren.dict.yaml": "...\n周杰伦\tzhōu jié lún\t6\n",
            "LICENSE": "fixture CC BY 4.0",
            "LICENSE-SCOPE.md": "fixture data-only license scope",
            "NOTICE.md": "fixture third-party notice",
            "tools/not-distributable.py": "raise RuntimeError('must not execute')",
        }
        self.manifest = {
            "version": "2026-11-13.1",
            "frequencyTotal": 0,
            "minimumEntries": 1,
            "sources": [
                {
                    "name": "rime-frost",
                    "repository": "https://github.com/gaboolic/rime-frost",
                    "revision": "1" * 40,
                    "license": "GPL-3.0",
                    "dictionary": "rime_frost.dict.yaml",
                    "skipImports": ["cn_dicts/41448", "cn_dicts/GB18030-2022", "cn_dicts/corrections"],
                    "importAliases": {"cn_dicts_cell/extthot": "cn_dicts_cell/exthot"},
                    "extraChars": ["cn_dicts/41448.dict.yaml", "cn_dicts/GB18030-2022.dict.yaml"],
                    "english": ["en_dicts/en.dict.yaml", "en_dicts/en_ext.dict.yaml"],
                },
                {
                    "name": "rime-wanxiang-data",
                    "repository": "https://github.com/amzxyz/rime-wanxiang-data",
                    "revision": "2" * 40,
                    "license": "CC-BY-4.0",
                    "dictionaries": ["dicts/zi.dict.yaml", "dicts/jichu.dict.yaml",
                                     "dicts/duoyin.dict.yaml", "dicts/diming.dict.yaml",
                                     "dicts/mingren.dict.yaml", "dicts/yiren.dict.yaml"],
                },
            ],
            "languageModel": {
                "url": "https://example.invalid/words.lm.xz",
                "sha256": hashlib.sha256(self.model.read_bytes()).hexdigest(),
                "license": "CC-BY-SA-4.0",
                "licenseUrl": "https://example.invalid/legalcode.txt",
                "licenseSha256": hashlib.sha256(self.model_license.read_bytes()).hexdigest(),
            },
        }
        self.hotwords = self.scratch / "hotwords.tsv"
        self.hotwords.write_text("chong pao\t重跑\nxin ci\t新词\n", encoding="utf-8")

    def archive(self, source, files):
        archive = self.archives / f"{source['name']}-{source['revision']}.tar.gz"
        with tarfile.open(archive, "w:gz") as bundle:
            for name, text in files.items():
                data = text.encode("utf-8")
                member = tarfile.TarInfo(f"{source['name']}-{source['revision']}/{name}")
                member.size = len(data)
                bundle.addfile(member, io.BytesIO(data))
        source["archiveSha256"] = hashlib.sha256(archive.read_bytes()).hexdigest()
        return archive

    def build(self):
        self.archive(self.manifest["sources"][0], self.frost_files)
        self.archive(self.manifest["sources"][1], self.wanxiang_files)
        manifest = self.scratch / "manifest.json"
        manifest.write_text(json.dumps(self.manifest), encoding="utf-8")
        return subprocess.run(
            [sys.executable, "-X", "utf8", str(BUILDER), "--manifest", str(manifest),
             "--source-root", str(self.sources), "--output", str(self.output),
             "--model", str(self.model), "--hotwords", str(self.hotwords)],
            cwd=ROOT, capture_output=True, text=True, encoding="utf-8",
        )

    def entries(self):
        rows = {}
        for line in (self.output / "lexicon.tsv").read_text(encoding="utf-8").splitlines():
            if line.startswith("#"):
                continue
            code, word, weight = line.split("\t")
            self.assertNotIn((code, word), rows)
            rows[(code, word)] = float(weight)
        return rows

    def test_combines_complete_base_and_normalized_supplement(self):
        result = self.build()
        self.assertEqual(result.returncode, 0, result.stderr)
        rows = self.entries()
        self.assertEqual(rows[("ni hao", "你好")], 100)
        self.assertEqual(rows[("lv bu", "吕布")], 10)
        self.assertIn(("xiang liang jian suo", "向量检索"), rows)
        self.assertIn(("hai dian", "海淀"), rows)
        self.assertIn(("zhong pao", "重跑"), rows)
        self.assertNotIn(("jiao se", "角色"), rows)
        self.assertEqual(rows[("hou", "㕈")], 0.8)
        self.assertEqual(rows[("ji", "𠮷")], 0.7)

    def test_project_hotwords_keep_their_declared_reading(self):
        result = self.build()
        self.assertEqual(result.returncode, 0, result.stderr)
        rows = self.entries()
        self.assertEqual(rows[("chong pao", "重跑")], 30000)
        self.assertEqual(rows[("xin ci", "新词")], 30000)

    def test_blocklist_and_corpus_freq_blend_without_touching_rare_chars(self):
        self.frost_files["cn_dicts/base.dict.yaml"] += "去次\tqu ci\t88\n弃车保帅\tqi ju bao shuai\t50\n"
        self.hotwords.write_text(
            "chong pao\t重跑\n"
            "qi ju bao shuai\t弃车保帅\t1000\n"
            "qi che bao shuai\t弃车保帅\t1000\n",
            encoding="utf-8",
        )
        blocklist = self.scratch / "blocklist.tsv"
        blocklist.write_text("qu ci\t去次\n", encoding="utf-8")
        corpus_freq = self.scratch / "corpus.tsv"
        corpus_freq.write_text("你好\t100\n常用\t400\n", encoding="utf-8")
        self.manifest["blocklist"] = str(blocklist)
        self.manifest["corpusFreq"] = {
            "file": str(corpus_freq),
            "weight": 0.3,
            "maxFactor": 20,
            "floor": 1000,
            "source": "https://example.invalid/tatoeba",
            "export": "fixture",
            "license": "CC-BY-2.0-FR",
            "licenseUrl": "https://creativecommons.org/licenses/by/2.0/fr/",
            "attribution": "Fixture contributors",
        }
        result = self.build()
        self.assertEqual(result.returncode, 0, result.stderr)
        rows = self.entries()
        self.assertNotIn(("qu ci", "去次"), rows)
        self.assertEqual(rows[("qi ju bao shuai", "弃车保帅")], 1000)
        self.assertEqual(rows[("qi che bao shuai", "弃车保帅")], 1000)
        self.assertGreater(rows[("chang yong", "常用")], 50)
        self.assertEqual(rows[("hou", "㕈")], 0.8)
        self.assertEqual(rows[("ji", "𠮷")], 0.7)
        source_text = (self.output / "SOURCE.txt").read_text(encoding="utf-8")
        self.assertIn("CC-BY-2.0-FR", source_text)
        self.assertIn("Fixture contributors", source_text)

    def test_corresponding_source_snapshot_rebuilds_and_rejects_modified_inputs(self):
        result = self.build()
        self.assertEqual(result.returncode, 0, result.stderr)
        snapshot = self.scratch / "snapshot"
        files = {}
        for source, entries in zip(self.manifest["sources"], [self.frost_files, self.wanxiang_files]):
            for name, content in entries.items():
                if name.endswith(".py"):
                    continue
                relative = source["name"] + "/" + name
                path = snapshot / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(content.encode("utf-8"))
                files[relative] = hashlib.sha256(path.read_bytes()).hexdigest()
        license_path = snapshot / "licenses/CC-BY-SA-4.0.txt"
        license_path.parent.mkdir(parents=True)
        license_path.write_bytes(self.model_license.read_bytes())
        files["licenses/CC-BY-SA-4.0.txt"] = hashlib.sha256(license_path.read_bytes()).hexdigest()
        (snapshot / "source-manifest.json").write_text(
            json.dumps({"sources": self.manifest["sources"], "files": files}), encoding="utf-8")
        rebuilt = self.scratch / "rebuilt"
        arguments = [sys.executable, "-X", "utf8", str(BUILDER), "--manifest", str(self.scratch / "manifest.json"),
                     "--snapshot-root", str(snapshot), "--output", str(rebuilt),
                     "--model", str(self.model), "--hotwords", str(self.hotwords)]
        result = subprocess.run(arguments, cwd=ROOT, capture_output=True, text=True, encoding="utf-8")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((rebuilt / "lexicon.tsv").read_bytes(), (self.output / "lexicon.tsv").read_bytes())
        (snapshot / "rime-frost/cn_dicts/base.dict.yaml").write_text("modified", encoding="utf-8")
        result = subprocess.run(arguments, cwd=ROOT, capture_output=True, text=True, encoding="utf-8")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("快照", result.stderr)

    def test_scaling_preserves_base_order_and_rare_character_weights(self):
        self.manifest["frequencyTotal"] = 320
        result = self.build()
        self.assertEqual(result.returncode, 0, result.stderr)
        rows = self.entries()
        self.assertEqual(rows[("ni hao", "你好")], 200)
        self.assertEqual(rows[("chang yong", "常用")], 100)
        self.assertEqual(rows[("lv bu", "吕布")], 20)
        self.assertEqual(rows[("hou", "㕈")], 0.8)

    def test_packaging_check_rejects_modified_generated_data(self):
        result = self.build()
        self.assertEqual(result.returncode, 0, result.stderr)
        with (self.output / "lexicon.tsv").open("a", encoding="utf-8") as output:
            output.write("bad\tbad\t1\n")
        result = subprocess.run(
            [sys.executable, "-X", "utf8", str(BUILDER), "--check",
             "--manifest", str(self.scratch / "manifest.json"), "--output", str(self.output),
             "--hotwords", str(self.hotwords)],
            cwd=ROOT, capture_output=True, text=True, encoding="utf-8",
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("lexicon.tsv", result.stderr)

    def test_retains_licenses_and_records_actual_output_hashes(self):
        result = self.build()
        self.assertEqual(result.returncode, 0, result.stderr)
        licenses = (self.output / "LICENSE").read_text(encoding="utf-8")
        for expected in ("fixture GPL-3.0", "fixture CC BY 4.0", "fixture CC BY-SA 4.0"):
            self.assertIn(expected, licenses)
        self.assertFalse(any(self.output.rglob("*.py")))
        record = json.loads((self.output / "MANIFEST.json").read_text(encoding="utf-8"))
        self.assertEqual([item["revision"] for item in record["sources"]], ["1" * 40, "2" * 40])
        self.assertEqual(record["files"]["lexicon.tsv"]["sha256"],
                         hashlib.sha256((self.output / "lexicon.tsv").read_bytes()).hexdigest())
        self.assertIn("100", (self.output / "english.txt").read_text(encoding="utf-8"))

    def test_missing_primary_import_rejects_partial_dictionary(self):
        del self.frost_files["cn_dicts/base.dict.yaml"]
        result = self.build()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("base.dict.yaml", result.stderr)
        self.assertFalse((self.output / "MANIFEST.json").exists())

    def test_missing_supplement_rejects_partial_dictionary(self):
        del self.wanxiang_files["dicts/jichu.dict.yaml"]
        result = self.build()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("jichu.dict.yaml", result.stderr)
        self.assertFalse((self.output / "MANIFEST.json").exists())

    def test_model_digest_mismatch_does_not_replace_existing_output(self):
        self.model.write_bytes(b"wrong-model")
        self.output.mkdir()
        (self.output / "lexicon.tsv").write_text("keep-existing", encoding="utf-8")
        result = self.build()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual((self.output / "lexicon.tsv").read_text(encoding="utf-8"), "keep-existing")

    def test_converter_reports_a_missing_import_as_failure(self):
        root = self.scratch / "missing.dict.yaml"
        root.write_text("---\nname: missing\nimport_tables:\n  - absent\n...\n", encoding="utf-8")
        result = subprocess.run(
            [sys.executable, "-X", "utf8", str(CONVERTER), str(root)],
            capture_output=True, text=True, encoding="utf-8",
        )
        self.assertNotEqual(result.returncode, 0, "Missing tables must not produce a successful partial dictionary")
        self.assertIn("absent.dict.yaml", result.stderr)


if __name__ == "__main__":
    unittest.main()
