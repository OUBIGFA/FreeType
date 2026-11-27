"""Opt-in diagnostics exercise the real service, with isolated data and no desktop."""
import json
import os
import time
import unittest

from test_update_handoff import ServerFixture


@unittest.skipUnless(os.name == "nt", "Windows real service")
class CandidateDiagnosticsTests(ServerFixture, unittest.TestCase):
    def ready(self):
        self.launch(self.server)
        connection = self.connect()
        self.wait_for(lambda: self.call(connection, "hello", session=True, app="diagnostics-test.exe")["status"]["ready"], "dictionary ready")
        return connection

    def type_text(self, connection, text, preview=False):
        for char in text:
            key = {"keysym": ord(char), "modifiers": 0, "test": False}
            if preview:
                self.call(connection, "key", key={**key, "test": True})
            self.call(connection, "key", key=key)

    def test_opt_in_records_once_and_export_clear_keep_plain_diagnostics_separate(self):
        connection = self.ready()
        log = self.data / "local/candidate-diagnostics.jsonl"
        self.type_text(connection, "nihao ")
        self.assertFalse(log.exists(), "default-off must not create a content log")
        config = self.call(connection, "get_config")["config"]
        config["candidate_diagnostics"] = True
        self.call(connection, "set_config", config=config)
        self.type_text(connection, "nihao ", preview=True)
        deadline = time.monotonic() + 3
        while not log.exists() and time.monotonic() < deadline:
            time.sleep(.02)
        self.assertTrue(log.exists(), "enabled real service did not write a candidate record")
        exported = self.root / "records.jsonl"
        self.call(connection, "export_candidate_diagnostics", path=str(exported))
        events = [json.loads(line) for line in exported.read_text(encoding="utf-8").splitlines()]
        commits = [e for e in events if e["e"] == "commit"]
        self.assertEqual(len(commits), 1, "preview must not create a duplicate event")
        event = commits[0]
        self.assertEqual((event["input"], event["text"]), ("nihao", "你好"))
        self.assertEqual(event["steps"][0]["pick"], 1)
        self.assertEqual(event["steps"][0]["cands"][0]["t"], "你好")
        samples = self.root / "samples.tsv"
        self.call(connection, "export_candidate_diagnostics", path=str(samples), evaluation=True)
        self.assertIn("nihao\t你好", samples.read_text(encoding="utf-8"))
        self.call(connection, "clear_candidate_diagnostics")
        self.call(connection, "export_candidate_diagnostics", path=str(exported))
        self.assertEqual(exported.read_text(encoding="utf-8"), "")
        config["candidate_diagnostics"] = False
        self.call(connection, "set_config", config=config)
        self.type_text(connection, "nihao ")
        self.call(connection, "export_candidate_diagnostics", path=str(exported))
        self.assertEqual(exported.read_text(encoding="utf-8"), "")

    def test_clear_discards_in_progress_content_and_export_cannot_overwrite_live_log(self):
        connection = self.ready()
        config = self.call(connection, "get_config")["config"]
        config["candidate_diagnostics"] = True
        self.call(connection, "set_config", config=config)
        self.type_text(connection, "nihao")
        self.call(connection, "clear_candidate_diagnostics")
        self.call(connection, "reset", commit=False)
        exported = self.root / "after-clear.jsonl"
        self.call(connection, "export_candidate_diagnostics", path=str(exported))
        self.assertNotIn("nihao", exported.read_text(encoding="utf-8"))
        self.type_text(connection, "nihao ")
        self.call(connection, "export_candidate_diagnostics", path=str(exported))
        live = self.data / "local/candidate-diagnostics.jsonl"
        before = live.read_bytes()
        connection.write((json.dumps({"op": "export_candidate_diagnostics", "path": str(live)}) + "\n").encode())
        self.assertFalse(json.loads(connection.readline())["ok"])
        self.assertEqual(live.read_bytes(), before)

    def test_sensitive_and_tool_input_are_excluded_and_restore_cannot_enable_recording(self):
        connection = self.ready()
        config = self.call(connection, "get_config")["config"]
        config["candidate_diagnostics"] = True
        self.call(connection, "set_config", config=config)
        self.call(connection, "sensitive_focus", active=True)
        self.type_text(connection, "secretpass ")
        self.call(connection, "sensitive_focus", active=False)
        self.call(connection, "reset", commit=False)
        self.call(connection, "command", command="toolbar")
        self.type_text(connection, "secretsearch")
        self.call(connection, "reset", commit=False)
        exported = self.root / "private-check.jsonl"
        self.call(connection, "export_candidate_diagnostics", path=str(exported))
        self.assertNotIn("secret", exported.read_text(encoding="utf-8"))
        backup = self.root / "settings.json"
        self.call(connection, "export_settings", path=str(backup))
        self.assertFalse(json.loads(backup.read_text(encoding="utf-8"))["config"].get("candidate_diagnostics", False))
        config["candidate_diagnostics"] = False
        self.call(connection, "set_config", config=config)
        malicious = json.loads(backup.read_text(encoding="utf-8"))
        malicious["config"]["candidate_diagnostics"] = True
        backup.write_text(json.dumps(malicious, ensure_ascii=False), encoding="utf-8")
        self.call(connection, "import_settings", path=str(backup))
        self.assertFalse(self.call(connection, "get_config")["config"]["candidate_diagnostics"])
