"""Real-server regressions for learned and imported dictionaries, without a desktop."""

import os
import json
import subprocess
import time
import unittest
from pathlib import Path

from test_update_handoff import ROOT, ServerFixture


@unittest.skipUnless(os.name == "nt", "Windows input performance")
class InputPerformanceTests(ServerFixture, unittest.TestCase):
    # As with an installed service, keep the executable path stable across launches.
    # Each process still gets private resources, writable files, mutex and pipe.
    copy_server = False

    def test_complete_english_and_radical_data_work_through_the_real_service(self):
        import shutil
        source = ROOT / ".local/data"
        for name in ("english-lookup.ftd", "english-spelling.ftd", "radical-lookup.ftd", "INPUT-TOOLS.json"):
            shutil.copyfile(source / name, self.resources / name)
        _, connection = self.ready()
        config = self.call(connection, "get_config")["config"]
        config.update(english_correction=True, english_space=True, english_gloss=True)
        self.call(connection, "set_config", config=config)
        for text, expected in [("inspiteof", "in spite of"), ("Tes", "Test"), ("conectivity", "connectivity")]:
            self.call(connection, "reset", commit=False)
            self.call(connection, "command", command="english_candidates")
            elapsed = []
            for char in text:
                started = time.perf_counter()
                frame = self.call(connection, "key", key={"keysym": ord(char), "modifiers": 0, "test": False})["frame"]
                elapsed.append(time.perf_counter() - started)
            self.assertLess(max(elapsed), 0.2, f"English query blocked an input key: {text} {elapsed}")
            self.assertIn(expected, frame["candidates"])
            self.assertTrue(frame["vertical"])
            self.assertTrue(any(frame["annotations"]))
            chosen = frame["candidates"].index(expected)
            self.assertEqual(self.call(connection, "select", index=chosen)["frame"]["commit"], expected + " ")
        self.call(connection, "reset", commit=False)
        self.call(connection, "command", command="radical_lookup")
        for char in "ririri":
            frame = self.call(connection, "key", key={"keysym": ord(char), "modifiers": 0, "test": False})["frame"]
        index = frame["candidates"].index("晶")
        self.assertIn("jīng", frame["annotations"][index])
        self.assertEqual(self.call(connection, "select", index=index)["frame"]["commit"], "晶")

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

    def test_shipped_dictionary_resident_memory(self):
        # Exercise the packaged resources and actual server, including both language models.
        root = Path(__file__).resolve().parents[2]
        self.env["FREETYPE_RESOURCE_DIR"] = os.environ.get("FREETYPE_TEST_RESOURCES", str(root / ".local/data"))
        (self.data / "config.json").write_text("{}", encoding="utf-8")
        server, connection = self.ready()
        for _ in range(10):
            self.call(connection, "focus", active=True, before="")
            for code, expected in [("nihao", "你好"), ("zhongguo", "中国"), ("yingai", "应该"), ("shijie", "世界")]:
                self.call(connection, "reset", commit=False)
                for char in code:
                    frame = self.call(connection, "key", key={"keysym": ord(char), "modifiers": 0, "test": False})["frame"]
                self.assertEqual(frame["candidates"][0], expected)
                self.call(connection, "key", key={"keysym": 32, "modifiers": 0, "test": False})
        # Reuse this full-dictionary process to cover completion through the real pipe,
        # with both models loaded. No extra copy or second expensive startup is needed.
        for shuangpin, code, expected in [
            (False, "zhonghuarenmin", "中华人民共和国"),
            (False, "chuangqianmingyue", "床前明月光"),
            (True, "vshxrfmb", "中华人民共和国"),
        ]:
            config = self.call(connection, "get_config")["config"]
            config.update(shuangpin=shuangpin, shuangpin_scheme="xiaohe", long_word_completion=True)
            self.call(connection, "set_config", config=config)
            self.call(connection, "reset", commit=False)
            for char in code:
                started = time.perf_counter()
                frame = self.call(connection, "key", key={"keysym": ord(char), "modifiers": 0, "test": False})["frame"]
                self.assertLess(time.perf_counter() - started, 1.0, "Completion approached the TSF 1.5-second timeout")
            self.assertEqual(frame["candidates"][1], expected)
            self.assertFalse(frame["badges"][1] if frame["badges"] else "", "Completed words must display without a badge")
            commit = self.call(connection, "key", key={"keysym": ord("2"), "modifiers": 0, "test": False})["frame"]
            self.assertEqual(commit["commit"], expected)
        counters = subprocess.check_output(["powershell", "-NoProfile", "-Command",
            f"Get-Process -Id {server.pid} | Select-Object WorkingSet64,PrivateMemorySize64 | ConvertTo-Json"])
        for name, size in json.loads(counters).items():
            self.assertLess(size / 2**20, 256, f"{name} for the full dictionary: {size / 2**20:.0f} MiB")

    def test_learning_does_not_rebuild_imported_dictionary(self):
        _, connection = self.ready()
        dictionary = self.root / "extra.tsv"
        dictionary.write_text("".join(f"测{i}\tce\t1000\n" for i in range(300000)), encoding="utf-8")
        self.call(connection, "import_dictionary", path=str(dictionary), name="performance")
        config = self.call(connection, "get_config")["config"]
        config["page_size"] = 5
        started = time.perf_counter()
        self.call(connection, "set_config", config=config)
        self.assertLess(time.perf_counter() - started, 0.5, "Changing page size reloaded imported words")
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

    def test_shutdown_keeps_unsaved_learning_available_for_retry(self):
        server, connection = self.ready()
        self.type_word(connection)
        blocked = self.data / "history.tsv"
        blocked.mkdir()
        try:
            connection.write(b'{"op":"shutdown"}\n')
            self.assertFalse(json.loads(connection.readline())["ok"])
            self.assertTrue(self.call(connection, "status")["status"]["ready"])
        finally:
            blocked.rename(self.data / "blocked-history")
        self.call(connection, "shutdown")
        self.assertEqual(server.wait(timeout=10), 0)
        self.assertIn("你好", blocked.read_text(encoding="utf-8"))

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
