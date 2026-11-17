"""Keep the real server out of an in-progress NSIS overwrite, without installing it."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import unittest
import uuid

ROOT = Path(__file__).resolve().parents[2]
INCLUDE = ROOT / "windows/app/installer.nsh"


@unittest.skipUnless(os.name == "nt", "Windows update handoff")
class UpdateHandoffTests(unittest.TestCase):
    def setUp(self):
        scratch = Path(os.environ.get("FREETYPE_TEST_TEMP", ROOT / "_temp/update-tests"))
        scratch.mkdir(parents=True, exist_ok=True)
        self.root = Path(tempfile.mkdtemp(prefix="handoff-", dir=scratch))
        self.instance = "update-" + uuid.uuid4().hex
        self.mutex = "Local\\FreeType.Server-" + self.instance
        self.compiler = Path(os.environ.get("FREETYPE_MAKENSIS", str(
            Path(os.environ["LOCALAPPDATA"]) / "mygo/nsis-3.13/makensis.exe")))
        self.server = self.root / "app/bin/FreeTypeServer.exe"
        self.server.parent.mkdir(parents=True)
        shutil.copyfile(Path(os.environ.get("FREETYPE_TEST_SERVER", ROOT / "target/release/FreeTypeServer.exe")), self.server)
        self.data = self.root / "data"
        self.data.mkdir()
        self.resources = self.root / "app/data"
        self.resources.mkdir()
        self.lexicon = self.resources / "lexicon.tsv"
        # More than one page of real words across unrelated single and multiple syllables.
        self.words = {
            "ge": "个歌各哥格隔割阁革葛", "gun": "滚棍辊衮磙鲧绲棞惃丨",
            "shi": "是时事市使十石世式师", "yi": "一以已意亿易衣依医义",
            "ni": "你尼泥拟逆妮倪腻呢昵", "hao": "好号浩豪耗郝皓昊毫嚎",
            "zhong": "中种重众终钟忠肿仲盅", "lv": "绿率旅律吕铝屡履缕侣",
            "nihao": ["你好", "拟好", "泥好", "你号", "尼豪", "妮好", "倪好", "你豪", "倪浩", "尼好"],
        }
        self.complete = "".join(
            f"{'ni hao' if code == 'nihao' else code}\t{word}\t{10000 - index * 100}\n"
            for code, words in self.words.items() for index, word in enumerate(words)
        )
        self.lexicon.write_text(self.complete, encoding="utf-8")
        config = {"page_size": 7, "emoji_suggest": False, "date_time": False,
                  "typo_correction": False, "fuzzy": False, "autospace": False}
        (self.data / "config.json").write_text(json.dumps(config), encoding="utf-8")
        self.env = {**os.environ, "FREETYPE_INSTANCE": self.instance,
                    "FREETYPE_DATA_DIR": str(self.data), "FREETYPE_RESOURCE_DIR": str(self.resources),
                    "XDG_CACHE_HOME": str(self.root / "cache")}
        import ctypes
        from ctypes import wintypes
        self.kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        self.kernel.OpenMutexW.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.LPCWSTR]
        self.kernel.OpenMutexW.restype = wintypes.HANDLE
        self.kernel.CloseHandle.argtypes = [wintypes.HANDLE]
        self.kernel.CloseHandle.restype = wintypes.BOOL
        self.processes = []
        self.files = []

    def tearDown(self):
        if self.mutex_exists():
            stop = subprocess.run([str(self.server), "--stop"], env=self.env,
                                  creationflags=subprocess.CREATE_NO_WINDOW, timeout=10)
            self.assertEqual(stop.returncode, 0)
        for file in self.files:
            file.close()
        for process in reversed(self.processes):
            if process.poll() is None:
                process.terminate()
            process.wait(timeout=10)

    def launch(self, path, *args):
        process = subprocess.Popen([str(path), *args], env=self.env, creationflags=subprocess.CREATE_NO_WINDOW)
        self.processes.append(process)
        return process

    def wait_for(self, predicate, description):
        deadline = time.monotonic() + 20
        while not predicate():
            self.assertLess(time.monotonic(), deadline, description)
            time.sleep(0.02)

    def mutex_exists(self):
        handle = self.kernel.OpenMutexW(0x00100000, False, self.mutex)
        if handle:
            self.kernel.CloseHandle(handle)
        return bool(handle)

    def connect(self):
        # A private instance suffix keeps these requests away from the installed service.
        import ctypes
        from ctypes import wintypes
        advapi = ctypes.WinDLL("advapi32", use_last_error=True)
        advapi.OpenProcessToken.argtypes = [wintypes.HANDLE, wintypes.DWORD, ctypes.POINTER(wintypes.HANDLE)]
        advapi.OpenProcessToken.restype = wintypes.BOOL
        advapi.GetTokenInformation.argtypes = [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p,
                                               wintypes.DWORD, ctypes.POINTER(wintypes.DWORD)]
        advapi.GetTokenInformation.restype = wintypes.BOOL
        token = wintypes.HANDLE()
        self.kernel.GetCurrentProcess.restype = wintypes.HANDLE
        self.assertTrue(advapi.OpenProcessToken(self.kernel.GetCurrentProcess(), 8, ctypes.byref(token)))
        try:
            size = wintypes.DWORD()
            advapi.GetTokenInformation(token, 1, None, 0, ctypes.byref(size))
            buf = ctypes.create_string_buffer(size.value)
            self.assertTrue(advapi.GetTokenInformation(token, 1, buf, size, ctypes.byref(size)))
            sid = ctypes.cast(buf, ctypes.POINTER(ctypes.c_void_p))[0]
            text = wintypes.LPWSTR()
            advapi.ConvertSidToStringSidW.argtypes = [ctypes.c_void_p, ctypes.POINTER(wintypes.LPWSTR)]
            self.assertTrue(advapi.ConvertSidToStringSidW(sid, ctypes.byref(text)))
            try:
                name = "\\\\.\\pipe\\FreeType-" + text.value + "-" + self.instance
            finally:
                self.kernel.LocalFree.argtypes = [ctypes.c_void_p]
                self.kernel.LocalFree(text)
        finally:
            self.kernel.CloseHandle(token)
        deadline = time.monotonic() + 10
        while True:
            try:
                file = open(name, "r+b", buffering=0)
                self.files.append(file)
                return file
            except OSError:
                self.assertLess(time.monotonic(), deadline, "Private server did not open its pipe")
                time.sleep(0.02)

    def call(self, file, op, **fields):
        file.write((json.dumps({"op": op, **fields}) + "\n").encode())
        response = json.loads(file.readline())
        self.assertTrue(response["ok"], response)
        return response

    def start_installer(self, *, abort=False):
        installer = self.root / "overwrite.exe"
        script = self.root / "overwrite.nsi"
        # Pause after the real preparation hook: the payload deliberately contains
        # only one word until the test allows replacement/registration to finish.
        script.write_text(
            'Unicode true\nRequestExecutionLevel user\nSilentInstall silent\n'
            f'Name "FreeType overwrite regression"\nOutFile "{installer}"\n'
            f'InstallDir "{self.server.parent.parent}"\n'
            '!include "MUI2.nsh"\n!insertmacro MUI_LANGUAGE "SimpChinese"\n'
            '!insertmacro MUI_LANGUAGE "TradChinese"\n!insertmacro MUI_LANGUAGE "English"\n'
            f'!define FREETYPE_SERVER_MUTEX "{self.mutex}"\n'
            f'!define FREETYPE_UPDATE_PENDING "{self.root / "pending.json"}"\n!include "{INCLUDE}"\n'
            'Section\n'
            f'FileOpen $0 "{self.root / "copying.txt"}" w\nFileClose $0\n'
            'StrCpy $1 0\nwait:\n'
            f'IfFileExists "{self.root / "continue.txt"}" ready 0\n'
            'Sleep 20\nIntOp $1 $1 + 1\nIntCmp $1 1000 timeout wait timeout\n'
            'timeout:\nSetErrorLevel 21\nQuit\nready:\n' +
            ('SetErrorLevel 22\nQuit\n' if abort else
             # Exercise the actual final launch hook with a nonexistent tray exe;
             # no UI, registration, current-user settings or installed app is touched.
             'Call FreeTypeLaunchAfterInstall\n') + 'SectionEnd\n', encoding="utf-8-sig")
        result = subprocess.run([str(self.compiler), "-V2", str(script)], capture_output=True, timeout=30)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return self.launch(installer, "/S")

    def test_overwrite_blocks_relaunch_until_complete_then_all_queries_and_page_sizes_work(self):
        initial = self.launch(self.server)
        self.wait_for(self.mutex_exists, "Initial server did not start")
        installer = self.start_installer()
        self.wait_for(lambda: (self.root / "copying.txt").exists(), "Installer preparation did not finish")
        self.assertIsNotNone(initial.poll(), "The previous server must stop")
        self.lexicon.write_text("ni\t你\t10000\n", encoding="utf-8")
        # Equivalent to old DLLs launching the service while NSIS copies resources.
        for _ in range(3):
            retry = self.launch(self.server)
            self.wait_for(lambda: retry.poll() is not None, "Server started against a partially copied dictionary")
            self.assertEqual(retry.returncode, 0)
        self.lexicon.write_text(self.complete, encoding="utf-8")
        (self.root / "continue.txt").touch()
        self.assertEqual(installer.wait(timeout=10), 0)
        # 完成钩子必须自己启动并验证新版服务；不能等重启电脑或测试手动拉起。
        self.assertTrue(self.mutex_exists(), "The completed installer must leave a ready server running")
        connection = self.connect()
        self.wait_for(lambda: self.call(connection, "hello", session=True, app="update-test.exe")["status"]["ready"],
                      "Dictionary did not load after the update")
        for page_size in [3, 7, 10]:
            # Each matrix row starts with the same learning state; choosing a word
            # in the previous row legitimately promotes it on the next query.
            self.call(connection, "clear_learned")
            config = self.call(connection, "get_config")["config"]
            config["page_size"] = page_size
            self.call(connection, "set_config", config=config)
            for code, words in self.words.items():
                with self.subTest(page_size=page_size, code=code):
                    self.call(connection, "reset", commit=False)
                    self.call(connection, "focus", active=True, before="")
                    for char in code:
                        # Old clients still send test/real pairs over the same JSON protocol.
                        for test in [True, False]:
                            frame = self.call(connection, "key", key={"keysym": ord(char), "modifiers": 0, "test": test})["frame"]
                    self.assertEqual(len(frame["candidates"]), page_size, frame)
                    self.assertEqual(frame["candidates"][0], words[0], frame)
                    selected = self.call(connection, "select", index=page_size - 1)["frame"]
                    self.assertEqual(selected["commit"], frame["candidates"][-1])

    def test_aborted_installer_releases_barrier(self):
        installer = self.start_installer(abort=True)
        self.wait_for(lambda: (self.root / "copying.txt").exists(), "Installer preparation did not finish")
        self.assertTrue(self.mutex_exists(), "Preparation must hold the server mutex")
        (self.root / "continue.txt").touch()
        self.assertEqual(installer.wait(timeout=10), 22)
        self.wait_for(lambda: not self.mutex_exists(), "Aborting must not leave a persistent startup blocker")
        server = self.launch(self.server)
        self.wait_for(self.mutex_exists, "Server did not restart after the installer exited")
        self.assertIsNone(server.poll())

    def test_ready_server_does_not_build_fuzzy_index_on_first_input(self):
        # The small handoff fixture hides the cost of a lazy index over the full
        # shipped dictionary. Use both a fresh snapshot and a cached restart.
        for name in ("lexicon.tsv", "words.lm", "english.txt"):
            shutil.copyfile(ROOT / ".local/data" / name, self.resources / name)
        timings = []
        for fuzzy, code, expected in [(True, "nihao", "你好"), (False, "yingai", "应该")]:
            (self.data / "config.json").write_text(
                json.dumps({"fuzzy": fuzzy, "predict_next": True}), encoding="utf-8")
            server = self.launch(self.server)
            connection = self.connect()
            deadline = time.monotonic() + 45
            while not self.call(connection, "hello", session=True, app="startup-test.exe")["status"]["ready"]:
                self.assertLess(time.monotonic(), deadline, "The full dictionary never became ready")
                time.sleep(0.05)
            for attempt in range(2):
                self.call(connection, "reset", commit=False)
                self.call(connection, "focus", active=True, before="")
                for char in code:
                    started = time.perf_counter()
                    for test in [True, False]:
                        frame = self.call(connection, "key", key={
                            "keysym": ord(char), "modifiers": 0, "test": test})["frame"]
                        self.assertTrue(frame["handled"], frame)
                    timings.append((fuzzy, attempt, char, time.perf_counter() - started))
                    self.assertTrue(frame["candidates"], frame)
                self.assertIn(expected, frame["candidates"], frame)
            self.call(connection, "shutdown")
            self.assertEqual(server.wait(timeout=10), 0)
            connection.close()
        # Leave a wide margin over normal millisecond queries while catching the
        # measured 1.4–1.5 s index build, which can exceed the TSF 1.5 s timeout.
        slow = [(fuzzy, attempt, char, round(seconds * 1000, 1))
                for fuzzy, attempt, char, seconds in timings if seconds >= 0.5]
        self.assertFalse(slow, f"Input after ready exceeded 500 ms (fuzzy, attempt, key, ms): {slow}")


if __name__ == "__main__":
    unittest.main()
