<#
.SYNOPSIS
  Build a FreeType test installer, release candidate or approved release tag.

.DESCRIPTION
  Runs the tests, builds the Rust server and TSF DLLs (x64 and x86), the mygo
  settings/tray app and its NSIS installer, then writes
    output/release/FreeType-0.0.1-Setup.exe
    output/release/FreeType-0.0.1-Setup.exe.sha256
    output/release/build-info.json

  Toolchains are taken from .local/toolchains when present (Rust via rustup,
  Go 1.27.4). Dictionary data must already be in .local/data (see
  docs/WINDOWS.md). Nothing is installed or registered on this machine.

.PARAMETER SkipTests
  Build without running the test suites (not allowed with ReleaseTag).

.PARAMETER Version
  Product version. 0.0.1 keeps the fixed test output; other versions use
  output/candidates/<version> and a temporary copy of the app configuration.

.PARAMETER ReleaseTag
  Build an existing matching tag from a clean checkout into output/releases/<version>.
  This does not create tags, publish a release or install the application.
#>
[CmdletBinding()]
param(
    [ValidatePattern('^(0|[1-9][0-9]{0,4})\.(0|[1-9][0-9]{0,4})\.(0|[1-9][0-9]{0,4})$')]
    [string]$Version = '0.0.1',
    [string]$ReleaseTag = '',
    [switch]$SkipTests
)

