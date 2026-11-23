# Run after successful replacement, using manifests captured before NSIS copies new files.
# Only recycle unchanged old payload files; custom dictionaries and edited source copies survive.
$ErrorActionPreference = 'Stop'
$OutputEncoding = [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
# NSIS starts Windows PowerShell with an inherited module path; load its own hash/JSON cmdlets explicitly.
Import-Module ([IO.Path]::Combine($PSHOME, 'Modules/Microsoft.PowerShell.Utility/Microsoft.PowerShell.Utility.psd1')) -ErrorAction Stop
try {
    if ([string]::IsNullOrWhiteSpace($env:FREETYPE_INSTALL_DIR) -or
        [string]::IsNullOrWhiteSpace($env:FREETYPE_PREVIOUS_MANIFESTS)) { throw 'Missing cleanup directories' }
    $InstallRoot = (Resolve-Path -LiteralPath $env:FREETYPE_INSTALL_DIR).Path
    $Previous = (Resolve-Path -LiteralPath $env:FREETYPE_PREVIOUS_MANIFESTS).Path
    Add-Type -AssemblyName Microsoft.VisualBasic
    function Recycle-Unchanged([string]$Relative, [string]$Expected) {
        if ($Expected -notmatch '^[0-9a-fA-F]{64}$') { throw 'Missing previous payload checksum' }
        $Target = [IO.Path]::GetFullPath((Join-Path $InstallRoot $Relative))
        if (-not $Target.StartsWith(($InstallRoot.TrimEnd('\') + '\'), [StringComparison]::OrdinalIgnoreCase)) {
            throw 'Previous payload path escapes the installation directory'
        }
        if (-not (Test-Path -LiteralPath $Target -PathType Leaf)) { return }
        $Entry = Get-Item -LiteralPath $Target
        while ($Entry.FullName -ne $InstallRoot) {
            if ($Entry.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Payload cleanup refuses linked paths' }
            $Entry = Get-Item -LiteralPath (Split-Path -Parent $Entry.FullName)
        }
        if ((Get-FileHash -LiteralPath $Target -Algorithm SHA256).Hash -ne $Expected) { return }
        [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteFile($Target,
            [Microsoft.VisualBasic.FileIO.UIOption]::OnlyErrorDialogs,
            [Microsoft.VisualBasic.FileIO.RecycleOption]::SendToRecycleBin,
            [Microsoft.VisualBasic.FileIO.UICancelOption]::ThrowException)
    }
    $NoticePath = Join-Path $Previous 'third-party.json'
    if (Test-Path -LiteralPath $NoticePath -PathType Leaf) {
        $Notice = Get-Content -LiteralPath $NoticePath -Raw -Encoding UTF8 | ConvertFrom-Json
        foreach ($Component in $Notice.components) {
            foreach ($Relative in $Component.sourceFiles) {
                if ($Relative -notmatch '^[^:]+/source/[^:]+$') { throw 'Invalid old source path' }
                Recycle-Unchanged ('third-party/' + $Relative) $Notice.files.$Relative
            }
        }
    }
    $DataPath = Join-Path $Previous 'data.json'
    if (Test-Path -LiteralPath $DataPath -PathType Leaf) {
        $Data = Get-Content -LiteralPath $DataPath -Raw -Encoding UTF8 | ConvertFrom-Json
        if ($Data.files.'lexicon.tsv') { Recycle-Unchanged 'data/lexicon.tsv' $Data.files.'lexicon.tsv'.sha256 }
    }
    exit 0
} catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 1
}
