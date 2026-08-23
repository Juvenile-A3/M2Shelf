param(
  [string]$Architecture = "x64"
)

$ErrorActionPreference = "Stop"
$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$config = Get-Content -LiteralPath (Join-Path $repoRoot "src-tauri\tauri.conf.json") -Raw -Encoding UTF8 | ConvertFrom-Json
$version = [string]$config.version
if ([string]::IsNullOrWhiteSpace($version)) { throw "Tauri version is missing" }

$iconDirectory = Join-Path $repoRoot "src-tauri\icons"
$assetNames = @(
  "icon-source.png",
  "icon.svg",
  "icon-master.png",
  "icon.png",
  "16x16.png",
  "24x24.png",
  "32x32.png",
  "48x48.png",
  "64x64.png",
  "128x128.png",
  "256x256.png",
  "128x128@2x.png",
  "512x512.png",
  "icon.ico"
)
foreach ($name in $assetNames) {
  $path = Join-Path $iconDirectory $name
  if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Brand asset is missing: $path" }
}

$bundleDirectory = Join-Path $repoRoot "bundle"
[System.IO.Directory]::CreateDirectory($bundleDirectory) | Out-Null
$stageDirectory = Join-Path $bundleDirectory (".brand-stage-" + $version)
$expectedStageRoot = [System.IO.Path]::GetFullPath($bundleDirectory).TrimEnd('\') + '\'
$resolvedStage = [System.IO.Path]::GetFullPath($stageDirectory)
if (-not $resolvedStage.StartsWith($expectedStageRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
  throw "Unsafe staging directory: $resolvedStage"
}
if (Test-Path -LiteralPath $resolvedStage) { Remove-Item -LiteralPath $resolvedStage -Recurse -Force }
[System.IO.Directory]::CreateDirectory($resolvedStage) | Out-Null

$archiveName = "M2Shelf-Brand-Assets-$version-$Architecture.zip"
$archivePath = Join-Path $bundleDirectory $archiveName
$archiveHashPath = "$archivePath.sha256"
try {
  foreach ($name in $assetNames) {
    Copy-Item -LiteralPath (Join-Path $iconDirectory $name) -Destination (Join-Path $resolvedStage $name) -Force
  }
  $sourceHash = (Get-FileHash -LiteralPath (Join-Path $iconDirectory "icon-source.png") -Algorithm SHA256).Hash
  $readme = @"
M²Shelf $version 品牌资源

- icon-source.png 是用户确认并去除黑色四角后的 1254×1254 透明背景权威位图母版；SHA-256：$sourceHash。
- 所有 PNG 与 ICO 都只由该母版等比例缩放生成；未裁切、改色、锐化、重排、描摹或重绘，红色圆角轮廓、渐变、柔光与字形完整保留。
- PNG 提供 16/24/32/48/64/128/256/512/1024 px；icon.ico 含 16/24/32/48/64/128/256 px 多尺寸帧。
- icon.svg 仅是引用 icon-source.png 的无变形兼容容器，不是重新描摹的矢量版本。
- 可通过仓库 scripts/generate_icons.ps1 从同一母版重复生成所有尺寸。
"@
  [System.IO.File]::WriteAllText((Join-Path $resolvedStage "README_zh-CN.txt"), $readme, [System.Text.UTF8Encoding]::new($true))
  $hashLines = Get-ChildItem -LiteralPath $resolvedStage -File | Sort-Object Name | ForEach-Object {
    "$(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256 | Select-Object -ExpandProperty Hash)  $($_.Name)"
  }
  [System.IO.File]::WriteAllLines((Join-Path $resolvedStage "SHA256SUMS.txt"), $hashLines, [System.Text.UTF8Encoding]::new($false))

  if (Test-Path -LiteralPath $archivePath) { Remove-Item -LiteralPath $archivePath -Force }
  Compress-Archive -Path (Join-Path $resolvedStage "*") -DestinationPath $archivePath -CompressionLevel Optimal
  $archiveHash = Get-FileHash -LiteralPath $archivePath -Algorithm SHA256
  [System.IO.File]::WriteAllText($archiveHashPath, "$($archiveHash.Hash)  $archiveName`r`n", [System.Text.UTF8Encoding]::new($false))
} finally {
  if (Test-Path -LiteralPath $resolvedStage) { Remove-Item -LiteralPath $resolvedStage -Recurse -Force }
}

Write-Output "Brand archive: $archivePath"
Write-Output "SHA-256: $($archiveHash.Hash)"