$OutputEncoding = [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$env:PYTHONUTF8 = '1'
$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root

if (@($Version.Split('.') | Where-Object { [int]$_ -gt 65535 }).Count -gt 0) {
    throw 'Each Windows version component must be between 0 and 65535'
}
if ($ReleaseTag -and ($ReleaseTag -notin @($Version, "freetype-$Version") -or $Version -eq '0.0.1' -or $SkipTests)) {
    throw 'ReleaseTag must match a non-test Version, and release builds must run all tests'
}
$Identifier = 'io.github.oubigfa.freetype'
$MygoVersion = 'v0.9.0'
$SelfUpdateVersion = 'v1.7.1'
$BuildNumber = Get-Date -Format 'yyyyMMddHHmmss'
$AppDir = Join-Path $Root 'windows\app'
$SourceAppDir = $AppDir
$Resources = Join-Path $AppDir 'resources'
$Data = Join-Path $Root '.local\data'
$Release = Join-Path $Root 'output\release'
$BuildKind = 'fixed-test'
if ($Version -ne '0.0.1') {
    $BuildKind = 'candidate'
    $Release = Join-Path $Root "output\candidates\$Version"
}
if ($ReleaseTag) {
    $BuildKind = 'release'
    $Release = Join-Path $Root "output\releases\$Version"
}
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

function Find-NsisCompiler {
    # Follow mygo v0.9.0's selection so the tests and record use the actual compiler.
    $Command = Get-Command makensis -CommandType Application -ErrorAction SilentlyContinue
    if ($Command) { return $Command.Source }
    $Candidates = @(
        (Join-Path ${env:ProgramFiles(x86)} 'NSIS\makensis.exe'),
        (Join-Path $env:ProgramFiles 'NSIS\makensis.exe'),
        (Join-Path $env:LOCALAPPDATA 'mygo\nsis-3.13\Bin\makensis.exe'),
        (Join-Path $env:LOCALAPPDATA 'mygo\nsis-3.13\makensis.exe')
    )
    foreach ($Candidate in $Candidates) {
        if (Test-Path -LiteralPath $Candidate -PathType Leaf) { return $Candidate }
    }
    throw 'Cannot locate the NSIS compiler used by mygo'
}

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
$LocalGo = Join-Path $Toolchains 'go1.27.4'
if (Test-Path (Join-Path $LocalGo 'bin\go.exe')) {
    $env:GOROOT = $LocalGo
    $env:PATH = (Join-Path $LocalGo 'bin') + ';' + $env:PATH
    $env:GOPATH = Join-Path $Root '.local\gopath'
}
$env:GOTOOLCHAIN = 'local'
$env:GOFLAGS = '-modcacherw -mod=readonly'

Step 'Checking inputs'
if ($ReleaseTag) {
    $TagCommit = Invoke-Native git @('rev-parse', '--verify', "refs/tags/$ReleaseTag^{commit}")
    $CurrentCommit = Invoke-Native git @('rev-parse', 'HEAD')
    if ($TagCommit -ne $CurrentCommit -or (Invoke-Native git @('status', '--porcelain', '--untracked-files=all'))) {
        throw 'Release builds require a clean checkout of the exact approved tag'
    }
}
Invoke-Native python @('-X', 'utf8', 'tools/check_repo.py')
foreach ($name in 'lexicon.tsv', 'words.lm', 'english.txt', 'LICENSE', 'SOURCE.txt', 'VERSION', 'MANIFEST.json', 'WANXIANG-LICENSE-SCOPE.md', 'WANXIANG-NOTICE.md') {
    if (-not (Test-Path (Join-Path $Data $name))) {
        throw "Missing .local\data\$name. Run python -X utf8 tools/make_default_data.py first (docs/WINDOWS.md)."
    }
}
Invoke-Native python @('-X', 'utf8', 'tools/make_default_data.py', '--check')
$GoVersion = (& go version)
if ($GoVersion -notmatch '^go version go1\.27\.4 windows/amd64$') { throw "Go 1.27.4 windows/amd64 is required, found: $GoVersion" }
$GoMod = Get-Content -Raw (Join-Path $AppDir 'go.mod')
if ($GoMod -notmatch "github.com/egoist/mygo $([regex]::Escape($MygoVersion))") { throw "windows/app/go.mod must pin mygo $MygoVersion" }
if ($GoMod -notmatch "github.com/creativeprojects/go-selfupdate $([regex]::Escape($SelfUpdateVersion))") { throw "windows/app/go.mod must pin go-selfupdate $SelfUpdateVersion" }

New-Item -ItemType Directory -Force -Path $Scratch | Out-Null
$env:TEMP = $Scratch
$env:TMP = $Scratch
$env:FREETYPE_TEST_TEMP = $Scratch
$Source = Invoke-Native python @('-X', 'utf8', 'tools/capture_build_source.py', '--index', (Join-Path $Scratch 'source-before.index')) | ConvertFrom-Json
$SourceTree = $Source.tree
$Head = $Source.baseCommit

if ($Version -ne '0.0.1') {
    Step "Preparing isolated app configuration for $Version"
    $AppDir = Join-Path $Scratch 'app'
    foreach ($Tracked in (Invoke-Native git @('-c', 'core.quotepath=false', 'ls-files', '--', 'windows/app'))) {
        $Relative = $Tracked.Substring('windows/app/'.Length)
        $Destination = Join-Path $AppDir $Relative
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Destination) | Out-Null
        Copy-Item -LiteralPath (Join-Path $Root $Tracked) -Destination $Destination
    }
    $ConfigPath = Join-Path $AppDir 'mygo.json'
    $Config = Get-Content -Raw -Encoding UTF8 $ConfigPath | ConvertFrom-Json
    $Config.version = $Version
    Write-Utf8 $ConfigPath (($Config | ConvertTo-Json -Depth 8) + "`n")
    $IncludePath = Join-Path $AppDir 'installer.nsh'
    Write-Utf8 $IncludePath ("!define FREETYPE_VERSION $Version`n" + (Get-Content -Raw -Encoding UTF8 $IncludePath))
    $Resources = Join-Path $AppDir 'resources'
}
$env:FREETYPE_BUILD_VERSION = $Version
$env:FREETYPE_BUILD_APP = $AppDir

if (-not $SkipTests) {
    Step 'Running tests'
    Invoke-Native python @('-X', 'utf8', '-m', 'unittest', 'discover', '-s', 'tests/repository', '-v')
    Invoke-Native python @('-X', 'utf8', '-m', 'unittest', 'discover', '-s', 'tests/data', '-v')
    Invoke-Native cargo @('test', '--locked', '-p', 'wowtype-core', '--no-default-features')
    Invoke-Native cargo @('test', '--locked', '-p', 'freetype-protocol', '-p', 'freetype-server', '-p', 'freetype-tsf')
    Invoke-Native go @('vet', './...') $AppDir
    Invoke-Native go @('test', './...') $AppDir
}

