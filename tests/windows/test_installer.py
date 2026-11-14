"""Execute the real NSIS preparation and launch hooks with disposable processes."""

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import unittest


ROOT = Path(__file__).resolve().parents[2]
INCLUDE = ROOT / "windows" / "app" / "installer.nsh"


@unittest.skipUnless(os.name == "nt", "Windows installer test")
class InstallerPreparationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        scratch = Path(os.environ.get("FREETYPE_TEST_TEMP", ROOT / "_temp" / "installer-tests"))
        scratch.mkdir(parents=True, exist_ok=True)
        cls.scratch = Path(tempfile.mkdtemp(prefix="installer-", dir=scratch))
        cls.compiler = Path(os.environ.get(
            "FREETYPE_MAKENSIS",
            str(Path(os.environ["LOCALAPPDATA"]) / "mygo" / "nsis-3.13" / "makensis.exe"),
        ))
        if not cls.compiler.is_file():
            raise RuntimeError("NSIS is required; run windows/build.ps1 to prepare it.")
        cls.fixture = cls.scratch / "process.exe"
        source = cls.scratch / "process.cs"
        source.write_text(
            'using System.Threading; public static class Fixture {'
            ' public static int Main(string[] args) {'
            ' if (args.Length > 0 && args[0] == "--stop") {'
            ' System.IO.File.WriteAllText(System.IO.Path.Combine('
            ' System.AppDomain.CurrentDomain.BaseDirectory, "stop-requested.txt"), "requested"); return 0; }'
            ' if (System.Environment.GetEnvironmentVariable("FREETYPE_INSTALL_COMPLETED") == "1") {'
            ' string name = args.Length > 0 ? "after-install.txt" : "finish-page.txt";'
            ' System.IO.File.WriteAllText(System.IO.Path.Combine('
            ' System.AppDomain.CurrentDomain.BaseDirectory, name),'
            ' "1|" + string.Join(" ", args)); return 0; }'
            ' Thread.Sleep(Timeout.Infinite); return 0; } }',
            encoding="utf-8",
        )
        subprocess.run(
            ["powershell", "-NoProfile", "-NonInteractive", "-Command",
             "Add-Type -Path $env:FT_TEST_SOURCE -OutputAssembly $env:FT_TEST_EXE -OutputType ConsoleApplication"],
            env={**os.environ, "FT_TEST_SOURCE": str(source), "FT_TEST_EXE": str(cls.fixture)},
            check=True, capture_output=True, timeout=30,
        )

    def setUp(self):
        self.directory = Path(tempfile.mkdtemp(prefix="case-", dir=self.scratch))
        self.target = self.directory / "FreeType 安装 & target"
        self.target.mkdir()
        self.processes = []

    def tearDown(self):
        for process in self.processes:
            if process.poll() is None:
                process.terminate()
            process.wait(timeout=10)

    def start(self, path):
        path.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(self.fixture, path)
        process = subprocess.Popen([str(path)], creationflags=subprocess.CREATE_NO_WINDOW)
        self.processes.append(process)
        return process

    def run_preparation(self, extra=""):
        installer = self.directory / "preparation.exe"
        script = self.directory / "preparation.nsi"
        script.write_text(
            'Unicode true\nRequestExecutionLevel user\nSilentInstall silent\n'
            f'Name "FreeType preparation test"\nOutFile "{installer}"\nInstallDir "{self.target}"\n'
            '!include "MUI2.nsh"\n'
            '!insertmacro MUI_LANGUAGE "SimpChinese"\n'
            '!insertmacro MUI_LANGUAGE "TradChinese"\n'
            '!insertmacro MUI_LANGUAGE "English"\n'
            f'!include "{INCLUDE}"\n'
            'Section\n' + extra +
            'FileOpen $0 "$INSTDIR\\prepared.txt" w\nFileWrite $0 "ready"\nFileClose $0\nSectionEnd\n',
            encoding="utf-8-sig",
        )
        compiled = subprocess.run([str(self.compiler), "-V2", str(script)], capture_output=True, timeout=30)
        self.assertEqual(compiled.returncode, 0, compiled.stdout + compiled.stderr)
        result = subprocess.run([str(installer), "/S"], capture_output=True, timeout=45)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual((self.target / "prepared.txt").read_text(), "ready")

    def test_stops_only_target_installation_before_copying(self):
        tray = self.start(self.target / "FreeType.exe")
        server = self.start(self.target / "bin" / "FreeTypeServer.exe")
        unrelated = self.start(self.directory / "other" / "FreeType.exe")
        other_program = self.start(self.target / "OtherProgram.exe")
        self.run_preparation()
        self.assertIsNotNone(tray.poll(), "The tray must stop before copying files")
        self.assertIsNotNone(server.poll(), "The server must stop before copying files")
        self.assertIsNone(unrelated.poll(), "A same-named process outside the install folder must be preserved")
        self.assertIsNone(other_program.poll(), "Other programs in the same folder must be preserved")
        self.assertEqual((self.target / "bin" / "stop-requested.txt").read_text(), "requested")

    def test_fresh_install_does_not_require_running_processes(self):
        self.run_preparation()

    def test_install_completion_marks_automatic_and_finish_page_launches(self):
        shutil.copyfile(self.fixture, self.target / "FreeType.exe")
        self.run_preparation(
            'Call FreeTypeLaunchAfterInstall\n'
            # mygo's finish page launches without arguments; it must inherit the marker too.
            'ExecWait \'"$INSTDIR\\FreeType.exe"\'\n'
        )
        automatic = self.target / "after-install.txt"
        deadline = time.monotonic() + 5
        while not automatic.exists() and time.monotonic() < deadline:
            time.sleep(0.02)
        self.assertEqual(automatic.read_text(encoding="utf-8"), "1|--after-install")
        self.assertEqual((self.target / "finish-page.txt").read_text(encoding="utf-8"), "1|")


if __name__ == "__main__":
    unittest.main()
