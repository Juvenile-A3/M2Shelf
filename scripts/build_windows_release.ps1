param(
  [ValidateSet("none", "nsis", "msi")]
  [string]$Bundles = "nsis"
)

$ErrorActionPreference = "Stop"
$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
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

Push-Location $repoRoot
try {
  if ($Bundles -eq "none") {
    & npm run tauri build -- --no-bundle
  } else {
    & npm run tauri build -- --bundles $Bundles
  }
  if ($LASTEXITCODE -ne 0) {
    throw "Tauri release build failed with exit code $LASTEXITCODE"
  }
} finally {
  Pop-Location
  if ($null -eq $previousEncodedFlags) {
    Remove-Item Env:CARGO_ENCODED_RUSTFLAGS -ErrorAction SilentlyContinue
  } else {
    $env:CARGO_ENCODED_RUSTFLAGS = $previousEncodedFlags
  }
}
