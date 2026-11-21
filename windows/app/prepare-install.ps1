$ErrorActionPreference = 'Stop'

try {
    if ([string]::IsNullOrWhiteSpace($env:FREETYPE_INSTALL_DIR)) {
        throw 'The installation directory is missing.'
    }
    # CIM can list an elevated process while leaving ExecutablePath empty. Query only
    # its image name, then resolve junctions for both the installed and running files.
    # NSIS runs this in its plugin directory, which contains a native System.dll.
    # Give the compiler the framework assembly's full path to avoid that name collision.
    Add-Type -ReferencedAssemblies ([ComponentModel.Win32Exception].Assembly.Location) -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Win32.SafeHandles;

public static class FreeTypeInstallFiles {
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern SafeProcessHandle OpenProcess(uint access, bool inherit, int pid);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool QueryFullProcessImageName(SafeProcessHandle process, uint flags, StringBuilder path, ref int size);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern uint WaitForSingleObject(SafeProcessHandle process, uint milliseconds);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern SafeFileHandle CreateFile(string path, uint access, uint share, IntPtr security, uint creation, uint flags, IntPtr template);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern uint GetFinalPathNameByHandle(SafeFileHandle file, StringBuilder path, uint size, uint flags);

    public static string ProcessPath(int pid) {
        using (var process = OpenProcess(0x101000, false, pid)) { // LIMITED_INFORMATION | SYNCHRONIZE
            if (process.IsInvalid) {
                int error = Marshal.GetLastWin32Error();
                if (error == 87) return null; // The enumerated process has already exited.
                throw new Win32Exception(error, "Cannot identify FreeType process " + pid);
            }
            int size = 32768;
            var path = new StringBuilder(size);
            if (!QueryFullProcessImageName(process, 0, path, ref size)) {
                int error = Marshal.GetLastWin32Error();
                // After --stop, an enumerated process can exit before this query;
                // Windows then denies its image name. Only a signalled handle proves exit.
                if (WaitForSingleObject(process, 0) == 0) return null;
                throw new Win32Exception(error, "Cannot query FreeType process " + pid);
            }
            return path.ToString();
        }
    }

    public static string FinalPath(string path) {
        // Metadata access only; allow the running image and other readers to keep their handles.
        using (var file = CreateFile(path, 0, 7, IntPtr.Zero, 3, 0, IntPtr.Zero)) {
            if (file.IsInvalid) throw new Win32Exception(Marshal.GetLastWin32Error(), "Cannot identify " + path);
            var resolved = new StringBuilder(32768);
            uint size = GetFinalPathNameByHandle(file, resolved, (uint)resolved.Capacity, 0);
            if (size == 0) throw new Win32Exception(Marshal.GetLastWin32Error());
            if (size >= resolved.Capacity) throw new InvalidOperationException("Installation path is too long");
            return resolved.ToString();
        }
    }
}
'@

    $Directory = [IO.Path]::GetFullPath($env:FREETYPE_INSTALL_DIR)
    $TrayPath = Join-Path $Directory 'FreeType.exe'
    $ServerPath = Join-Path $Directory 'bin\FreeTypeServer.exe'
    $Paths = @($TrayPath, $ServerPath)
    $FinalPaths = @{}
    foreach ($Path in $Paths) {
        if (Test-Path -LiteralPath $Path -PathType Leaf) {
            $FinalPaths[[FreeTypeInstallFiles]::FinalPath($Path)] = $Path
        }
    }

    function Get-InstallationProcesses {
        if ($FinalPaths.Count -eq 0) { return }
        foreach ($Entry in @(Get-CimInstance Win32_Process -Filter "Name='FreeType.exe' OR Name='FreeTypeServer.exe'")) {
            $Image = [FreeTypeInstallFiles]::ProcessPath($Entry.ProcessId)
            if (-not $Image) { continue }
            $Final = [FreeTypeInstallFiles]::FinalPath($Image)
            if ($FinalPaths.ContainsKey($Final)) {
                [pscustomobject]@{ ProcessId = $Entry.ProcessId; Path = $FinalPaths[$Final]; FinalPath = $Final }
            }
        }
    }

    $Running = @(Get-InstallationProcesses)

    foreach ($Path in $Paths) {
        $Targets = @($Running | Where-Object { $_.Path -eq $Path })
        if ($Targets.Count -eq 0) { continue }
        if ($Path -eq $ServerPath) {
            $Stop = Start-Process -FilePath $ServerPath -ArgumentList '--stop' -WindowStyle Hidden -PassThru
            try {
                if (-not $Stop.WaitForExit(30000)) { throw 'Timed out waiting for FreeType to save its data.' }
                if ($Stop.ExitCode -ne 0) { throw 'FreeType could not save its data. The update was stopped.' }
            } finally {
                $Stop.Dispose()
            }
        }
        foreach ($Target in $Targets) {
            try {
                $Process = [Diagnostics.Process]::GetProcessById($Target.ProcessId)
            } catch [ArgumentException] {
                continue
            }
            try {
                if ($Process.HasExited) { continue }
                $Image = [FreeTypeInstallFiles]::ProcessPath($Target.ProcessId)
                if (-not $Image -or [FreeTypeInstallFiles]::FinalPath($Image) -ne $Target.FinalPath) { continue }
                if ($Path -eq $ServerPath) {
                    # --stop asks the service to save and exit; its watchdog then exits too.
                    # Waiting requires no termination rights, even for an elevated service.
                    if ($Process.WaitForExit(5000)) { continue }
                    throw 'FreeType did not exit after saving. The update was stopped.'
                }
                try {
                    $Process.Kill()
                } catch {
                    if (-not $Process.HasExited) { throw }
                }
                if (-not $Process.WaitForExit(5000)) { throw "Could not stop $Path." }
            } finally {
                $Process.Dispose()
            }
        }
    }
    $Remaining = @(Get-InstallationProcesses)
    if ($Remaining.Count -gt 0) { throw 'FreeType is still running in the installation directory.' }
    # A different process can also hold an executable open. Verify write access before
    # NSIS copies any files; opening an existing file here does not change its contents.
    foreach ($Path in $Paths) {
        if (Test-Path -LiteralPath $Path -PathType Leaf) {
            $File = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Write,
                ([IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete))
            $File.Dispose()
        }
    }
    exit 0
} catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 1
}
