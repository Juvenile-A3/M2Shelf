#Requires -Version 7.2

[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)][string]$CandidateDirectory,
  [Parameter(Mandatory = $true)][string]$EncryptedSeedPath,
  [Parameter(Mandatory = $true)][string]$UpdaterPath,
  [Parameter(Mandatory = $true)][ValidatePattern('^[0-9A-Fa-f]{64}$')][string]$TrustedSignerSha256,
  [Parameter(Mandatory = $true)][ValidatePattern('^[0-9A-Fa-f]{64}$')][string]$TrustedManifestGeneratorSha256,
  [Parameter(Mandatory = $true)][ValidatePattern('^[0-9A-Fa-f]{64}$')][string]$TrustedPortableSha256,
  [Parameter(Mandatory = $true)][ValidatePattern('^[0-9A-Fa-f]{64}$')][string]$TrustedNsisSha256,
  [string]$NotesPath = "",
  [string]$PublishedAt = "",
  [switch]$Force
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version 3.0

if (-not [System.OperatingSystem]::IsWindows()) {
  throw "Offline update signing requires Windows CurrentUser DPAPI."
}

$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$manifestScript = Join-Path $PSScriptRoot "generate_update_manifest.ps1"
if (-not (Test-Path -LiteralPath $manifestScript -PathType Leaf)) {
  throw "generate_update_manifest.ps1 was not found next to this wrapper."
}
$manifestScriptItem = Get-Item -LiteralPath $manifestScript -Force
if (($manifestScriptItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
  throw "generate_update_manifest.ps1 must not be a symbolic link or other reparse point."
}
$actualManifestGeneratorSha256 =
  (Get-FileHash -LiteralPath $manifestScriptItem.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actualManifestGeneratorSha256 -cne $TrustedManifestGeneratorSha256.ToLowerInvariant()) {
  throw "Manifest generator SHA-256 does not match the independently supplied fingerprint."
}

$config = Get-Content -LiteralPath (Join-Path $repoRoot "src-tauri\tauri.conf.json") -Raw -Encoding UTF8 | ConvertFrom-Json
$version = [string]$config.version
if ($version -notmatch '^(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)$') {
  throw "Tauri version must be a stable canonical semantic version (major.minor.patch)."
}

$candidatePath = [System.IO.Path]::GetFullPath($CandidateDirectory)
if (-not (Test-Path -LiteralPath $candidatePath -PathType Container)) {
  throw "CandidateDirectory was not found."
}
$candidatePath = (Resolve-Path -LiteralPath $candidatePath).ProviderPath
$portablePath = Join-Path $candidatePath "M2Shelf-Portable-$version-x64.zip"
$installerPath = Join-Path $candidatePath "M2Shelf-Setup-$version-x64.exe"
foreach ($assetPath in @($portablePath, $installerPath)) {
  if (-not (Test-Path -LiteralPath $assetPath -PathType Leaf)) {
    throw "Offline signing candidate is incomplete: $([System.IO.Path]::GetFileName($assetPath)) was not found."
  }
  $assetItem = Get-Item -LiteralPath $assetPath -Force
  if (($assetItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
    throw "Offline signing candidates must not be symbolic links or other reparse points."
  }
}
$actualPortableSha256 = (Get-FileHash -LiteralPath $portablePath -Algorithm SHA256).Hash.ToLowerInvariant()
$actualNsisSha256 = (Get-FileHash -LiteralPath $installerPath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actualPortableSha256 -cne $TrustedPortableSha256.ToLowerInvariant() -or
    $actualNsisSha256 -cne $TrustedNsisSha256.ToLowerInvariant()) {
  throw "Candidate hashes do not match the independently verified release fingerprints."
}

$resolvedUpdater = [System.IO.Path]::GetFullPath($UpdaterPath)
if (-not (Test-Path -LiteralPath $resolvedUpdater -PathType Leaf)) {
  throw "UpdaterPath was not found."
}
$resolvedUpdater = (Resolve-Path -LiteralPath $resolvedUpdater).ProviderPath
$actualSignerSha256 = (Get-FileHash -LiteralPath $resolvedUpdater -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actualSignerSha256 -cne $TrustedSignerSha256.ToLowerInvariant()) {
  throw "Signer helper SHA-256 does not match the independently supplied fingerprint."
}

$seedPath = [System.IO.Path]::GetFullPath($EncryptedSeedPath)
if (-not (Test-Path -LiteralPath $seedPath -PathType Leaf)) {
  throw "EncryptedSeedPath was not found."
}
$seedItem = Get-Item -LiteralPath $seedPath -Force
if (($seedItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
  throw "EncryptedSeedPath must not be a symbolic link or other reparse point."
}
$seedPath = $seedItem.FullName
$repoPrefix = $repoRoot.TrimEnd(
  [System.IO.Path]::DirectorySeparatorChar,
  [System.IO.Path]::AltDirectorySeparatorChar
) + [System.IO.Path]::DirectorySeparatorChar
if ($seedPath.StartsWith($repoPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
  throw "The DPAPI-protected signing seed must be stored outside the source repository."
}
if ($seedItem.Length -lt 1 -or $seedItem.Length -gt 16384) {
  throw "The DPAPI-protected signing seed has an invalid size."
}

if (-not [string]::IsNullOrWhiteSpace(
    [System.Environment]::GetEnvironmentVariable("M2SHELF_UPDATE_PRIVATE_KEY", [System.EnvironmentVariableTarget]::Process)
  )) {
  throw "M2SHELF_UPDATE_PRIVATE_KEY is already set. Clear it before using the DPAPI wrapper."
}

$protectedBytes = $null
$seedBytes = $null
$privateKeyText = $null
$trustedInputLocks = [System.Collections.Generic.List[System.IO.FileStream]]::new()
try {
  foreach ($trustedPath in @($manifestScriptItem.FullName, $resolvedUpdater, $portablePath, $installerPath)) {
    $trustedInputLocks.Add([System.IO.File]::Open(
      $trustedPath,
      [System.IO.FileMode]::Open,
      [System.IO.FileAccess]::Read,
      [System.IO.FileShare]::Read
    ))
  }
  if ((Get-FileHash -LiteralPath $manifestScriptItem.FullName -Algorithm SHA256).Hash.ToLowerInvariant() -cne $actualManifestGeneratorSha256 -or
      (Get-FileHash -LiteralPath $resolvedUpdater -Algorithm SHA256).Hash.ToLowerInvariant() -cne $actualSignerSha256 -or
      (Get-FileHash -LiteralPath $portablePath -Algorithm SHA256).Hash.ToLowerInvariant() -cne $actualPortableSha256 -or
      (Get-FileHash -LiteralPath $installerPath -Algorithm SHA256).Hash.ToLowerInvariant() -cne $actualNsisSha256) {
    throw "A trusted signing input changed while it was being locked."
  }

  $protectedBytes = [System.IO.File]::ReadAllBytes($seedPath)
  try {
    $seedBytes = [System.Security.Cryptography.ProtectedData]::Unprotect(
      $protectedBytes,
      $null,
      [System.Security.Cryptography.DataProtectionScope]::CurrentUser
    )
  } catch {
    throw "CurrentUser DPAPI could not decrypt the signing seed for this Windows user."
  }
  if ($seedBytes.Length -ne 32) {
    throw "The decrypted Ed25519 seed must contain exactly 32 bytes."
  }

  $privateKeyText = [System.Convert]::ToBase64String($seedBytes)
  [System.Environment]::SetEnvironmentVariable(
    "M2SHELF_UPDATE_PRIVATE_KEY",
    $privateKeyText,
    [System.EnvironmentVariableTarget]::Process
  )

  $manifestArguments = @{
    PortableArchive = $portablePath
    NsisInstaller = $installerPath
    UpdaterPath = $resolvedUpdater
    OutputPath = (Join-Path $candidatePath "latest.json")
    TrustedSignerSha256 = $TrustedSignerSha256.ToLowerInvariant()
  }
  if (-not [string]::IsNullOrWhiteSpace($NotesPath)) {
    $manifestArguments.NotesPath = [System.IO.Path]::GetFullPath($NotesPath)
  }
  if (-not [string]::IsNullOrWhiteSpace($PublishedAt)) {
    $manifestArguments.PublishedAt = $PublishedAt
  }
  if ($Force) {
    $manifestArguments.Force = $true
  }

  & $manifestScript @manifestArguments
} finally {
  [System.Environment]::SetEnvironmentVariable(
    "M2SHELF_UPDATE_PRIVATE_KEY",
    $null,
    [System.EnvironmentVariableTarget]::Process
  )
  $privateKeyText = $null
  if ($null -ne $seedBytes) {
    [System.Array]::Clear($seedBytes, 0, $seedBytes.Length)
  }
  if ($null -ne $protectedBytes) {
    [System.Array]::Clear($protectedBytes, 0, $protectedBytes.Length)
  }
  $seedBytes = $null
  $protectedBytes = $null
  foreach ($stream in $trustedInputLocks) {
    $stream.Dispose()
  }
}
