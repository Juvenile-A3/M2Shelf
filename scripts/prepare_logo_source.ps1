param(
  [string]$InputImage = (Join-Path $PSScriptRoot "..\src-tauri\icons\logo-input-original.png"),
  [string]$OutputImage = (Join-Path $PSScriptRoot "..\src-tauri\icons\icon-source.png")
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing

$expectedInputSha256 = "AB6B86469FEE688E03468A160820F2A4E209B9904C0F7C239FC6918222295B72"
$resolvedInput = [System.IO.Path]::GetFullPath($InputImage)
$resolvedOutput = [System.IO.Path]::GetFullPath($OutputImage)

if (-not (Test-Path -LiteralPath $resolvedInput -PathType Leaf)) {
  throw "Original Logo input is missing: $resolvedInput"
}
$actualInputSha256 = (Get-FileHash -LiteralPath $resolvedInput -Algorithm SHA256).Hash
if ($actualInputSha256 -ne $expectedInputSha256) {
  throw "Original Logo input changed. Expected $expectedInputSha256, got $actualInputSha256"
}

$cornerCleaner = @'
using System;
using System.Drawing;
using System.Drawing.Imaging;

public static class M2ShelfLogoCornerCleaner
{
    private const int InteriorRedThreshold = 140;
    private const int MatteNoiseThreshold = 8;
    private const int InteriorReferenceOffset = 8;

    public static void RemoveBlackCorners(string inputPath, string outputPath)
    {
        using (var input = new Bitmap(inputPath))
        {
            if (input.Width != input.Height)
                throw new InvalidOperationException($"Logo input must be square, got {input.Width}x{input.Height}");

            using (var output = new Bitmap(input.Width, input.Height, PixelFormat.Format32bppArgb))
            {
                for (var y = 0; y < input.Height; y++)
                {
                    var left = 0;
                    while (left < input.Width && input.GetPixel(left, y).R < InteriorRedThreshold)
                        left++;

                    var right = input.Width - 1;
                    while (right >= 0 && input.GetPixel(right, y).R < InteriorRedThreshold)
                        right--;

                    if (left > right)
                        continue;

                    var leftReference = input.GetPixel(Math.Min(right, left + InteriorReferenceOffset), y);
                    var rightReference = input.GetPixel(Math.Max(left, right - InteriorReferenceOffset), y);

                    for (var x = 0; x < left; x++)
                        output.SetPixel(x, y, RemoveBlackMatte(input.GetPixel(x, y), leftReference));

                    for (var x = left; x <= right; x++)
                    {
                        var pixel = input.GetPixel(x, y);
                        output.SetPixel(x, y, Color.FromArgb(255, pixel.R, pixel.G, pixel.B));
                    }

                    for (var x = right + 1; x < input.Width; x++)
                        output.SetPixel(x, y, RemoveBlackMatte(input.GetPixel(x, y), rightReference));
                }

                output.Save(outputPath, ImageFormat.Png);
            }
        }
    }

    private static Color RemoveBlackMatte(Color sample, Color reference)
    {
        if (sample.R < MatteNoiseThreshold && sample.G < MatteNoiseThreshold && sample.B < MatteNoiseThreshold)
            return Color.Transparent;

        var alpha = Math.Max(0.0, Math.Min(1.0, sample.R / (double)Math.Max(1, (int)reference.R)));
        if (alpha < 0.03)
            return Color.Transparent;

        return Color.FromArgb(
            (int)Math.Round(alpha * 255.0),
            reference.R,
            reference.G,
            reference.B
        );
    }
}
'@

if (-not ("M2ShelfLogoCornerCleaner" -as [type])) {
  $drawingCommonPath = [System.Drawing.Bitmap].Assembly.Location
  $drawingPrimitivesPath = [System.Drawing.Color].Assembly.Location
  $drawingRuntimeDirectory = Split-Path -Parent $drawingCommonPath
  $drawingReferences = @(
    $drawingCommonPath,
    $drawingPrimitivesPath,
    (Join-Path $drawingRuntimeDirectory "System.Private.Windows.GdiPlus.dll"),
    (Join-Path $drawingRuntimeDirectory "System.Private.Windows.Core.dll")
  ) | Select-Object -Unique
  Add-Type -TypeDefinition $cornerCleaner -ReferencedAssemblies $drawingReferences
}

$outputDirectory = Split-Path -Parent $resolvedOutput
[System.IO.Directory]::CreateDirectory($outputDirectory) | Out-Null
$temporaryOutput = Join-Path $outputDirectory ("." + [System.IO.Path]::GetFileName($resolvedOutput) + ".tmp.png")
try {
  [M2ShelfLogoCornerCleaner]::RemoveBlackCorners($resolvedInput, $temporaryOutput)
  Move-Item -LiteralPath $temporaryOutput -Destination $resolvedOutput -Force
} finally {
  if (Test-Path -LiteralPath $temporaryOutput) {
    Remove-Item -LiteralPath $temporaryOutput -Force
  }
}

$outputHash = (Get-FileHash -LiteralPath $resolvedOutput -Algorithm SHA256).Hash
Write-Output "Prepared transparent-corner Logo source: $resolvedOutput"
Write-Output "Input SHA-256: $actualInputSha256"
Write-Output "Output SHA-256: $outputHash"
