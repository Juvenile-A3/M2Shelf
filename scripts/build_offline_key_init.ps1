#Requires -Version 7.2

[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
Set-StrictMode -Version 3.0

$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$manifestPath = Join-Path $repoRoot "tools\offline-key-init\Cargo.toml"
$tauriConfig = Get-Content -LiteralPath (Join-Path $repoRoot "src-tauri\tauri.conf.json") -Raw -Encoding UTF8 | ConvertFrom-Json
$version = [string]$tauriConfig.version
if ($version -notmatch '^(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)$') {
  throw "Tauri version must be a stable canonical semantic version."
}
if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
  throw "Offline key initializer manifest was not found."
}
$outputDirectory = Join-Path $repoRoot "bundle\offline-key-init-v$version"
if (Test-Path -LiteralPath $outputDirectory) {
  throw "Offline key initializer distribution directory already exists; refusing to overwrite it."
}

function Assert-PlainFile {
  param(
    [Parameter(Mandatory = $true)][string]$Path,
    [Parameter(Mandatory = $true)][string]$Label
  )
  if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
    throw "$Label was not produced."
  }
  $item = Get-Item -LiteralPath $Path -Force
  if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
    throw "$Label must be a plain file."
  }
  return $item
}

function Assert-X64Pe {
  param([Parameter(Mandatory = $true)][string]$Path)
  $stream = [System.IO.File]::OpenRead($Path)
  $reader = [System.IO.BinaryReader]::new($stream)
  try {
    if ($stream.Length -lt 0x40) { throw "Offline key initializer is too small." }
    $stream.Position = 0x3C
    $peOffset = $reader.ReadInt32()
    if ($peOffset -lt 0 -or ($peOffset + 6) -gt $stream.Length) {
      throw "Offline key initializer PE header offset is invalid."
    }
    $stream.Position = $peOffset
    if ($reader.ReadUInt32() -ne 0x00004550 -or $reader.ReadUInt16() -ne 0x8664) {
      throw "Offline key initializer is not an x64 PE."
    }
  } finally {
    $reader.Dispose()
    $stream.Dispose()
  }
}

