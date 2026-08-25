#Requires -Version 7.2

[CmdletBinding()]
param(
  [string]$OutputDirectory = "",
  [switch]$Force
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version 3.0

if (-not [System.OperatingSystem]::IsWindows()) {
  throw "The portable key tool is supported only on Windows."
}

$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$manifestPath = Join-Path $repoRoot "tools\portable-key-tool\Cargo.toml"
$configPath = Join-Path $repoRoot "src-tauri\tauri.conf.json"
if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
  throw "Portable key tool manifest was not found."
}
if (-not (Test-Path -LiteralPath $configPath -PathType Leaf)) {
  throw "Tauri configuration was not found."
}

$config = Get-Content -LiteralPath $configPath -Raw -Encoding UTF8 | ConvertFrom-Json
$version = [string]$config.version
if ($version -notmatch '^(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)$') {
  throw "Tauri version must be a stable canonical semantic version."
}

$resolvedOutputDirectory = if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
  Join-Path $repoRoot "bundle\portable-key-tool-v$version"
} else {
  [System.IO.Path]::GetFullPath($OutputDirectory)
}
$outputPath = Join-Path $resolvedOutputDirectory "M2ShelfPortableKeyTool.exe"
$sidecarPath = "$outputPath.sha256"

function Assert-PlainFile {
  param(
    [Parameter(Mandatory = $true)][string]$Path,
    [Parameter(Mandatory = $true)][string]$Label
  )

  if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
    throw "$Label was not found."
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
    if ($stream.Length -lt 0x40) {
      throw "Portable key tool is too small to be a Windows executable."
    }
    $stream.Position = 0x3C
    $peOffset = $reader.ReadInt32()
    if ($peOffset -lt 0 -or ($peOffset + 6) -gt $stream.Length) {
      throw "Portable key tool PE header offset is invalid."
    }
    $stream.Position = $peOffset
    if ($reader.ReadUInt32() -ne 0x00004550 -or $reader.ReadUInt16() -ne 0x8664) {
      throw "Portable key tool is not an x64 Windows executable."
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
      throw "Portable key tool formatting check failed with exit code $LASTEXITCODE."
    }
    & cargo test --manifest-path $manifestPath --locked
    if ($LASTEXITCODE -ne 0) {
      throw "Portable key tool tests failed with exit code $LASTEXITCODE."
    }
    & cargo clippy --manifest-path $manifestPath --all-targets --locked -- -D warnings
    if ($LASTEXITCODE -ne 0) {
      throw "Portable key tool clippy failed with exit code $LASTEXITCODE."
    }
    & cargo build --manifest-path $manifestPath --release --locked --bin M2ShelfPortableKeyTool
    if ($LASTEXITCODE -ne 0) {
      throw "Portable key tool release build failed with exit code $LASTEXITCODE."
    }
    $metadataText = & cargo metadata --manifest-path $manifestPath --no-deps --format-version 1
    if ($LASTEXITCODE -ne 0) {
      throw "Could not locate the portable key tool build output."
    }
  } finally {
    Pop-Location
  }
} finally {
  $env:CARGO_ENCODED_RUSTFLAGS = $previousEncodedFlags
}

try {
  $metadata = $metadataText | ConvertFrom-Json
  $targetDirectory = [System.IO.Path]::GetFullPath([string]$metadata.target_directory)
} catch {
  throw "Cargo returned invalid metadata for the portable key tool."
}
$sourcePath = Join-Path $targetDirectory "release\M2ShelfPortableKeyTool.exe"
$sourceItem = Assert-PlainFile -Path $sourcePath -Label "Portable key tool"
Assert-X64Pe -Path $sourceItem.FullName
Assert-NoPrivateBuildPath -Path $sourceItem.FullName

if (Test-Path -LiteralPath $resolvedOutputDirectory) {
  $outputDirectoryItem = Get-Item -LiteralPath $resolvedOutputDirectory -Force
  if (-not $outputDirectoryItem.PSIsContainer -or
      ($outputDirectoryItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
    throw "Portable key tool output directory must be a plain directory."
  }
} else {
  $outputDirectoryItem = New-Item -ItemType Directory -Path $resolvedOutputDirectory -ErrorAction Stop
  if (($outputDirectoryItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
    throw "Portable key tool output directory must be a plain directory."
  }
}

foreach ($existingPath in @($outputPath, $sidecarPath)) {
  if (Test-Path -LiteralPath $existingPath) {
    $existingItem = Get-Item -LiteralPath $existingPath -Force
    if ($existingItem.PSIsContainer -or
        ($existingItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
      throw "Existing portable key tool output is not a plain file."
    }
    if (-not $Force) {
      throw "Portable key tool output already exists. Pass -Force only to rebuild this local tool copy."
    }
  }
}

[System.IO.File]::Copy($sourceItem.FullName, $outputPath, [bool]$Force)
$outputItem = Assert-PlainFile -Path $outputPath -Label "Copied portable key tool"
Assert-X64Pe -Path $outputItem.FullName
Assert-NoPrivateBuildPath -Path $outputItem.FullName
$hash = (Get-FileHash -LiteralPath $outputItem.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
$sidecarText = "$hash  $($outputItem.Name)$([Environment]::NewLine)"
[System.IO.File]::WriteAllText($sidecarPath, $sidecarText, [System.Text.UTF8Encoding]::new($false))

Write-Output "Portable key tool: $($outputItem.FullName)"
Write-Output "SHA-256: $hash"
