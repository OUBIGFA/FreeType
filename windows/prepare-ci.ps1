<#
.SYNOPSIS
  Prepare pinned tools inside an ephemeral GitHub Actions Windows workspace.
#>
[CmdletBinding()]
param()

$OutputEncoding = [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -ne 'true') { throw 'This preparation is only for a GitHub Actions runner' }
$Root = Split-Path -Parent $PSScriptRoot
$Toolchains = Join-Path $Root '.local/toolchains'
$RustVersion = '1.99.1'
$GoVersion = 'go version go1.27.4 windows/amd64'
$RustupSource = (Get-Command rustup -CommandType Application).Source

function Invoke-Native {
    param([string]$File, [string[]]$Arguments)
    & $File @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$File $($Arguments -join ' ') failed with exit code $LASTEXITCODE" }
}

$InstalledGo = (Invoke-Native go @('env', 'GOROOT')).Trim()
if ((Invoke-Native go @('version')).Trim() -ne $GoVersion) { throw 'The setup-go step must provide Go 1.27.4 windows/amd64' }
$LocalGo = Join-Path $Toolchains 'go1.27.4'
New-Item -ItemType Directory -Force -Path $Toolchains | Out-Null
if ($InstalledGo -ne $LocalGo) {
    if (Test-Path -LiteralPath $LocalGo) { throw 'Refusing to overwrite an existing project-local Go installation' }
    Copy-Item -LiteralPath $InstalledGo -Destination $LocalGo -Recurse
}
$env:GOROOT = $LocalGo
$env:GOPATH = Join-Path $Root '.local/gopath'
$env:RUSTUP_HOME = Join-Path $Toolchains 'rustup'
$env:CARGO_HOME = Join-Path $Toolchains 'cargo'
$CargoBin = Join-Path $env:CARGO_HOME 'bin'
New-Item -ItemType Directory -Force -Path $CargoBin | Out-Null
# Rustup proxies choose the project-local toolchain by their executable name.
foreach ($Proxy in @('rustup.exe', 'rustc.exe', 'cargo.exe')) {
    Copy-Item -LiteralPath $RustupSource -Destination (Join-Path $CargoBin $Proxy) -Force
}
$GoBin = Join-Path $LocalGo 'bin'
$env:PATH = "$CargoBin;$GoBin;$env:PATH"
Invoke-Native rustup @('set', 'default-host', 'x86_64-pc-windows-msvc')
Invoke-Native rustup @('toolchain', 'install', $RustVersion, '--profile', 'minimal', '--no-self-update')
Invoke-Native rustup @('default', $RustVersion)
$env:RUSTUP_TOOLCHAIN = $RustVersion
$env:GOTOOLCHAIN = 'local'
$env:GOFLAGS = '-modcacherw -mod=readonly'

$Downloader = Join-Path $Toolchains 'aria2-next/aria2-next.exe'
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Downloader) | Out-Null
$DownloaderHash = '12bbd48dfd2c86f55419bf92b196dd3aedd6983116b5ce10c85d27a5a76c2baa'
if (-not (Test-Path -LiteralPath $Downloader)) {
    # Bootstrap the downloader once; later source downloads use aria2-next.
    Invoke-WebRequest -Uri 'https://github.com/AnInsomniacy/aria2-next/releases/download/v3.1.0/aria2-next-3.1.0-windows-x86_64.exe' -OutFile $Downloader
}
if ((Get-FileHash -LiteralPath $Downloader -Algorithm SHA256).Hash.ToLowerInvariant() -ne $DownloaderHash) {
    throw 'aria2-next SHA-256 does not match its fixed release input'
}
$env:ARIA2_NEXT = $Downloader
Invoke-Native python @('-X', 'utf8', '-m', 'pip', 'install', '--disable-pip-version-check', 'Pillow==12.2.0')

$Exports = [ordered]@{
    GOROOT = $env:GOROOT
    GOPATH = $env:GOPATH
    RUSTUP_HOME = $env:RUSTUP_HOME
    CARGO_HOME = $env:CARGO_HOME
    RUSTUP_TOOLCHAIN = $env:RUSTUP_TOOLCHAIN
    GOTOOLCHAIN = $env:GOTOOLCHAIN
    GOFLAGS = $env:GOFLAGS
    ARIA2_NEXT = $env:ARIA2_NEXT
}
foreach ($Entry in $Exports.GetEnumerator()) {
    [IO.File]::AppendAllText($env:GITHUB_ENV, "$($Entry.Key)=$($Entry.Value)`n", [Text.UTF8Encoding]::new($false))
}
[IO.File]::AppendAllText($env:GITHUB_PATH, "$CargoBin`n$GoBin`n", [Text.UTF8Encoding]::new($false))