function Assert-NoPrivateBuildPath {
  param([Parameter(Mandatory = $true)][string]$Path)
  $bytes = [System.IO.File]::ReadAllBytes($Path)
  $utf8 = [System.Text.Encoding]::UTF8.GetString($bytes)
  $utf16 = [System.Text.Encoding]::Unicode.GetString($bytes)
  $privatePaths = @($repoRoot, $env:USERPROFILE) |
    Where-Object { -not [string]::IsNullOrWhiteSpace($_) } |
    ForEach-Object { [System.IO.Path]::GetFullPath($_).TrimEnd('\') } |
    Select-Object -Unique
  foreach ($privatePath in $privatePaths) {
    foreach ($variant in @($privatePath, $privatePath.Replace('\', '/')) | Select-Object -Unique) {
      if ($utf8.IndexOf($variant, [System.StringComparison]::OrdinalIgnoreCase) -ge 0 -or
          $utf16.IndexOf($variant, [System.StringComparison]::OrdinalIgnoreCase) -ge 0) {
        throw "$([System.IO.Path]::GetFileName($Path)) contains a private builder path."
      }
    }
  }
}

$separator = [char]0x1F
$remapArguments = [System.Collections.Generic.List[string]]::new()
if (-not [string]::IsNullOrWhiteSpace($env:USERPROFILE)) {
  $userProfile = [System.IO.Path]::GetFullPath($env:USERPROFILE).TrimEnd('\')
  $remapArguments.Add("--remap-path-prefix=$userProfile=<USERPROFILE>")
}
$remapArguments.Add("--remap-path-prefix=$($repoRoot.TrimEnd('\'))=<SOURCE>")
$previousEncodedFlags = $env:CARGO_ENCODED_RUSTFLAGS
$encodedRemaps = $remapArguments -join $separator
$env:CARGO_ENCODED_RUSTFLAGS = if ([string]::IsNullOrWhiteSpace($previousEncodedFlags)) {
  $encodedRemaps
} else {
  "$previousEncodedFlags$separator$encodedRemaps"
}

try {
  Push-Location $repoRoot
  try {
    & cargo fmt --manifest-path $manifestPath -- --check
    if ($LASTEXITCODE -ne 0) {
      throw "Offline key initializer formatting check failed with exit code $LASTEXITCODE."
    }
    & cargo test --manifest-path $manifestPath --locked
    if ($LASTEXITCODE -ne 0) {
      throw "Offline key initializer tests failed with exit code $LASTEXITCODE."
    }
    & cargo clippy --manifest-path $manifestPath --all-targets --locked -- -D warnings
    if ($LASTEXITCODE -ne 0) {
      throw "Offline key initializer clippy failed with exit code $LASTEXITCODE."
    }
    & cargo build --manifest-path $manifestPath --release --locked --bin M2ShelfOfflineKeyInit
    if ($LASTEXITCODE -ne 0) {
      throw "Offline key initializer release build failed with exit code $LASTEXITCODE."
    }
  } finally {
    Pop-Location
  }
} finally {
  $env:CARGO_ENCODED_RUSTFLAGS = $previousEncodedFlags
}

$source = Join-Path $repoRoot "tools\offline-key-init\target\release\M2ShelfOfflineKeyInit.exe"
$sourceItem = Assert-PlainFile -Path $source -Label "Offline key initializer"
Assert-X64Pe -Path $sourceItem.FullName
Assert-NoPrivateBuildPath -Path $sourceItem.FullName

if (Test-Path -LiteralPath $outputDirectory) {
  throw "Offline key initializer distribution directory appeared during the build; refusing to overwrite it."
}
$outputDirectoryItem = New-Item -ItemType Directory -Path $outputDirectory -ErrorAction Stop
if (($outputDirectoryItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
  throw "Offline key initializer output directory must be a plain directory."
}
$output = Join-Path $outputDirectory "M2ShelfOfflineKeyInit-v$version-x64.exe"
$existingOutput = Get-Item -LiteralPath $output -Force -ErrorAction SilentlyContinue
if ($null -ne $existingOutput -and
    (($existingOutput.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0 -or
     $existingOutput.PSIsContainer)) {
  throw "Existing offline key initializer output is not a plain file."
}
[System.IO.File]::Copy($sourceItem.FullName, $output, $false)
$outputItem = Assert-PlainFile -Path $output -Label "Copied offline key initializer"
Assert-X64Pe -Path $outputItem.FullName
Assert-NoPrivateBuildPath -Path $outputItem.FullName

$hash = (Get-FileHash -LiteralPath $outputItem.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
$sidecar = "$output.sha256"
$existingSidecar = Get-Item -LiteralPath $sidecar -Force -ErrorAction SilentlyContinue
if ($null -ne $existingSidecar -and
    (($existingSidecar.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0 -or
     $existingSidecar.PSIsContainer)) {
  throw "Existing offline key initializer checksum is not a plain file."
}
$sidecarText = "$hash  $([System.IO.Path]::GetFileName($outputItem.FullName))$([Environment]::NewLine)"
$sidecarBytes = [System.Text.UTF8Encoding]::new($false).GetBytes($sidecarText)
$sidecarStream = [System.IO.File]::Open(
  $sidecar,
  [System.IO.FileMode]::CreateNew,
  [System.IO.FileAccess]::Write,
  [System.IO.FileShare]::None
)
try {
  $sidecarStream.Write($sidecarBytes, 0, $sidecarBytes.Length)
  $sidecarStream.Flush($true)
} finally {
  $sidecarStream.Dispose()
}
Write-Output "Offline key initializer: $($outputItem.FullName)"
Write-Output "SHA-256: $hash"
