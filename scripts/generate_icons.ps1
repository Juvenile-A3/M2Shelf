param(
  [string]$OutputDirectory = (Join-Path $PSScriptRoot "..\src-tauri\icons"),
  [string]$SourceImage = (Join-Path $PSScriptRoot "..\src-tauri\icons\icon-source.png")
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing

$expectedSourceSha256 = "027D80A3665955E59A8A443B075C03BD17116155B6E2B6872B7E3D48F36ACA1B"
$resolvedOutput = [System.IO.Path]::GetFullPath($OutputDirectory)
$resolvedSource = [System.IO.Path]::GetFullPath($SourceImage)
$canonicalSource = Join-Path $resolvedOutput "icon-source.png"

if (-not (Test-Path -LiteralPath $resolvedSource -PathType Leaf)) {
  throw "Logo source image is missing: $resolvedSource"
}

[System.IO.Directory]::CreateDirectory($resolvedOutput) | Out-Null
if (-not $resolvedSource.Equals($canonicalSource, [System.StringComparison]::OrdinalIgnoreCase)) {
  Copy-Item -LiteralPath $resolvedSource -Destination $canonicalSource -Force
  $resolvedSource = $canonicalSource
}

$actualSourceSha256 = (Get-FileHash -LiteralPath $resolvedSource -Algorithm SHA256).Hash
if ($actualSourceSha256 -ne $expectedSourceSha256) {
  throw "Logo source differs from the user-approved image. Expected $expectedSourceSha256, got $actualSourceSha256"
}

$sourceBitmap = [System.Drawing.Bitmap]::FromFile($resolvedSource)
try {
  if ($sourceBitmap.Width -ne $sourceBitmap.Height) {
    throw "Logo source must be square, got $($sourceBitmap.Width)x$($sourceBitmap.Height)"
  }

  function New-LogoBitmap {
    param([int]$Size)

    $bitmap = [System.Drawing.Bitmap]::new($Size, $Size, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $attributes = [System.Drawing.Imaging.ImageAttributes]::new()
    try {
      # Scale the complete approved transparent-corner source only. Do not crop,
      # recolour, redraw, sharpen, or otherwise alter its design.
      $graphics.CompositingMode = [System.Drawing.Drawing2D.CompositingMode]::SourceCopy
      $graphics.CompositingQuality = [System.Drawing.Drawing2D.CompositingQuality]::HighQuality
      $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
      $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
      $graphics.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
      $attributes.SetWrapMode([System.Drawing.Drawing2D.WrapMode]::TileFlipXY)
      $destination = [System.Drawing.Rectangle]::new(0, 0, $Size, $Size)
      $graphics.DrawImage(
        $sourceBitmap,
        $destination,
        0,
        0,
        $sourceBitmap.Width,
        $sourceBitmap.Height,
        [System.Drawing.GraphicsUnit]::Pixel,
        $attributes
      )
    } finally {
      $attributes.Dispose()
      $graphics.Dispose()
    }
    return $bitmap
  }

  function Save-Png {
    param([int]$Size, [string]$Name)

    $bitmap = New-LogoBitmap -Size $Size
    try {
      $bitmap.Save((Join-Path $resolvedOutput $Name), [System.Drawing.Imaging.ImageFormat]::Png)
    } finally {
      $bitmap.Dispose()
    }
  }

  function Write-Ico {
    param([string]$Path, [int[]]$Sizes)

    $streams = @()
    try {
      foreach ($size in $Sizes) {
        $bitmap = New-LogoBitmap -Size $size
        $stream = [System.IO.MemoryStream]::new()
        try {
          $bitmap.Save($stream, [System.Drawing.Imaging.ImageFormat]::Png)
        } finally {
          $bitmap.Dispose()
        }
        $streams += ,$stream
      }

      $file = [System.IO.File]::Open($Path, [System.IO.FileMode]::Create, [System.IO.FileAccess]::Write)
      $writer = [System.IO.BinaryWriter]::new($file)
      try {
        $writer.Write([uint16]0)
        $writer.Write([uint16]1)
        $writer.Write([uint16]$Sizes.Count)
        $offset = 6 + (16 * $Sizes.Count)
        for ($index = 0; $index -lt $Sizes.Count; $index++) {
          $size = $Sizes[$index]
          $data = $streams[$index].ToArray()
          $writer.Write([byte]$(if ($size -eq 256) { 0 } else { $size }))
          $writer.Write([byte]$(if ($size -eq 256) { 0 } else { $size }))
          $writer.Write([byte]0)
          $writer.Write([byte]0)
          $writer.Write([uint16]1)
          $writer.Write([uint16]32)
          $writer.Write([uint32]$data.Length)
          $writer.Write([uint32]$offset)
          $offset += $data.Length
        }
        foreach ($stream in $streams) {
          $writer.Write($stream.ToArray())
        }
      } finally {
        $writer.Dispose()
        $file.Dispose()
      }
    } finally {
      foreach ($stream in $streams) {
        $stream.Dispose()
      }
    }
  }

  $svg = @"
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 $($sourceBitmap.Width) $($sourceBitmap.Height)" role="img" aria-label="M squared">
  <!-- Exact wrapper around the approved raster master. It is intentionally not traced or redesigned. -->
  <image href="icon-source.png" x="0" y="0" width="$($sourceBitmap.Width)" height="$($sourceBitmap.Height)" preserveAspectRatio="xMidYMid meet"/>
</svg>
"@
  [System.IO.File]::WriteAllText((Join-Path $resolvedOutput "icon.svg"), $svg, [System.Text.UTF8Encoding]::new($false))

  Save-Png -Size 16 -Name "16x16.png"
  Save-Png -Size 24 -Name "24x24.png"
  Save-Png -Size 32 -Name "32x32.png"
  Save-Png -Size 48 -Name "48x48.png"
  Save-Png -Size 64 -Name "64x64.png"
  Save-Png -Size 128 -Name "128x128.png"
  Save-Png -Size 256 -Name "256x256.png"
  Save-Png -Size 256 -Name "128x128@2x.png"
  Save-Png -Size 512 -Name "icon.png"
  Save-Png -Size 512 -Name "512x512.png"
  Save-Png -Size 1024 -Name "icon-master.png"
  # Tauri 2.6.x decodes the first ICO directory entry into its runtime
  # default_window_icon. Keep the largest authored frame first so the taskbar
  # receives full-resolution RGBA data instead of stretching the 16 px frame.
  # Windows resource loading can still select every embedded native size.
  Write-Ico -Path (Join-Path $resolvedOutput "icon.ico") -Sizes @(256, 128, 64, 48, 32, 24, 16)
} finally {
  $sourceBitmap.Dispose()
}

Write-Output "Generated M²Shelf icons from the exact approved raster source in $resolvedOutput"
Write-Output "Source SHA-256: $actualSourceSha256"
