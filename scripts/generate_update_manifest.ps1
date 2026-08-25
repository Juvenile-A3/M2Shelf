param(
  [Parameter(Mandatory = $true)][string]$PortableArchive,
  [Parameter(Mandatory = $true)][string]$NsisInstaller,
  [string]$UpdaterPath = "",
  [string]$OutputPath = "",
  [string]$NotesPath = "",
  [string]$PublishedAt = "",
  [string]$TrustedSignerSha256 = $env:M2SHELF_TRUSTED_SIGNER_SHA256,
  [switch]$Force
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version 3.0

$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$config = Get-Content -LiteralPath (Join-Path $repoRoot "src-tauri\tauri.conf.json") -Raw -Encoding UTF8 | ConvertFrom-Json
$version = [string]$config.version
if ($version -notmatch '^(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)$') {
  throw "Tauri version must be a stable canonical semantic version (major.minor.patch)."
}

if ([string]::IsNullOrWhiteSpace($env:M2SHELF_UPDATE_PRIVATE_KEY)) {
  throw "M2SHELF_UPDATE_PRIVATE_KEY is required to sign update artifacts. The key must be supplied only through the environment."
}

$portablePath = [System.IO.Path]::GetFullPath($PortableArchive)
$installerPath = [System.IO.Path]::GetFullPath($NsisInstaller)
foreach ($assetPath in @($portablePath, $installerPath)) {
  if (-not (Test-Path -LiteralPath $assetPath -PathType Leaf)) {
    throw "Update artifact was not found: $([System.IO.Path]::GetFileName($assetPath))"
  }
}
if ([System.IO.Path]::GetFileName($portablePath) -cne "M2Shelf-Portable-$version-x64.zip" -or
    [System.IO.Path]::GetFileName($installerPath) -cne "M2Shelf-Setup-$version-x64.exe") {
  throw "Update artifact names must exactly match the frozen versioned release names."
}

$releaseDirectory = Join-Path $repoRoot "src-tauri\target\release"
$updaterCandidates = if ([string]::IsNullOrWhiteSpace($UpdaterPath)) {
  @(
    (Join-Path $releaseDirectory "M2ShelfUpdater.exe"),
    (Join-Path $releaseDirectory "m2shelf-updater.exe"),
    (Join-Path $releaseDirectory "m2shelf_updater.exe")
  )
} else {
  @([System.IO.Path]::GetFullPath($UpdaterPath))
}
$resolvedUpdater = $updaterCandidates |
  Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } |
  Select-Object -First 1
if (-not $resolvedUpdater) {
  throw "M2ShelfUpdater signer helper was not found. Build the release helper before signing."
}
if ($TrustedSignerSha256 -notmatch '^[0-9A-Fa-f]{64}$') {
  throw "TrustedSignerSha256 (or M2SHELF_TRUSTED_SIGNER_SHA256) is required and must come from an independently stored signer fingerprint."
}
$actualSignerSha256 = (Get-FileHash -LiteralPath $resolvedUpdater -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actualSignerSha256 -cne $TrustedSignerSha256.ToLowerInvariant()) {
  throw "Signer helper SHA-256 does not match the independently trusted fingerprint."
}

$publicKeyPath = Join-Path $repoRoot "src-tauri\update-public-key.txt"
$publicKeyText = (Get-Content -LiteralPath $publicKeyPath -Raw -Encoding UTF8).Trim()
try { $publicKeyBytes = [Convert]::FromBase64String($publicKeyText) } catch { throw "Updater public key is invalid Base64." }
if ($publicKeyBytes.Length -ne 32 -or [Convert]::ToBase64String($publicKeyBytes) -cne $publicKeyText) {
  throw "Updater public key must be canonical Base64 containing exactly 32 bytes."
}

