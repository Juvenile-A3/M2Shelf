param(
  [switch]$SkipBuild,
  [string]$Architecture = "x64"
)

$ErrorActionPreference = "Stop"
$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$tauriConfigPath = Join-Path $repoRoot "src-tauri\tauri.conf.json"
$config = Get-Content -LiteralPath $tauriConfigPath -Raw -Encoding UTF8 | ConvertFrom-Json
$version = [string]$config.version
if ([string]::IsNullOrWhiteSpace($version)) { throw "Tauri version is missing" }

$package = Get-Content -LiteralPath (Join-Path $repoRoot "package.json") -Raw -Encoding UTF8 | ConvertFrom-Json
$cargoManifestPath = Join-Path $repoRoot "src-tauri\Cargo.toml"
$cargoManifest = Get-Content -LiteralPath $cargoManifestPath -Raw -Encoding UTF8
$cargoVersionMatch = [regex]::Match($cargoManifest, '(?m)^version\s*=\s*"([^"]+)"')
if (-not $cargoVersionMatch.Success) { throw "Cargo package version is missing" }
$cargoLockPath = Join-Path $repoRoot "src-tauri\Cargo.lock"
$cargoLock = Get-Content -LiteralPath $cargoLockPath -Raw -Encoding UTF8
$cargoLockVersionMatch = [regex]::Match($cargoLock, '(?ms)^\[\[package\]\]\s*\r?\nname\s*=\s*"m2shelf"\s*\r?\nversion\s*=\s*"([^"]+)"')
if (-not $cargoLockVersionMatch.Success) { throw "Cargo lockfile package version is missing" }
$boundVersions = @(
  [string]$package.version,
  [string]$cargoVersionMatch.Groups[1].Value,
  [string]$cargoLockVersionMatch.Groups[1].Value,
  $version
) | Select-Object -Unique
if ($boundVersions.Count -ne 1) {
  throw "package.json, Cargo.toml, Cargo.lock, and tauri.conf.json versions must match: $($boundVersions -join ', ')"
}

$machineByArchitecture = @{
  "x64" = [uint16]0x8664
  "ARM64" = [uint16]0xAA64
  "x86" = [uint16]0x014C
}
if (-not $machineByArchitecture.ContainsKey($Architecture)) {
  throw "Unsupported Portable architecture: $Architecture"
}

if (-not $SkipBuild) {
  & (Join-Path $PSScriptRoot "build_windows_release.ps1") -Bundles none
  if ($LASTEXITCODE -ne 0) { throw "Tauri build failed with exit code $LASTEXITCODE" }
}

$releaseDirectory = Join-Path $repoRoot "src-tauri\target\release"
$binaryCandidates = @(
  (Join-Path $releaseDirectory "m2shelf.exe"),
  (Join-Path $releaseDirectory "M2Shelf.exe")
)
$sourceBinary = $binaryCandidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
if (-not $sourceBinary) { throw "Release executable was not found. Run the Tauri release build first." }

$expectedProductName = "M$([char]0x00B2)Shelf"
$binaryVersion = (Get-Item -LiteralPath $sourceBinary).VersionInfo
if ($binaryVersion.ProductVersion -ne $version -or $binaryVersion.ProductName -ne $expectedProductName) {
  throw "Release executable metadata does not match $expectedProductName $version. Rebuild before using -SkipBuild."
}
$stream = [System.IO.File]::OpenRead($sourceBinary)
$reader = [System.IO.BinaryReader]::new($stream)
try {
  $stream.Position = 0x3C
  $peOffset = $reader.ReadInt32()
  $stream.Position = $peOffset
  if ($reader.ReadUInt32() -ne 0x00004550) { throw "Release executable has an invalid PE signature." }
  $actualMachine = $reader.ReadUInt16()
  $expectedMachine = $machineByArchitecture[$Architecture]
  if ($actualMachine -ne $expectedMachine) {
    throw "Release executable architecture does not match $Architecture (PE machine 0x$($actualMachine.ToString('X4')))."
  }
} finally {
  $reader.Dispose()
  $stream.Dispose()
}

