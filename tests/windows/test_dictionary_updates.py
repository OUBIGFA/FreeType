"""Dictionary update packages through the real isolated service: staging, next-start activation and fallback."""
import hashlib
import json
import os
import subprocess
import time
import unittest

from test_update_handoff import ServerFixture


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


@unittest.skipUnless(os.name == "nt", "Windows service")
class DictionaryUpdateTests(ServerFixture, unittest.TestCase):
    def package(self, version, extra_line):
        """Compile a package with the server's own compiler, as the weekly workflow does."""
        directory = self.data / "local" / "data-updates" / version
        directory.mkdir(parents=True)
        source = self.root / f"{version}.tsv"
        source.write_text(self.complete + extra_line, encoding="utf-8")
        compiled = subprocess.run([str(self.server), "--compile-lexicon", str(source), str(directory / "lexicon.ftd")],
                                  env=self.env, creationflags=subprocess.CREATE_NO_WINDOW, timeout=60)
        self.assertEqual(compiled.returncode, 0)
        (directory / "english.txt").write_text("github\n", encoding="utf-8")
        files = {name: {"bytes": (directory / name).stat().st_size, "sha256": sha256(directory / name)}
                 for name in ("lexicon.ftd", "english.txt")}
        manifest = {"format": 1, "channel": "freetype-data", "version": version, "lexiconFormat": "FTD2", "files": files}
        (directory / "PACKAGE.json").write_text(json.dumps(manifest), encoding="utf-8")
        return directory

    def start(self):
        self.launch(self.server)
        connection = self.connect()
        deadline = time.monotonic() + 40
        while not self.call(connection, "hello", session=True, app="dictionary-update.exe")["status"]["ready"]:
            self.assertLess(time.monotonic(), deadline)
            time.sleep(.02)
        return connection

    def restart(self):
        stop = subprocess.run([str(self.server), "--stop"], env=self.env,
                              creationflags=subprocess.CREATE_NO_WINDOW, timeout=20)
        self.assertEqual(stop.returncode, 0)
        self.wait_for(lambda: not self.mutex_exists(), "service did not stop")
        for file in self.files:
            file.close()
        self.files.clear()
        return self.start()

    def candidates(self, connection, raw):
        self.call(connection, "reset", commit=False)
        frame = None
        for char in raw:
            key = {"keysym": ord(char), "modifiers": 0, "test": False}
            frame = self.call(connection, "key", key=key)["frame"]
        return frame["candidates"]

    def test_staged_package_applies_on_next_start_and_a_damaged_one_falls_back(self):
        connection = self.start()
        self.call(connection, "add_user_word", word="个人词条", pinyin="ge ren")
        status = self.call(connection, "dictionary_update_status")["dictionary_update"]
        self.assertEqual(status["active"], "")
        self.package("2026.11.29.1", "xin ci\t新词更新\t90000\n")
        staged = self.call(connection, "stage_dictionary_update", version="2026.11.29.1", automatic=False)
        self.assertEqual(staged["dictionary_update"]["pending"], "2026.11.29.1")
        self.assertNotIn("新词更新", self.candidates(connection, "xinci"), "staging must not switch while typing")

        connection = self.restart()
        status = self.call(connection, "dictionary_update_status")["dictionary_update"]
        self.assertEqual((status["active"], status["current"], status["pending"]), ("2026.11.29.1", "2026.11.29.1", ""))
        self.assertIn("新词更新", self.candidates(connection, "xinci"))
        self.assertIn("个人词条", self.candidates(connection, "geren"), "personal data stays in its own directory")

        damaged = self.package("2026.12.06.1", "geng xin\t更新二\t90000\n")
        self.call(connection, "stage_dictionary_update", version="2026.12.06.1", automatic=True)
        data = bytearray((damaged / "lexicon.ftd").read_bytes())
        data[-1] ^= 0xFF
        (damaged / "lexicon.ftd").write_bytes(bytes(data))

        connection = self.restart()
        status = self.call(connection, "dictionary_update_status")["dictionary_update"]
        self.assertEqual(status["active"], "2026.11.29.1", status)
        self.assertEqual(status["failed_version"], "2026.12.06.1")
        self.assertIn("新词更新", self.candidates(connection, "xinci"))

        self.call(connection, "restore_previous_dictionary")
        connection = self.restart()
        status = self.call(connection, "dictionary_update_status")["dictionary_update"]
        self.assertEqual((status["active"], status["previous"]), ("", "2026.11.29.1"), status)
        self.assertNotIn("新词更新", self.candidates(connection, "xinci"))


if __name__ == "__main__":
    unittest.main()
