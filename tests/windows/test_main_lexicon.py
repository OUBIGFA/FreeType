"""Start the real service with a saved main dictionary selection."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import unittest

from test_update_handoff import ROOT, ServerFixture


@unittest.skipUnless(os.name == "nt", "Windows dictionary loader")
class MainLexiconTests(ServerFixture, unittest.TestCase):
    def start_with_notice(self):
        notice = self.resources.parent / "THIRD-PARTY-NOTICES.txt"
        shutil.copyfile(ROOT / "third_party/NOTICE.md", notice)
        config_path = self.data / "config.json"
        config = json.loads(config_path.read_text(encoding="utf-8"))
        config["lexicon"] = str(notice)
        config_path.write_text(json.dumps(config), encoding="utf-8")
        self.launch(self.server)
        return self.connect(), notice

    def loaded(self, connection, path=None):
        deadline = time.monotonic() + 10
        while True:
            status = self.call(connection, "status")["status"]
            if status.get("error") or (status["ready"] and (path is None or Path(status["lexicon"]) == path)):
                return status
            self.assertLess(time.monotonic(), deadline, status)
            time.sleep(0.02)

    def commit(self, connection, spelling):
        self.call(connection, "hello", session=True, app="dictionary-test.exe")
        for char in spelling + " ":
            frame = self.call(connection, "key", key={"keysym": ord(char), "modifiers": 0, "test": False})["frame"]
        return frame.get("commit")

    def test_rejected_notice_uses_packaged_index_and_accepts_a_later_valid_text_dictionary(self):
        source = self.lexicon.rename(self.root / "build-input.tsv")
        index = self.resources / "lexicon.ftd"
        subprocess.run([self.server, "--compile-lexicon", source, index], env=self.env, check=True,
                       creationflags=subprocess.CREATE_NO_WINDOW, timeout=10)
        connection, notice = self.start_with_notice()
        status = self.loaded(connection)
        self.assertTrue(status["ready"], status.get("error"))
        self.assertFalse(status.get("error"))
        self.assertEqual(Path(status["lexicon"]), index)
        self.assertIn(notice.name, status["lexicon_warning"])
        self.assertIn("第 3 行", status["lexicon_warning"])
        self.assertEqual(self.commit(connection, "nihao"), "你好")
        config = self.call(connection, "get_config")["config"]
        self.assertEqual(config["lexicon"], "")
        self.assertFalse(json.loads((self.data / "config.json").read_text(encoding="utf-8")).get("lexicon"))
        self.assertEqual(notice.read_bytes(), (ROOT / "third_party/NOTICE.md").read_bytes())

        custom = self.root / "custom.txt"
        custom.write_text("ce shi\t测试\t1000\n", encoding="utf-8")
        self.call(connection, "set_config", config={**config, "lexicon": str(custom)})
        status = self.loaded(connection, custom)
        self.assertFalse(status.get("error"), status)
        self.assertFalse(status.get("lexicon_warning"), status)
        self.assertEqual(self.commit(connection, "ceshi"), "测试")

    def test_broken_default_is_not_reported_as_a_successful_recovery(self):
        index = self.resources / "lexicon.ftd"
        index.write_bytes(b"not a dictionary index")
        connection, notice = self.start_with_notice()
        status = self.loaded(connection)
        self.assertFalse(status["ready"])
        self.assertIn(notice.name, status["error"])
        self.assertIn(index.name, status["error"])
        self.assertFalse(status.get("lexicon_warning"))
        self.assertEqual(self.call(connection, "get_config")["config"]["lexicon"], str(notice))
