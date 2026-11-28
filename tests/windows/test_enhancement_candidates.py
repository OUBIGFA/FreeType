"""Exercise candidate preservation through the real service and optional model."""
import json
import os
from pathlib import Path
import shutil
import time
import unittest

from test_update_handoff import ROOT, ServerFixture


@unittest.skipUnless(os.name == "nt" and os.environ.get("FREETYPE_TEST_NOVA"), "requires the optional model")
class EnhancementCandidateTests(ServerFixture, unittest.TestCase):
    def test_model_refresh_preserves_learned_words_and_commits_the_selected_candidate(self):
        source = Path(os.environ.get("FREETYPE_TEST_RESOURCES", ROOT / "windows/app/resources/data"))
        self.env["FREETYPE_RESOURCE_DIR"] = str(source)
        dll = Path(os.environ.get("FREETYPE_TEST_ENHANCEMENT_DLL", ROOT / ".local/build/enhancement/FreeTypeEnhancement.dll"))
        shutil.copyfile(dll, self.server.parent / "FreeTypeEnhancement.dll")
        model = self.data / "enhancement"
        model.mkdir()
        shutil.copyfile(os.environ["FREETYPE_TEST_NOVA"], model / "nova-v1.gguf")
        (self.data / "config.json").write_text(json.dumps({"fuzzy": True, "fuzzy_groups": 65535,
            "sentence_editor": True, "page_size": 10, "emoji_suggest": False}), encoding="utf-8")
        self.launch(self.server)
        connection = self.connect()
        deadline = time.monotonic() + 40
        while not self.call(connection, "hello", session=True, app="model-regression.exe")["status"]["ready"]:
            self.assertLess(time.monotonic(), deadline)
            time.sleep(.02)

        def type_input(raw):
            for char in raw:
                key = {"keysym": ord(char), "modifiers": 0, "test": True}
                self.call(connection, "key", key=key)
                key["test"] = False
                frame = self.call(connection, "key", key=key)["frame"]
            return frame

        # First make an ordinary explicit selection, reproducing the learned-first
        # candidate that the old model refresh overwrote and lost completely.
        frame = type_input("zhiding")
        self.call(connection, "select", index=frame["candidates"].index("置顶"))
        self.call(connection, "set_enhancement", enabled=True)
        deadline = time.monotonic() + 40
        while True:
            status = self.call(connection, "enhancement_status")["enhancement"]
            self.assertFalse(status["error"], status)
            if status["state"] == "ready":
                break
            self.assertLess(time.monotonic(), deadline, status)
            time.sleep(.03)

        changed = False
        for context in ("我们需要", "由管理员", "按照", "把窗口"):
            for raw in ("zhiding", "zhidin", "ziding"):
                self.call(connection, "reset", commit=False)
                self.call(connection, "focus", active=True, before=context)
                frame = type_input(raw)
                before = frame["candidates"]
                deadline = time.monotonic() + 30
                while frame["enhancement_pending"]:
                    self.assertLess(time.monotonic(), deadline)
                    time.sleep(.05)
                    frame = self.call(connection, "refresh_enhancement", revision=frame["page_revision"])["frame"]
                changed |= before[0] != frame["candidates"][0]
                self.assertIn("置顶", frame["candidates"], (context, raw, before, frame["candidates"]))
                self.assertEqual(frame["candidates"].count("置顶"), 1)
                self.assertEqual(frame["sentence"]["text"], frame["candidates"][0])
        self.assertTrue(changed, "must exercise a real model replacement, not only unchanged results")
        selected = self.call(connection, "select", index=frame["candidates"].index("置顶"))["frame"]
        self.assertEqual(selected.get("commit"), "置顶")
        self.assertFalse(selected["preedit"])
