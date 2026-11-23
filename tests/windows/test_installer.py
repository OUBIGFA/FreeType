"""Execute the real NSIS preparation and launch hooks with disposable processes."""

import os
from pathlib import Path
import re
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
            ' [System.Runtime.InteropServices.DllImport("advapi32.dll", SetLastError=true)]'
            ' static extern bool SetKernelObjectSecurity(System.IntPtr handle, uint info, byte[] descriptor);'
            ' [System.Runtime.InteropServices.DllImport("kernel32.dll")]'
            ' static extern System.IntPtr GetCurrentProcess();'
            ' public static int Main(string[] args) {'
            ' if (args.Length > 0 && args[0] == "--stop") {'
            ' System.IO.File.WriteAllText(System.IO.Path.Combine('
            ' System.AppDomain.CurrentDomain.BaseDirectory, "stop-requested.txt"), "requested"); return 0; }'
            ' if (args.Length > 0 && (args[0] == "--enable" || args[0] == "--disable" || args[0] == "--unregister-tsf" || args[0] == "--start-and-check")) {'
            ' System.IO.File.AppendAllText(System.IO.Path.Combine('
            ' System.AppDomain.CurrentDomain.BaseDirectory, "setup-calls.txt"), args[0] + "\\n");'
            ' string variable = args[0] == "--enable" ? "ENABLE" : args[0] == "--disable" ? "DISABLE" : args[0] == "--start-and-check" ? "START" : "UNREGISTER";'
            ' int code = int.Parse(System.Environment.GetEnvironmentVariable("FREETYPE_TEST_" + variable + "_EXIT") ?? "0");'
            ' int marker = System.Array.IndexOf(args, "--marker");'
            ' if (code == 0 && args[0] == "--unregister-tsf" && marker >= 0 && marker + 1 < args.Length) {'
            ' System.IO.File.WriteAllText(args[marker + 1], "ok"); } return code; }'
            ' if (System.Environment.GetEnvironmentVariable("FREETYPE_INSTALL_COMPLETED") == "1") {'
            ' string name = args.Length > 0 ? "after-install.txt" : "finish-page.txt";'
            ' System.IO.File.WriteAllText(System.IO.Path.Combine('
            ' System.AppDomain.CurrentDomain.BaseDirectory, name),'
            ' "1|" + string.Join(" ", args)); return 0; }'
            ' if (args.Length > 0 && args[0] == "--ignore-stop") { Thread.Sleep(Timeout.Infinite); return 0; }'
            ' if (args.Length > 0 && args[0] == "--limited-query") {'
            # Match the relevant elevated-process boundary without a UAC prompt: the
            # installer can query/wait, but cannot read modules or terminate the fixture.
            ' string sid = System.Security.Principal.WindowsIdentity.GetCurrent().User.Value;'
            ' var sd = new System.Security.AccessControl.RawSecurityDescriptor("D:P(A;;0x101000;;;" + sid + ")");'
            ' byte[] bytes = new byte[sd.BinaryLength]; sd.GetBinaryForm(bytes, 0);'
            ' if (!SetKernelObjectSecurity(GetCurrentProcess(), 4, bytes)) return 19;'
            ' string root = System.AppDomain.CurrentDomain.BaseDirectory;'
            ' System.IO.File.WriteAllText(System.IO.Path.Combine(root, "limited-ready.txt"), "ready");'
            ' while (!System.IO.File.Exists(System.IO.Path.Combine(root, "stop-requested.txt"))) Thread.Sleep(10);'
            ' return 0; }'
            ' string stop = System.IO.Path.Combine(System.AppDomain.CurrentDomain.BaseDirectory, "stop-requested.txt");'
            ' while (!System.IO.File.Exists(stop)) Thread.Sleep(10); return 0; } }',
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
        # Real preparation now holds the server mutex across the entire install.
        # Each disposable NSIS installer must use a private name, never the user's.
        self.guard_define = f'!define FREETYPE_SERVER_MUTEX "Local\\FreeType.Test-{self.directory.name}"\n'
        self.guard_define += f'!define FREETYPE_UPDATE_PENDING "{self.directory / "pending.json"}"\n'
        # Uninstall deletes the user's data directories; disposable installers get private ones.
        self.user_data = self.directory / "roaming" / "FreeType"
        self.local_data = self.directory / "local" / "FreeType"
        self.guard_define += f'!define FREETYPE_USER_DATA "{self.user_data}"\n'
        self.guard_define += f'!define FREETYPE_LOCAL_DATA "{self.local_data}"\n'

    def tearDown(self):
        for process in self.processes:
            if process.poll() is None:
                process.terminate()
            process.wait(timeout=10)

    def start(self, path, *arguments):
        path.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(self.fixture, path)
        process = subprocess.Popen([str(path), *arguments], creationflags=subprocess.CREATE_NO_WINDOW)
        self.processes.append(process)
        return process

    def run_preparation(self, extra="", expected_code=0, environment=None, install_dir=None):
        installer = self.directory / "preparation.exe"
        script = self.directory / "preparation.nsi"
        include = self.directory / "preparation-include.nsh"
        log = self.directory / "preparation-log.txt"
        # Capture the same helper output shown in the installer's details pane, so
        # silent regression failures retain their real PowerShell error message.
        content = INCLUDE.read_text(encoding="utf-8").replace("nsExec::ExecToLog /TIMEOUT=60000",
                                                            "nsExec::ExecToStack /TIMEOUT=60000", 1)
        marker = "  Pop $0\n  System::Call 'Kernel32::SetEnvironmentVariable(t \"FREETYPE_INSTALL_DIR\", t \"\")'"
        content = content.replace(marker, '  Pop $0\n  Pop $1\n' +
                                  f'  FileOpen $2 "{log}" w\n  FileWriteUTF16LE $2 "$0$\\r$\\n$1"\n  FileClose $2\n' +
                                  marker.split("\n", 1)[1])
        include.write_text(content, encoding="utf-8-sig")
        shutil.copyfile(INCLUDE.with_name("prepare-install.ps1"), include.with_name("prepare-install.ps1"))
        shutil.copyfile(INCLUDE.with_name("cleanup-install.ps1"), include.with_name("cleanup-install.ps1"))
        script.write_text(
            'Unicode true\nRequestExecutionLevel user\nSilentInstall silent\n'
            f'Name "FreeType preparation test"\nOutFile "{installer}"\nInstallDir "{install_dir or self.target}"\n'
            '!include "MUI2.nsh"\n'
            '!insertmacro MUI_LANGUAGE "SimpChinese"\n'
            '!insertmacro MUI_LANGUAGE "TradChinese"\n'
            '!insertmacro MUI_LANGUAGE "English"\n'
            + self.guard_define + f'!include "{include}"\n'
            'Section\n' + extra +
            'FileOpen $0 "$INSTDIR\\prepared.txt" w\nFileWrite $0 "ready"\nFileClose $0\nSectionEnd\n',
            encoding="utf-8-sig",
        )
        compiled = subprocess.run([str(self.compiler), "-V2", str(script)], capture_output=True, timeout=30)
        self.assertEqual(compiled.returncode, 0, compiled.stdout + compiled.stderr)
        result = subprocess.run(
            [str(installer), "/S"], capture_output=True, timeout=45,
            env={**os.environ, **(environment or {})},
        )
        diagnostic = log.read_text(encoding="utf-16-le") if log.exists() else str(result.stdout + result.stderr)
        self.assertEqual(result.returncode, expected_code, diagnostic)
        if expected_code == 0:
            self.assertEqual((self.target / "prepared.txt").read_text(), "ready")
        else:
            self.assertFalse((self.target / "prepared.txt").exists(), "Failure must stop installation")

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

    def installation_alias(self):
        alias = self.directory / "安装目录联接 & alias"
        subprocess.run(
            ["powershell", "-NoProfile", "-NonInteractive", "-Command",
             "New-Item -ItemType Junction -Path $env:FT_TEST_ALIAS -Target $env:FT_TEST_TARGET | Out-Null"],
            env={**os.environ, "FT_TEST_ALIAS": str(alias), "FT_TEST_TARGET": str(self.target)},
            check=True, capture_output=True, timeout=10,
        )
        return alias

    def test_junction_installation_stops_the_same_physical_executable(self):
        tray = self.start(self.target / "FreeType.exe")
        server = self.start(self.target / "bin" / "FreeTypeServer.exe")
        unrelated = self.start(self.directory / "other" / "FreeType.exe")
        alias = self.installation_alias()
        self.assertTrue(os.path.samefile(alias / "FreeType.exe", self.target / "FreeType.exe"))
        self.run_preparation(install_dir=alias)
        self.assertIsNotNone(tray.poll(), "A junction must not hide the tray from preparation")
        self.assertIsNotNone(server.poll(), "A junction must not hide the server from preparation")
        self.assertIsNone(unrelated.poll(), "A different installation must stay running")
        self.assertEqual((self.target / "bin" / "stop-requested.txt").read_text(), "requested")

    def test_limited_query_server_is_stopped_without_reading_its_modules(self):
        server = self.start(self.target / "bin" / "FreeTypeServer.exe", "--limited-query")
        ready = self.target / "bin" / "limited-ready.txt"
        deadline = time.monotonic() + 5
        while not ready.exists() and time.monotonic() < deadline:
            self.assertIsNone(server.poll(), "The restricted fixture exited before becoming ready")
            time.sleep(0.02)
        self.assertTrue(ready.exists())
        query = subprocess.run(
            ["powershell", "-NoProfile", "-NonInteractive", "-Command",
             "(Get-CimInstance Win32_Process -Filter ('ProcessId=' + $env:FT_TEST_PID)).ExecutablePath"],
            env={**os.environ, "FT_TEST_PID": str(server.pid)},
            check=True, capture_output=True, timeout=10,
        )
        self.assertFalse(query.stdout.strip(), "The fixture must reproduce the unavailable CIM path")
        self.run_preparation(install_dir=self.installation_alias())
        self.assertEqual(server.poll(), 0, "The service must exit normally after saving, without forced termination")
        self.assertEqual((self.target / "bin" / "stop-requested.txt").read_text(), "requested")

    def test_service_that_does_not_exit_blocks_copying_without_being_killed(self):
        server = self.start(self.target / "bin" / "FreeTypeServer.exe", "--ignore-stop")
        self.run_preparation(expected_code=2)
        self.assertIsNone(server.poll(), "An unconfirmed shutdown must not force-kill a service with possible unsaved data")
        self.assertEqual((self.target / "bin" / "stop-requested.txt").read_text(), "requested")

    def test_locked_executable_stops_preparation_before_any_copy(self):
        import ctypes
        from ctypes import wintypes

        server = self.target / "bin" / "FreeTypeServer.exe"
        server.parent.mkdir()
        shutil.copyfile(self.fixture, server)
        original = server.read_bytes()
        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel.CreateFileW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, ctypes.c_void_p,
                                      wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE]
        kernel.CreateFileW.restype = wintypes.HANDLE
        kernel.CloseHandle.argtypes = [wintypes.HANDLE]
        handle = kernel.CreateFileW(str(server), 0x80000000, 1, None, 3, 0, None)
        self.assertNotEqual(handle, wintypes.HANDLE(-1).value)
        try:
            self.run_preparation(expected_code=2)
            self.assertEqual(server.read_bytes(), original, "Failed preparation must preserve the installed executable")
        finally:
            kernel.CloseHandle(handle)

    def test_enable_failure_stops_installation_before_reporting_success(self):
        (self.target / "bin").mkdir()
        shutil.copyfile(self.fixture, self.target / "bin" / "FreeTypeServer.exe")
        self.run_preparation(
            "Call FreeTypeEnableForUser\n", expected_code=4,
            environment={"FREETYPE_TEST_ENABLE_EXIT": "23"},
        )

    def test_enable_success_allows_installation_to_continue(self):
        (self.target / "bin").mkdir()
        shutil.copyfile(self.fixture, self.target / "bin" / "FreeTypeServer.exe")
        self.run_preparation("Call FreeTypeEnableForUser\n")

    def test_uninstall_preparation_preserves_other_installations(self):
        installer = self.directory / "make-uninstaller.exe"
        uninstaller = self.directory / "uninstall.exe"
        script = self.directory / "uninstall.nsi"
        script.write_text(
            'Unicode true\nRequestExecutionLevel user\nSilentInstall silent\nSilentUnInstall silent\n'
            f'Name "FreeType uninstall preparation test"\nOutFile "{installer}"\nInstallDir "{self.target}"\n'
            '!include "MUI2.nsh"\n'
            '!insertmacro MUI_LANGUAGE "SimpChinese"\n'
            '!insertmacro MUI_LANGUAGE "TradChinese"\n'
            '!insertmacro MUI_LANGUAGE "English"\n'
            + self.guard_define + f'!include "{INCLUDE}"\n'
            f'Section\nWriteUninstaller "{uninstaller}"\nSectionEnd\n'
            'Section "Uninstall"\nCall un.FreeTypePrepare\n'
            'FileOpen $0 "$INSTDIR\\uninstall-prepared.txt" w\nFileWrite $0 "ready"\nFileClose $0\nSectionEnd\n',
            encoding="utf-8-sig",
        )
        compiled = subprocess.run([str(self.compiler), "-V2", str(script)], capture_output=True, timeout=30)
        self.assertEqual(compiled.returncode, 0, compiled.stdout + compiled.stderr)
        subprocess.run([str(installer), "/S"], check=True, capture_output=True, timeout=45)
        tray = self.start(self.target / "FreeType.exe")
        server = self.start(self.target / "bin" / "FreeTypeServer.exe")
        unrelated = self.start(self.directory / "other" / "FreeType.exe")
        # NSIS requires _?= last and unquoted; this is a process command line, not a shell.
        result = subprocess.run(
            f'"{uninstaller}" /S _?={self.target}', capture_output=True, timeout=45,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual((self.target / "uninstall-prepared.txt").read_text(), "ready")
        self.assertIsNotNone(tray.poll())
        self.assertIsNotNone(server.poll())
        self.assertIsNone(unrelated.poll(), "Uninstall must not kill another FreeType installation")

    def test_install_completion_marks_automatic_and_finish_page_launches(self):
        shutil.copyfile(self.fixture, self.target / "FreeType.exe")
        (self.target / "bin").mkdir()
        shutil.copyfile(self.fixture, self.target / "bin/FreeTypeServer.exe")
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
        self.assertEqual((self.target / "bin/setup-calls.txt").read_text(), "--start-and-check\n")

    def test_new_service_failure_does_not_launch_or_report_success(self):
        shutil.copyfile(self.fixture, self.target / "FreeType.exe")
        (self.target / "bin").mkdir()
        shutil.copyfile(self.fixture, self.target / "bin/FreeTypeServer.exe")
        pending = self.directory / "pending.json"
        pending.write_text("pending", encoding="utf-8")
        self.run_preparation('Call FreeTypeLaunchAfterInstall\n', expected_code=8,
                             environment={"FREETYPE_TEST_START_EXIT": "23"})
        self.assertFalse((self.target / "after-install.txt").exists())
        self.assertEqual(pending.read_text(encoding="utf-8"), "pending")

    def run_uninstall(self, expected_code, expected_calls, *, cancelled=False, stale_marker=False, environment=None):
        import winreg

        server = self.target / "bin" / "FreeTypeServer.exe"
        server.parent.mkdir()
        shutil.copyfile(self.fixture, server)
        # Settings, learned words, an imported dictionary, a downloaded update, logs and caches.
        user_files = [self.user_data / "config.json", self.user_data / "user.tsv",
                      self.user_data / "dicts" / "词库.txt", self.user_data / "updates" / "FreeType-Update-Setup.exe",
                      self.local_data / "logs" / "tsf.log", self.local_data / "cache" / "lexicon.bin"]
        for file in user_files:
            file.parent.mkdir(parents=True, exist_ok=True)
            file.write_text("retained", encoding="utf-8")
        # A neighbour in the same parent folder belongs to another application.
        neighbour = self.user_data.parent / "Other" / "settings.json"
        neighbour.parent.mkdir()
        neighbour.write_text("other", encoding="utf-8")
        registry = "Software\\FreeTypeInstallerTests-" + self.directory.name
        include = self.directory / "installer.nsh"
        content = INCLUDE.read_text(encoding="utf-8").replace(
            "Software\\Microsoft\\Windows\\CurrentVersion\\Run", registry)

        # Keep the real macro, replacing only elevation and the user's startup key.
        def without_elevation(match):
            if "--unregister-tsf" not in match[4]:
                return match[0]
            if cancelled:
                return match[1] + "SetErrors"
            return match[1] + f"ExecWait '{match[2]} {match[4]}' $0"

        content = re.sub(r'^(\s*)ExecShellWait "runas" ("[^"]+") ([\'\"])(.*?)\3 SW_HIDE$',
                         without_elevation, content, flags=re.M)
        self.assertNotRegex(content, r'ExecShellWait[^\n]*--unregister-tsf')
        include.write_text(content, encoding="utf-8-sig")
        shutil.copyfile(INCLUDE.with_name("prepare-install.ps1"), include.with_name("prepare-install.ps1"))
        shutil.copyfile(INCLUDE.with_name("cleanup-install.ps1"), include.with_name("cleanup-install.ps1"))
        installer = self.directory / "make-uninstaller.exe"
        uninstaller = self.directory / "uninstall.exe"
        script = self.directory / "uninstall.nsi"
        stale = ('InitPluginsDir\nFileOpen $0 "$PLUGINSDIR\\freetype-unregister.ok" w\n'
                 'FileWrite $0 "old"\nFileClose $0\n') if stale_marker else ""
        script.write_text(
            'Unicode true\nRequestExecutionLevel user\nSilentInstall silent\nSilentUnInstall silent\n'
            f'Name "FreeType uninstall test"\nOutFile "{installer}"\nInstallDir "{self.target}"\n'
            '!include "MUI2.nsh"\n'
            '!insertmacro MUI_LANGUAGE "SimpChinese"\n'
            '!insertmacro MUI_LANGUAGE "TradChinese"\n'
            '!insertmacro MUI_LANGUAGE "English"\n'
            + self.guard_define + f'!include "{include}"\nSection\nWriteUninstaller "{uninstaller}"\nSectionEnd\n'
            'Section "Uninstall"\n' + stale + '!insertmacro customUnInstall\n'
            'Delete "$INSTDIR\\bin\\FreeTypeServer.exe"\n'
            'FileOpen $0 "$INSTDIR\\uninstalled.txt" w\nFileWrite $0 "done"\nFileClose $0\nSectionEnd\n',
            encoding="utf-8-sig",
        )
        compiled = subprocess.run([str(self.compiler), "-V2", str(script)], capture_output=True, timeout=30)
        self.assertEqual(compiled.returncode, 0, compiled.stdout + compiled.stderr)
        subprocess.run([str(installer), "/S"], check=True, capture_output=True, timeout=45)
        with winreg.CreateKeyEx(winreg.HKEY_CURRENT_USER, registry, 0,
                                winreg.KEY_WRITE | winreg.KEY_WOW64_32KEY) as key:
            winreg.SetValueEx(key, "FreeType", 0, winreg.REG_SZ, "startup")
        try:
            result = subprocess.run(f'"{uninstaller}" /S _?={self.target}', capture_output=True, timeout=45,
                                    env={**os.environ, **(environment or {})})
            self.assertEqual(result.returncode, expected_code, result.stdout + result.stderr)
            self.assertEqual((server.parent / "setup-calls.txt").read_text().splitlines(), expected_calls)
            self.assertEqual(server.exists(), expected_code != 0, "Failure must keep the program for repair")
            self.assertEqual((self.target / "uninstalled.txt").exists(), expected_code == 0)
            if expected_code == 0:
                self.assertFalse(self.user_data.exists(), "Uninstall must remove settings and learned data")
                self.assertFalse(self.local_data.exists(), "Uninstall must remove logs and caches")
            else:
                for file in user_files:
                    self.assertEqual(file.read_text(encoding="utf-8"), "retained",
                                     "A failed uninstall must keep user data for repair")
            self.assertEqual(neighbour.read_text(encoding="utf-8"), "other")
            with winreg.OpenKey(winreg.HKEY_CURRENT_USER, registry, 0,
                                winreg.KEY_READ | winreg.KEY_WOW64_32KEY) as key:
                if expected_code == 0:
                    with self.assertRaises(FileNotFoundError):
                        winreg.QueryValueEx(key, "FreeType")
                else:
                    self.assertEqual(winreg.QueryValueEx(key, "FreeType")[0], "startup")
        finally:
            winreg.DeleteKeyEx(winreg.HKEY_CURRENT_USER, registry, winreg.KEY_WOW64_32KEY)

    def test_uninstall_disable_failure_preserves_installation(self):
        self.run_uninstall(5, ["--disable"], environment={"FREETYPE_TEST_DISABLE_EXIT": "23"})

    def test_uninstall_cancelled_uac_preserves_installation_and_reenables_user(self):
        self.run_uninstall(6, ["--disable", "--enable"], cancelled=True)

    def test_uninstall_failed_unregister_preserves_installation(self):
        self.run_uninstall(6, ["--disable", "--unregister-tsf", "--enable"],
                           environment={"FREETYPE_TEST_UNREGISTER_EXIT": "23"})

    def test_uninstall_failed_restore_is_reported(self):
        self.run_uninstall(7, ["--disable", "--enable"], cancelled=True,
                           environment={"FREETYPE_TEST_ENABLE_EXIT": "23"})

    def test_uninstall_ignores_stale_success_marker(self):
        self.run_uninstall(6, ["--disable", "--enable"], cancelled=True, stale_marker=True)

    def test_uninstall_success_allows_cleanup(self):
        self.run_uninstall(0, ["--disable", "--unregister-tsf"])


if __name__ == "__main__":
    unittest.main()
