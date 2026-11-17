"""Real-server regressions for learned and imported dictionaries, without a desktop."""

import os
import json
import time
import unittest
from pathlib import Path

from test_update_handoff import ServerFixture


@unittest.skipUnless(os.name == "nt", "Windows input performance")
class InputPerformanceTests(ServerFixture, unittest.TestCase):

    def ready(self):
        server = self.launch(self.server)
        connection = self.connect()
        deadline = time.monotonic() + 30
        while not self.call(connection, "hello", session=True, app="performance.exe")["status"]["ready"]:
            self.assertLess(time.monotonic(), deadline, "Dictionary did not become ready")
            time.sleep(0.02)
        return server, connection

    def type_word(self, connection):
        self.call(connection, "reset", commit=False)
        for char in "nihao":
            frame = self.call(connection, "key", key={"keysym": ord(char), "modifiers": 0, "test": False})["frame"]
        self.assertEqual(frame["candidates"][0], "你好")
        started = time.perf_counter()
        frame = self.call(connection, "key", key={"keysym": 32, "modifiers": 0, "test": False})["frame"]
        self.assertEqual(frame["commit"], "你好")
        return time.perf_counter() - started

    def test_learned_words_load_in_one_batch(self):
        (self.data / "user.tsv").write_text(
            "".join(f"ce shi\t测试{i}\t1000\n" for i in range(2500)), encoding="utf-8")
        started = time.perf_counter()
        _, connection = self.ready()
        elapsed = time.perf_counter() - started
        self.assertEqual(len(self.call(connection, "list_user_words")["user_words"]), 2500)
        self.assertLess(elapsed, 2, f"Loading 2500 learned words took {elapsed:.2f} seconds")

    def test_learning_does_not_rebuild_imported_dictionary(self):
        _, connection = self.ready()
        dictionary = self.root / "extra.tsv"
        dictionary.write_text("".join(f"测{i}\tce\t1000\n" for i in range(300000)), encoding="utf-8")
        self.call(connection, "import_dictionary", path=str(dictionary), name="performance")
        elapsed = [self.type_word(connection) for _ in range(4)]
        self.assertTrue(any(word["word"] == "你好" for word in self.call(connection, "list_user_words")["user_words"]))
        self.assertLess(max(elapsed), 0.15, f"Learning rebuilt the imported dictionary: {elapsed}")

    def test_learning_is_saved_while_requests_keep_arriving(self):
        _, connection = self.ready()
        self.type_word(connection)
        deadline = time.monotonic() + 32
        history = self.data / "history.tsv"
        # Status polling and typing must not restart the 30-second save timer.
        while time.monotonic() < deadline and not history.exists():
            self.call(connection, "status")
            time.sleep(0.05)
        self.assertTrue(history.exists(), "Continuous requests starved the periodic learning save")
        self.assertIn("你好", history.read_text(encoding="utf-8"))

    def test_export_custom_dictionary_does_not_rebuild_query_indexes(self):
        main = Path(__file__).resolve().parents[2] / ".local/data/lexicon.tsv"
        (self.data / "config.json").write_text(json.dumps({"lexicon": str(main)}), encoding="utf-8")
        _, connection = self.ready()
        destination = self.root / "backup.json"
        started = time.perf_counter()
        self.call(connection, "export_settings", path=str(destination))
        elapsed = time.perf_counter() - started
        document = json.loads(destination.read_text(encoding="utf-8"))
        self.assertEqual(document["main_lexicon"].encode("utf-8"), main.read_bytes())
        self.assertLess(elapsed, 3, f"Backup rebuilt the full custom dictionary: {elapsed:.2f} seconds")