$freshnessFiles = @(
  (Join-Path $repoRoot "package.json"),
  (Join-Path $repoRoot "package-lock.json"),
  (Join-Path $repoRoot "index.html"),
  (Join-Path $repoRoot "vite.config.ts"),
  (Join-Path $repoRoot "tsconfig.json"),
  (Join-Path $repoRoot "tsconfig.node.json"),
  $cargoManifestPath,
  $cargoLockPath,
  (Join-Path $repoRoot "src-tauri\build.rs"),
  $tauriConfigPath,
  (Join-Path $repoRoot "src-tauri\windows-app-manifest.xml"),
  (Join-Path $repoRoot "src-tauri\icons\icon.ico")
)
$freshnessDirectories = @(
  (Join-Path $repoRoot "dist"),
  (Join-Path $repoRoot "src"),
  (Join-Path $repoRoot "src-tauri\capabilities"),
  (Join-Path $repoRoot "src-tauri\migrations"),
  (Join-Path $repoRoot "src-tauri\src")
)
foreach ($directory in $freshnessDirectories) {
  if (Test-Path -LiteralPath $directory -PathType Container) {
    $freshnessFiles += Get-ChildItem -LiteralPath $directory -File -Recurse | Select-Object -ExpandProperty FullName
  }
}
$newestInput = $freshnessFiles |
  Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } |
  ForEach-Object { Get-Item -LiteralPath $_ } |
  Sort-Object LastWriteTimeUtc -Descending |
  Select-Object -First 1
$releaseBinary = Get-Item -LiteralPath $sourceBinary
if ($newestInput -and $releaseBinary.LastWriteTimeUtc -lt $newestInput.LastWriteTimeUtc) {
  throw "Release executable is older than $($newestInput.FullName). Run a fresh Tauri build before packaging."
}

$bundleDirectory = Join-Path $repoRoot "bundle"
[System.IO.Directory]::CreateDirectory($bundleDirectory) | Out-Null
$stageDirectory = Join-Path $bundleDirectory (".portable-stage-" + $version)
$expectedStageRoot = [System.IO.Path]::GetFullPath($bundleDirectory).TrimEnd('\') + '\'
$resolvedStage = [System.IO.Path]::GetFullPath($stageDirectory)
if (-not $resolvedStage.StartsWith($expectedStageRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
  throw "Unsafe staging directory: $resolvedStage"
}
if (Test-Path -LiteralPath $resolvedStage) { Remove-Item -LiteralPath $resolvedStage -Recurse -Force }
[System.IO.Directory]::CreateDirectory($resolvedStage) | Out-Null

$archiveName = "M2Shelf-Portable-$version-$Architecture.zip"
$archivePath = Join-Path $bundleDirectory $archiveName
$archiveHashPath = "$archivePath.sha256"
try {
  $portableExe = Join-Path $resolvedStage "M2Shelf.exe"
  $portableReadme = Join-Path $resolvedStage "README_zh-CN.txt"
  Copy-Item -LiteralPath $sourceBinary -Destination $portableExe -Force

  $readmeTemplate = Get-Content -LiteralPath (Join-Path $repoRoot "docs\PORTABLE_README_zh-CN.txt") -Raw -Encoding UTF8
  $readme = $readmeTemplate.Replace("{{VERSION}}", $version)
  [System.IO.File]::WriteAllText($portableReadme, $readme, [System.Text.UTF8Encoding]::new($true))

  $checksumLines = @(
    "$(Get-FileHash -LiteralPath $portableExe -Algorithm SHA256 | Select-Object -ExpandProperty Hash)  M2Shelf.exe",
    "$(Get-FileHash -LiteralPath $portableReadme -Algorithm SHA256 | Select-Object -ExpandProperty Hash)  README_zh-CN.txt"
  )
  [System.IO.File]::WriteAllLines((Join-Path $resolvedStage "SHA256SUMS.txt"), $checksumLines, [System.Text.UTF8Encoding]::new($false))

  if (Test-Path -LiteralPath $archivePath) { Remove-Item -LiteralPath $archivePath -Force }
  Compress-Archive -Path (Join-Path $resolvedStage "*") -DestinationPath $archivePath -CompressionLevel Optimal
  $archiveHash = Get-FileHash -LiteralPath $archivePath -Algorithm SHA256
  [System.IO.File]::WriteAllText($archiveHashPath, "$($archiveHash.Hash)  $archiveName`r`n", [System.Text.UTF8Encoding]::new($false))
} finally {
  if (Test-Path -LiteralPath $resolvedStage) { Remove-Item -LiteralPath $resolvedStage -Recurse -Force }
}

Write-Output "Portable archive: $archivePath"
Write-Output "SHA-256: $($archiveHash.Hash)"
