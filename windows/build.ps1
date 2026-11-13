<#
.SYNOPSIS
  Build the FreeType fixed test installer.

.DESCRIPTION
  Runs the tests, builds the Rust server and TSF DLLs (x64 and x86), the mygo
  settings/tray app and its NSIS installer, then writes
    output/release/FreeType-0.0.1-Setup.exe
    output/release/FreeType-0.0.1-Setup.exe.sha256
    output/release/build-info.json

  Toolchains are taken from .local/toolchains when present (Rust via rustup,
  Go 1.27.3). Dictionary data must already be in .local/data (see
  docs/WINDOWS.md). Nothing is installed or registered on this machine.

.PARAMETER SkipTests
  Build without running the test suites.
#>
param([switch]$SkipTests)

$OutputEncoding = [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$env:PYTHONUTF8 = '1'
$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root

$Version = '0.0.1'
$Identifier = 'io.github.oubigfa.freetype'
$MygoVersion = 'v0.9.0'
$BuildNumber = Get-Date -Format 'yyyyMMddHHmmss'
$AppDir = Join-Path $Root 'windows\app'
$Resources = Join-Path $AppDir 'resources'
$Data = Join-Path $Root '.local\data'
$Release = Join-Path $Root 'output\release'
$Installer = Join-Path $Release "FreeType-$Version-Setup.exe"
$Scratch = Join-Path $Root "_temp\build-$BuildNumber"

function Step($text) { Write-Host "==> $text" -ForegroundColor Cyan }

function Invoke-Native {
    param([string]$File, [string[]]$Arguments, [string]$Where = $Root)
    Push-Location $Where
    try {
        & $File @Arguments
        if ($LASTEXITCODE -ne 0) { throw "$File $($Arguments -join ' ') failed with exit code $LASTEXITCODE" }
    } finally {
        Pop-Location
    }
}

function Write-Utf8([string]$Path, [string]$Text) {
    [IO.File]::WriteAllText($Path, $Text, (New-Object Text.UTF8Encoding $false))
}

function Sha256([string]$Path) { (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLowerInvariant() }

function Recycle-GeneratedDirectory([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) { return }
    $Resolved = (Resolve-Path -LiteralPath $Path).Path
    $Allowed = @($Resources, (Join-Path $AppDir 'build'), $Scratch)
    if ($Allowed -notcontains $Resolved -or ((Get-Item -LiteralPath $Resolved).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw "Not a generated directory owned by this build: $Resolved"
    }
    Add-Type -AssemblyName Microsoft.VisualBasic
    [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteDirectory(
        $Resolved,
        [Microsoft.VisualBasic.FileIO.UIOption]::OnlyErrorDialogs,
        [Microsoft.VisualBasic.FileIO.RecycleOption]::SendToRecycleBin,
        [Microsoft.VisualBasic.FileIO.UICancelOption]::ThrowException)
}

# Project-local toolchains (not tracked by Git).
$Toolchains = Join-Path $Root '.local\toolchains'
if (Test-Path (Join-Path $Toolchains 'cargo\bin\cargo.exe')) {
    $env:RUSTUP_HOME = Join-Path $Toolchains 'rustup'
    $env:CARGO_HOME = Join-Path $Toolchains 'cargo'
    $env:PATH = (Join-Path $Toolchains 'cargo\bin') + ';' + $env:PATH
}
$LocalGo = Join-Path $Toolchains 'go1.27.3'
if (Test-Path (Join-Path $LocalGo 'bin\go.exe')) {
    $env:GOROOT = $LocalGo
    $env:PATH = (Join-Path $LocalGo 'bin') + ';' + $env:PATH
    $env:GOPATH = Join-Path $Root '.local\gopath'
}
$env:GOTOOLCHAIN = 'local'
$env:GOFLAGS = '-modcacherw'

Step 'Checking inputs'
foreach ($name in 'lexicon.tsv', 'words.lm', 'english.txt', 'LICENSE', 'SOURCE.txt', 'VERSION') {
    if (-not (Test-Path (Join-Path $Data $name))) {
        throw "Missing .local\data\$name. Generate the dictionary data first (docs/WINDOWS.md)."
    }
}
$GoVersion = (& go version)
if ($GoVersion -notmatch 'go1\.27\.3') { throw "Go 1.27.3 is required for mygo $MygoVersion, found: $GoVersion" }
$GoMod = Get-Content -Raw (Join-Path $AppDir 'go.mod')
if ($GoMod -notmatch "github.com/egoist/mygo $([regex]::Escape($MygoVersion))") { throw "windows/app/go.mod must pin mygo $MygoVersion" }

New-Item -ItemType Directory -Force -Path $Scratch | Out-Null
$env:TEMP = $Scratch
$env:TMP = $Scratch
$env:FREETYPE_TEST_TEMP = $Scratch

if (-not $SkipTests) {
    Step 'Running tests'
    Invoke-Native cargo @('test', '-p', 'wowtype-core', '--no-default-features')
    Invoke-Native cargo @('test', '-p', 'freetype-protocol', '-p', 'freetype-server', '-p', 'freetype-tsf')
    Invoke-Native go @('vet', './...') $AppDir
    Invoke-Native go @('test', './...') $AppDir
}

Step 'Building Rust components'
Invoke-Native cargo @('build', '--release', '-p', 'freetype-server', '-p', 'freetype-tsf')
Invoke-Native rustup @('target', 'add', 'i686-pc-windows-msvc')
Invoke-Native cargo @('build', '--release', '-p', 'freetype-tsf', '--target', 'i686-pc-windows-msvc')

Step 'Staging app resources'
Recycle-GeneratedDirectory $Resources
$Bin = Join-Path $Resources 'windows-amd64\bin'
New-Item -ItemType Directory -Force -Path (Join-Path $Bin 'x86'), (Join-Path $Resources 'icons'), (Join-Path $Resources 'data') | Out-Null
$Icons = Join-Path $Root "_temp\build-$BuildNumber\icons"
Invoke-Native python @('-X', 'utf8', 'windows\tools\make_icons.py', 'windows\assets\freetype-logo.png', $Icons)
Copy-Item (Join-Path $Icons 'icon.png') (Join-Path $Resources 'icon.png')
Copy-Item (Join-Path $Icons 'tray.png'), (Join-Path $Icons 'tray-16.png'), (Join-Path $Icons 'logo.png'), (Join-Path $Icons 'freetype.ico') (Join-Path $Resources 'icons')
Copy-Item (Join-Path $Icons 'freetype.ico') $Bin
Copy-Item 'target\release\FreeTypeServer.exe', 'target\release\freetype_tsf.dll' $Bin
Copy-Item 'target\i686-pc-windows-msvc\release\freetype_tsf.dll' (Join-Path $Bin 'x86')
foreach ($name in 'lexicon.tsv', 'words.lm', 'english.txt', 'LICENSE', 'SOURCE.txt', 'VERSION') {
    Copy-Item (Join-Path $Data $name) (Join-Path $Resources 'data')
}
Copy-Item 'core\data\s2t.tsv' (Join-Path $Resources 'data')

Step "Building the mygo app and installer (mygo $MygoVersion)"
$AppBuild = Join-Path $AppDir 'build'
Recycle-GeneratedDirectory $AppBuild
Invoke-Native go @('tool', 'mygo', 'build') $AppDir
$Built = Get-ChildItem -Recurse -File $AppBuild -Filter "freetype-setup-$Version-*.exe" | Select-Object -First 1
if (-not $Built) { throw "mygo build produced no installer under $AppBuild" }

if (-not $SkipTests) {
    Step 'Testing installer preparation with disposable processes'
    Invoke-Native python @('-X', 'utf8', '-m', 'unittest', 'discover', '-s', 'tests/windows', '-v')
}

Step 'Recording the release'
New-Item -ItemType Directory -Force -Path $Release | Out-Null
Copy-Item $Built.FullName $Installer -Force
$InstallerHash = Sha256 $Installer
Write-Utf8 "$Installer.sha256" "$InstallerHash *FreeType-$Version-Setup.exe`n"

# Source snapshot: the tree Git would commit from the working directory now.
$TempIndex = Join-Path $Root "_temp\build-$BuildNumber\index"
$env:GIT_INDEX_FILE = $TempIndex
try {
    Invoke-Native git @('read-tree', 'HEAD')
    Invoke-Native git @('-c', 'core.safecrlf=false', 'add', '-A')
    $SourceTree = (& git write-tree).Trim()
} finally {
    Remove-Item Env:\GIT_INDEX_FILE
}
$Head = (& git rev-parse HEAD).Trim()
$RimeCommit = ''
$SourceText = Get-Content -Raw -Encoding UTF8 (Join-Path $Data 'SOURCE.txt')
if ($SourceText -match '([0-9a-f]{40})') { $RimeCommit = $Matches[1] }
$Pillow = (& python -c 'import PIL; print(PIL.__version__)').Trim()

$Info = [ordered]@{
    product        = 'FreeType'
    version        = $Version
    identifier     = $Identifier
    buildNumber    = $BuildNumber
    builtAt        = (Get-Date).ToString('o')
    source         = [ordered]@{ baseCommit = $Head; tree = $SourceTree }
    installer      = [ordered]@{ file = "FreeType-$Version-Setup.exe"; bytes = (Get-Item $Installer).Length; sha256 = $InstallerHash }
    toolchain      = [ordered]@{
        rustc  = (& rustc --version).Trim()
        cargo  = (& cargo --version).Trim()
        go     = $GoVersion.Trim()
        mygo   = $MygoVersion
        python = (& python --version).Trim()
        pillow = $Pillow
    }
    components     = [ordered]@{
        'bin/FreeTypeServer.exe'       = Sha256 (Join-Path $Bin 'FreeTypeServer.exe')
        'bin/freetype_tsf.dll'         = Sha256 (Join-Path $Bin 'freetype_tsf.dll')
        'bin/x86/freetype_tsf.dll'     = Sha256 (Join-Path $Bin 'x86\freetype_tsf.dll')
        'bin/freetype.ico'             = Sha256 (Join-Path $Bin 'freetype.ico')
    }
    data           = [ordered]@{
        rimeIceCommit  = $RimeCommit
        dataVersion    = (Get-Content -Raw (Join-Path $Data 'VERSION')).Trim()
        lexiconSha256  = Sha256 (Join-Path $Data 'lexicon.tsv')
        wordsLmSha256  = Sha256 (Join-Path $Data 'words.lm')
        englishSha256  = Sha256 (Join-Path $Data 'english.txt')
    }
}
Write-Utf8 (Join-Path $Release 'build-info.json') (($Info | ConvertTo-Json -Depth 5) + "`n")
Recycle-GeneratedDirectory $Scratch

Write-Host ''
Write-Host "Installer : $Installer"
Write-Host "SHA-256   : $InstallerHash"
Write-Host "Build     : $BuildNumber (source tree $SourceTree)"