$identityOutput = @(& $resolvedUpdater identity)
if ($LASTEXITCODE -ne 0) { throw "Trusted signer identity check failed." }
$identityLines = @($identityOutput | ForEach-Object { [string]$_ } | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
if ($identityLines.Count -ne 1) { throw "Trusted signer identity must be exactly one JSON object." }
try { $identity = $identityLines[0] | ConvertFrom-Json } catch { throw "Trusted signer identity is invalid JSON." }
$identityProperties = @($identity.PSObject.Properties.Name | Sort-Object)
$expectedIdentityProperties = @("appId", "publicKey", "schemaVersion", "version") | Sort-Object
if (($identityProperties -join "`n") -cne ($expectedIdentityProperties -join "`n") -or
    [uint32]$identity.schemaVersion -ne 1 -or
    [string]$identity.appId -cne "app.morimediashelf.desktop" -or
    [string]$identity.version -cne $version -or
    [string]$identity.publicKey -cne $publicKeyText) {
  throw "Trusted signer identity, version, or public key does not match this release."
}

$resolvedOutput = if ([string]::IsNullOrWhiteSpace($OutputPath)) {
  Join-Path $repoRoot "bundle\latest.json"
} else {
  [System.IO.Path]::GetFullPath($OutputPath)
}
$outputDirectory = Split-Path -Parent $resolvedOutput
[System.IO.Directory]::CreateDirectory($outputDirectory) | Out-Null

function Assert-WritableOutput {
  param([Parameter(Mandatory = $true)][string]$Path)
  if ((Test-Path -LiteralPath $Path) -and -not $Force) {
    throw "Output already exists: $([System.IO.Path]::GetFileName($Path)). Pass -Force only for a deliberate local regeneration."
  }
}

function Get-StrictProperties {
  param([Parameter(Mandatory = $true)]$Object)
  return @($Object.PSObject.Properties | Select-Object -ExpandProperty Name | Sort-Object)
}

function Ensure-HashSidecar {
  param([Parameter(Mandatory = $true)][string]$Path)

  $fileName = [System.IO.Path]::GetFileName($Path)
  $hash = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
  $sidecarPath = "$Path.sha256"
  $expected = "$hash  $fileName"
  if (Test-Path -LiteralPath $sidecarPath -PathType Leaf) {
    $existing = (Get-Content -LiteralPath $sidecarPath -Raw -Encoding UTF8).Trim()
    if ($existing -ieq $expected) { return $sidecarPath }
    if (-not $Force) {
      throw "Existing SHA-256 sidecar does not match $fileName."
    }
  }
  [System.IO.File]::WriteAllText(
    $sidecarPath,
    "$expected`r`n",
    [System.Text.UTF8Encoding]::new($false)
  )
  return $sidecarPath
}

function Invoke-ArtifactSigner {
  param(
    [Parameter(Mandatory = $true)][string]$Platform,
    [Parameter(Mandatory = $true)][string]$Path
  )

  $stdout = @(& $resolvedUpdater sign --version $version --platform $Platform --file $Path)
  if ($LASTEXITCODE -ne 0) {
    throw "M2ShelfUpdater failed to sign $Platform (exit code $LASTEXITCODE)."
  }
  $lines = @($stdout | ForEach-Object { [string]$_ } | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
  if ($lines.Count -ne 1) {
    throw "M2ShelfUpdater must write exactly one JSON object to stdout when signing."
  }
  try {
    $signed = $lines[0] | ConvertFrom-Json
  } catch {
    throw "M2ShelfUpdater returned invalid signing JSON."
  }
  $expectedProperties = @("fileName", "sha256", "signature", "size") | Sort-Object
  if (((Get-StrictProperties -Object $signed) -join "`n") -ne ($expectedProperties -join "`n")) {
    throw "M2ShelfUpdater signing JSON does not match the frozen schema."
  }

  $file = Get-Item -LiteralPath $Path
  $expectedFileName = $file.Name
  $expectedSize = [uint64]$file.Length
  $expectedSha256 = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
  if ([string]$signed.fileName -cne $expectedFileName) {
    throw "Signer fileName does not match the artifact."
  }
  if ([uint64]$signed.size -ne $expectedSize) {
    throw "Signer size does not match the artifact."
  }
  if ([string]$signed.sha256 -cne $expectedSha256 -or [string]$signed.sha256 -notmatch '^[0-9a-f]{64}$') {
    throw "Signer SHA-256 does not match the artifact."
  }
  $signature = [string]$signed.signature
  if ($signature -notmatch '^[A-Za-z0-9+/]{86}==$') {
    throw "Signer signature is not canonical standard Base64."
  }
  try {
    $signatureBytes = [Convert]::FromBase64String($signature)
  } catch {
    throw "Signer signature is not standard Base64."
  }
  if ($signatureBytes.Length -ne 64) {
    throw "Signer signature must contain exactly 64 Ed25519 bytes."
  }

  $signaturePath = Join-Path $outputDirectory "$expectedFileName.sig"
  Assert-WritableOutput -Path $signaturePath
  [System.IO.File]::WriteAllText(
    $signaturePath,
    "$signature`n",
    [System.Text.UTF8Encoding]::new($false)
  )

  $encodedFileName = [System.Uri]::EscapeDataString($expectedFileName)
  return [ordered]@{
    url = "https://github.com/Undermori/M2Shelf/releases/download/v$version/$encodedFileName"
    fileName = $expectedFileName
    size = $expectedSize
    sha256 = $expectedSha256
    signature = $signature
  }
}

$requiredLocales = @("zh-CN", "en-US", "ja-JP", "ko-KR")
if ([string]::IsNullOrWhiteSpace($NotesPath)) {
  $notes = [ordered]@{
    "zh-CN" = "M²Shelf ${version}：稳定性、性能与更新体验改进。"
    "en-US" = "M²Shelf ${version}: stability, performance, and update experience improvements."
    "ja-JP" = "M²Shelf ${version}：安定性、パフォーマンス、更新体験を改善しました。"
    "ko-KR" = "M²Shelf ${version}: 안정성, 성능 및 업데이트 경험을 개선했습니다."
  }
} else {
  $notesFile = [System.IO.Path]::GetFullPath($NotesPath)
  if (-not (Test-Path -LiteralPath $notesFile -PathType Leaf)) {
    throw "Release notes JSON was not found."
  }
  $notesInput = Get-Content -LiteralPath $notesFile -Raw -Encoding UTF8 | ConvertFrom-Json
  $actualLocales = Get-StrictProperties -Object $notesInput
  if (($actualLocales -join "`n") -ne (($requiredLocales | Sort-Object) -join "`n")) {
    throw "Release notes JSON must contain exactly zh-CN, en-US, ja-JP, and ko-KR."
  }
  $notes = [ordered]@{}
  foreach ($locale in $requiredLocales) {
    $value = [string]$notesInput.$locale
    if ([string]::IsNullOrWhiteSpace($value)) { throw "Release note $locale cannot be empty." }
    $notes[$locale] = $value
  }
}

$publishedAtValue = if ([string]::IsNullOrWhiteSpace($PublishedAt)) {
  [DateTimeOffset]::UtcNow
} else {
  try {
    [DateTimeOffset]::Parse(
      $PublishedAt,
      [Globalization.CultureInfo]::InvariantCulture,
      [Globalization.DateTimeStyles]::RoundtripKind
    ).ToUniversalTime()
  } catch {
    throw "PublishedAt must be an ISO-8601 timestamp."
  }
}

Assert-WritableOutput -Path $resolvedOutput
$portableHashPath = Ensure-HashSidecar -Path $portablePath
$installerHashPath = Ensure-HashSidecar -Path $installerPath
$portableAsset = Invoke-ArtifactSigner -Platform "windows-x64-portable" -Path $portablePath
$installerAsset = Invoke-ArtifactSigner -Platform "windows-x64-nsis" -Path $installerPath
$manifest = [ordered]@{
  schemaVersion = 1
  version = $version
  publishedAt = $publishedAtValue.ToString("yyyy-MM-dd'T'HH:mm:ss'Z'", [Globalization.CultureInfo]::InvariantCulture)
  notes = $notes
  platforms = [ordered]@{
    "windows-x64-portable" = $portableAsset
    "windows-x64-nsis" = $installerAsset
  }
}
$manifestJson = $manifest | ConvertTo-Json -Depth 8
[System.IO.File]::WriteAllText($resolvedOutput, "$manifestJson`n", [System.Text.UTF8Encoding]::new($false))

Write-Output "Update manifest: $resolvedOutput"
Write-Output "Portable SHA-256: $portableHashPath"
Write-Output "NSIS SHA-256: $installerHashPath"
Write-Output "Portable signature: $(Join-Path $outputDirectory "$([System.IO.Path]::GetFileName($portablePath)).sig")"
Write-Output "NSIS signature: $(Join-Path $outputDirectory "$([System.IO.Path]::GetFileName($installerPath)).sig")"
