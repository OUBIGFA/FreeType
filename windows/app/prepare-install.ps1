$ErrorActionPreference = 'Stop'

try {
    if ([string]::IsNullOrWhiteSpace($env:FREETYPE_INSTALL_DIR)) {
        throw 'The installation directory is missing.'
    }
    $Directory = [IO.Path]::GetFullPath($env:FREETYPE_INSTALL_DIR)
    $TrayPath = Join-Path $Directory 'FreeType.exe'
    $ServerPath = Join-Path $Directory 'bin\FreeTypeServer.exe'
    $Paths = @($TrayPath, $ServerPath)
    $Running = @(Get-CimInstance Win32_Process -Filter "Name='FreeType.exe' OR Name='FreeTypeServer.exe'" |
        Where-Object { $Paths -contains $_.ExecutablePath })

    foreach ($Path in $Paths) {
        $Targets = @($Running | Where-Object { $_.ExecutablePath -eq $Path })
        if ($Targets.Count -eq 0) { continue }
        if ($Path -eq $ServerPath) {
            $Stop = Start-Process -FilePath $ServerPath -ArgumentList '--stop' -WindowStyle Hidden -PassThru
            if (-not $Stop.WaitForExit(5000)) {
                $Stop.Kill()
                if (-not $Stop.WaitForExit(5000)) { throw 'The shutdown request did not exit.' }
            }
            $Stop.Dispose()
        }
        foreach ($Target in $Targets) {
            try {
                $Process = [Diagnostics.Process]::GetProcessById($Target.ProcessId)
            } catch [ArgumentException] {
                continue
            }
            try {
                if ($Process.HasExited) { continue }
                if ($Process.MainModule.FileName -ne $Path) { continue }
                if ($Path -eq $ServerPath -and $Process.WaitForExit(5000)) { continue }
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
    $Remaining = @(Get-CimInstance Win32_Process -Filter "Name='FreeType.exe' OR Name='FreeTypeServer.exe'" |
        Where-Object { $Paths -contains $_.ExecutablePath })
    if ($Remaining.Count -gt 0) { throw 'FreeType is still running in the installation directory.' }
    exit 0
} catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 1
}