Step 'Building Rust components'
Invoke-Native cargo @('build', '--locked', '--release', '-p', 'freetype-server', '-p', 'freetype-tsf')
Invoke-Native rustup @('target', 'add', 'i686-pc-windows-msvc')
Invoke-Native cargo @('build', '--locked', '--release', '-p', 'freetype-tsf', '--target', 'i686-pc-windows-msvc')

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
foreach ($name in 'lexicon.tsv', 'words.lm', 'english.txt', 'LICENSE', 'SOURCE.txt', 'VERSION', 'MANIFEST.json', 'WANXIANG-LICENSE-SCOPE.md', 'WANXIANG-NOTICE.md') {
    Copy-Item (Join-Path $Data $name) (Join-Path $Resources 'data')
}
Copy-Item 'core\data\s2t.tsv' (Join-Path $Resources 'data')

Step 'Collecting third-party notices and corresponding source'
Invoke-Native python @('-X', 'utf8', 'tools/make_third_party_notices.py', '--out', (Join-Path $Resources 'third-party'))
Copy-Item (Join-Path $Root 'third_party\NOTICE.md') (Join-Path $Resources 'THIRD-PARTY-NOTICES.txt')

Step "Building the mygo app and installer (mygo $MygoVersion)"
$AppBuild = Join-Path $AppDir 'build'
Recycle-GeneratedDirectory $AppBuild
Invoke-Native go @('tool', 'mygo', 'build') $AppDir
$Installers = @(Get-ChildItem -Recurse -File $AppBuild -Filter "freetype-setup-$Version-*.exe")
if ($Installers.Count -ne 1) { throw "mygo must produce exactly one $Version installer under $AppBuild" }
$Built = $Installers[0]
$NsisCompiler = Find-NsisCompiler
$NsisVersion = (Invoke-Native $NsisCompiler @('/VERSION')).Trim()
$NoticeInputs = Get-Content -Raw -Encoding UTF8 (Join-Path $Root 'third_party\manifests\notice-inputs.json') | ConvertFrom-Json
if ($NsisVersion -ne "v$($NoticeInputs.nsisSource.version)") {
    throw "NSIS $NsisVersion does not match the bundled source and notices ($($NoticeInputs.nsisSource.version))"
}
$env:FREETYPE_MAKENSIS = $NsisCompiler

Step 'Checking Windows version resources and bundled notices'
$Stage = Join-Path $AppBuild 'windows-amd64'
$ArtifactPaths = [ordered]@{
    'FreeType.exe' = Join-Path $Stage 'FreeType.exe'
    'bin/FreeTypeServer.exe' = Join-Path $Stage 'bin\FreeTypeServer.exe'
    'bin/freetype_tsf.dll' = Join-Path $Stage 'bin\freetype_tsf.dll'
    'bin/x86/freetype_tsf.dll' = Join-Path $Stage 'bin\x86\freetype_tsf.dll'
    "FreeType-$Version-Setup.exe" = $Built.FullName
}
$VersionResources = [ordered]@{}
foreach ($Artifact in $ArtifactPaths.GetEnumerator()) {
    $Metadata = (Get-Item -LiteralPath $Artifact.Value).VersionInfo
    if ($Metadata.ProductName -ne 'FreeType' -or $Metadata.ProductVersion -ne $Version -or
        $Metadata.FileVersion -ne $Version -or -not $Metadata.FileDescription -or
        $Metadata.OriginalFilename -ne [IO.Path]::GetFileName($Artifact.Key)) {
        throw "Missing or inconsistent Windows version resource: $($Artifact.Key)"
    }
    $VersionResources[$Artifact.Key] = [ordered]@{
        product = $Metadata.ProductName
        productVersion = $Metadata.ProductVersion
        fileVersion = $Metadata.FileVersion
        description = $Metadata.FileDescription
        originalFilename = $Metadata.OriginalFilename
    }
}
$NoticeDirectory = Join-Path $Stage 'third-party'
$NoticeManifestPath = Join-Path $NoticeDirectory 'manifest.json'
$NoticeManifest = Get-Content -Raw -Encoding UTF8 $NoticeManifestPath | ConvertFrom-Json
foreach ($NoticeFile in $NoticeManifest.files.PSObject.Properties) {
    if ((Sha256 (Join-Path $NoticeDirectory $NoticeFile.Name)) -ne $NoticeFile.Value) {
        throw "Bundled third-party file hash mismatch: $($NoticeFile.Name)"
    }
}
$NoticeHash = Sha256 (Join-Path $Stage 'THIRD-PARTY-NOTICES.txt')
if ($NoticeHash -ne (Sha256 (Join-Path $Root 'third_party\NOTICE.md'))) { throw 'Bundled notice text does not match its source' }
if ($NoticeManifest.unresolved.Count -gt 0) {
    throw 'Unresolved third-party permissions prevent packaging this build'
}

