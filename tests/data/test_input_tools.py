"""Generated lookup data must preserve actual words/readings, not a demonstration list."""
import importlib.util
from pathlib import Path
import struct
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("input_tools", ROOT / "tools" / "make_input_tools.py")
tools = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tools)


class InputToolsTests(unittest.TestCase):
    def test_english_keeps_phrases_case_readings_and_rank(self):
        rows = list(tools.english_entries([
            {"word": "NASA", "phonetic": "næsə", "translation": "美国航空航天局", "bnc": "100", "frq": "0"},
            {"word": "in spite of", "translation": "尽管\\n虽然", "bnc": "80"},
            {"word": "-able", "translation": "词缀"},
        ]))
        self.assertEqual([row[0] for row in rows], ["nasa", "inspiteof"])
        self.assertEqual(rows[0][2:4], ("NASA", "næsə"))
        self.assertIn("尽管", rows[1][4])
        self.assertEqual(tools.spelling_key("conectivity"), tools.spelling_key("connectivity"))

    def test_radical_deduplicates_and_selects_modern_polyphonic_readings(self):
        common = {"晶": ["jīng"], "长": ["zhǎng", "cháng", "zhàng"]}
        modern = [{"长": ["zhǎng", "cháng"]}, {"长": ["cháng"]}, {"长": ["zhǎng"]}]
        rows = list(tools.radical_entries(["晶\tri'ri'ri\t90", "晶\tri'ri'ri\t10", "长\tchang\t5", "A\ta\t100"], common, modern))
        self.assertEqual(len(rows), 4)
        by_code = {row[0]: row for row in rows}
        self.assertEqual(by_code["ririri"][2], "晶")
        self.assertEqual(by_code["ririri"][3], "jīng")
        self.assertEqual(by_code["jing"][2], "晶")
        self.assertEqual(by_code["chang"][3], "zhǎng / cháng")

    def test_lookup_table_has_ordered_offsets_and_deterministic_records(self):
        rows = [("b", 1, "乙", "yǐ", ""), ("a", 2, "甲", "jiǎ", "first")]
        data = tools.encode_table(rows)
        self.assertEqual(data[:8], b"FTLOOK01")
        self.assertEqual(struct.unpack_from("<I", data, 8)[0], 2)
        offsets = struct.unpack_from("<3Q", data, 16)
        self.assertLess(offsets[0], offsets[1])
        self.assertEqual(offsets[-1], len(data))
        self.assertEqual(data[offsets[0]:offsets[1]].decode("utf-8"), "a\t2\t甲\tjiǎ\tfirst\n")
        self.assertEqual(data, tools.encode_table(reversed(rows)))


if __name__ == "__main__":
    unittest.main()
