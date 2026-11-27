"""Verify factory defaults through the real service, using a disposable profile."""

import json
import os
import unittest

from test_update_handoff import ServerFixture


@unittest.skipUnless(os.name == "nt", "Windows settings service")
class SettingsResetTests(ServerFixture, unittest.TestCase):
    copy_server = False

    def test_empty_config_restores_defaults_and_survives_restart(self):
        process = self.launch(self.server)
        connection = self.connect()
        self.wait_for(lambda: self.call(connection, "status")["status"]["ready"], "dictionary ready")
        phrases = [{"code": "dz", "text": "保留短语", "weight": 10}]
        self.call(connection, "set_quick_phrases", phrases=phrases)
        self.call(connection, "set_common_phrases", phrases=["保留常用语"])
        self.call(connection, "set_config", config={"font_size": 26, "shuangpin": True,
                  "tray_icon": False, "auto_update": True, "candidate_style": "blue"})

        # The settings app sends an empty object; the service owns every default.
        restored = self.call(connection, "set_config", config={})["config"]
        self.assertEqual(restored["font_size"], 18)
        self.assertFalse(restored["shuangpin"])
        self.assertTrue(restored["tray_icon"])
        self.assertFalse(restored["auto_update"])
        self.assertEqual(restored["candidate_style"], "pink")
        self.assertEqual(json.loads((self.data / "config.json").read_text(encoding="utf-8")), {})
        self.assertEqual(self.call(connection, "list_quick_phrases")["quick_phrases"], phrases)
        self.assertEqual(self.call(connection, "list_common_phrases")["common_phrases"], ["保留常用语"])

        connection.close()
        process.terminate()
        process.wait(timeout=10)
        self.launch(self.server)
        connection = self.connect()
        self.assertEqual(self.call(connection, "get_config")["config"], restored)