if (-not $SkipTests) {
    Step 'Testing installer preparation with disposable processes'
    Invoke-Native python @('-X', 'utf8', '-m', 'unittest', 'discover', '-s', 'tests/windows', '-v')
}

Step 'Verifying the source snapshot has not changed'
Invoke-Native python @('-X', 'utf8', 'tools/capture_build_source.py', '--index', (Join-Path $Scratch 'source-after.index'), '--expect-tree', $SourceTree, '--expect-commit', $Head) | Out-Null

Step 'Recording the release'
New-Item -ItemType Directory -Force -Path $Release | Out-Null
Copy-Item $Built.FullName $Installer -Force
$InstallerHash = Sha256 $Installer
Write-Utf8 "$Installer.sha256" "$InstallerHash *FreeType-$Version-Setup.exe`n"

$DataManifest = Get-Content -Raw -Encoding UTF8 (Join-Path $Data 'MANIFEST.json') | ConvertFrom-Json
$Pillow = (& python -c 'import PIL; print(PIL.__version__)').Trim()

$Info = [ordered]@{
    product        = 'FreeType'
    version        = $Version
    identifier     = $Identifier
    buildNumber    = $BuildNumber
    buildKind      = $BuildKind
    releaseTag     = $ReleaseTag
    builtAt        = (Get-Date).ToString('o')
    source         = [ordered]@{ baseCommit = $Head; tree = $SourceTree }
    configuration  = [ordered]@{
        sourceAppSha256 = Sha256 (Join-Path $SourceAppDir 'mygo.json')
        builtAppSha256 = Sha256 (Join-Path $AppDir 'mygo.json')
        productVersion = $Version
    }
    installer      = [ordered]@{ file = "FreeType-$Version-Setup.exe"; bytes = (Get-Item $Installer).Length; sha256 = $InstallerHash }
    toolchain      = [ordered]@{
        rustc  = (& rustc --version).Trim()
        cargo  = (& cargo --version).Trim()
        go     = $GoVersion.Trim()
        mygo   = $MygoVersion
        goSelfUpdate = $SelfUpdateVersion
        nsis = $NsisVersion
        nsisSha256 = Sha256 $NsisCompiler
        python = (& python --version).Trim()
        pillow = $Pillow
    }
    dependencies   = [ordered]@{
        cargoLockSha256 = Sha256 (Join-Path $Root 'Cargo.lock')
        goModSha256 = Sha256 (Join-Path $AppDir 'go.mod')
        goSumSha256 = Sha256 (Join-Path $AppDir 'go.sum')
    }
    versionResources = $VersionResources
    thirdParty     = [ordered]@{
        noticeSha256 = $NoticeHash
        manifestSha256 = Sha256 $NoticeManifestPath
        componentCount = $NoticeManifest.components.Count
        fileCount = @($NoticeManifest.files.PSObject.Properties).Count
        unresolved = @($NoticeManifest.unresolved)
    }
    components     = [ordered]@{
        'FreeType.exe'                 = Sha256 (Join-Path $AppBuild 'windows-amd64\FreeType.exe')
        'bin/FreeTypeServer.exe'       = Sha256 (Join-Path $Bin 'FreeTypeServer.exe')
        'bin/freetype_tsf.dll'         = Sha256 (Join-Path $Bin 'freetype_tsf.dll')
        'bin/x86/freetype_tsf.dll'     = Sha256 (Join-Path $Bin 'x86\freetype_tsf.dll')
        'bin/freetype.ico'             = Sha256 (Join-Path $Bin 'freetype.ico')
    }
    data           = [ordered]@{
        frostCommit    = $DataManifest.sources[0].revision
        wanxiangCommit = $DataManifest.sources[1].revision
        manifestSha256 = Sha256 (Join-Path $Data 'MANIFEST.json')
        counts         = $DataManifest.counts
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
