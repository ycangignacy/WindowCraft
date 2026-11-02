# Made by ycangignacy
<#
.SYNOPSIS
    Regenerates every icon and installer image from assets\logo-source.jpg.

.DESCRIPTION
    Outputs into crates\windowcraft-app\assets:
      windowcraft.ico      exe, taskbar, Start menu, MSI and setup.exe
      icon-64.rgba         window icon, read by src\lib.rs
      logo-64.png          setup.exe screen
      logo-256.png         README
      wix-banner.bmp       MSI dialog banner, 493x58
      wix-dialog.bmp       MSI welcome and finish panel, 493x312
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies System.Drawing, System.Core -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Drawing.Imaging;
using System.Linq;
using System.Runtime.InteropServices;

public static class LogoTools {
    public static Rectangle FindPlate(Bitmap source) {
        Func<Color, bool> plate = c => c.B > 60 && c.B > c.R + 15;
        int middle = source.Width / 2, band = source.Height / 5;
        int left = 0, top = 0, right = source.Width - 1, bottom = source.Height - 1;
        while (!plate(source.GetPixel(left, band))) left++;
        while (!plate(source.GetPixel(middle, top))) top++;
        while (!plate(source.GetPixel(right, band))) right--;
        while (!plate(source.GetPixel(middle, bottom))) bottom--;
        return Rectangle.FromLTRB(left, top, right + 1, bottom + 1);
    }

    static GraphicsPath Rounded(RectangleF area, float radius) {
        var path = new GraphicsPath();
        float d = radius * 2;
        path.AddArc(area.Left, area.Top, d, d, 180, 90);
        path.AddArc(area.Right - d, area.Top, d, d, 270, 90);
        path.AddArc(area.Right - d, area.Bottom - d, d, d, 0, 90);
        path.AddArc(area.Left, area.Bottom - d, d, d, 90, 90);
        path.CloseFigure();
        return path;
    }

    // Scales one crop of the source onto a rounded plate. `crop` is in source
    // pixels and `radius` is a fraction of the output size.
    public static Bitmap Render(Bitmap source, Rectangle crop, int size, float radius) {
        var scaled = new Bitmap(size, size, PixelFormat.Format32bppArgb);
        using (var g = Graphics.FromImage(scaled)) {
            g.InterpolationMode = InterpolationMode.HighQualityBicubic;
            g.PixelOffsetMode = PixelOffsetMode.HighQuality;
            using (var attributes = new ImageAttributes()) {
                attributes.SetWrapMode(WrapMode.TileFlipXY);
                g.DrawImage(source, new Rectangle(0, 0, size, size), crop.X, crop.Y, crop.Width, crop.Height, GraphicsUnit.Pixel, attributes);
            }
        }
        var result = new Bitmap(size, size, PixelFormat.Format32bppArgb);
        using (var g = Graphics.FromImage(result))
        using (var brush = new TextureBrush(scaled))
        using (var path = Rounded(new RectangleF(0, 0, size, size), size * radius)) {
            g.SmoothingMode = SmoothingMode.AntiAlias;
            g.PixelOffsetMode = PixelOffsetMode.HighQuality;
            g.FillPath(brush, path);
        }
        scaled.Dispose();
        return result;
    }

    public static byte[] Bgra(Bitmap bitmap) {
        var area = new Rectangle(0, 0, bitmap.Width, bitmap.Height);
        var data = bitmap.LockBits(area, ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
        var bytes = new byte[bitmap.Width * bitmap.Height * 4];
        for (int y = 0; y < bitmap.Height; y++)
            Marshal.Copy(data.Scan0 + y * data.Stride, bytes, y * bitmap.Width * 4, bitmap.Width * 4);
        bitmap.UnlockBits(data);
        return bytes;
    }

    public static byte[] Rgba(Bitmap bitmap) {
        var bytes = Bgra(bitmap);
        for (int i = 0; i < bytes.Length; i += 4) {
            byte blue = bytes[i];
            bytes[i] = bytes[i + 2];
            bytes[i + 2] = blue;
        }
        return bytes;
    }

    // Eight bits with a palette built from the image. The stock halftone
    // palette has no dark blue, so it turns the plate teal.
    public static Bitmap Indexed(Bitmap source) {
        var pixels = Bgra(source);
        Func<int, int> key = i => (pixels[i + 2] >> 3) << 10 | (pixels[i + 1] >> 3) << 5 | (pixels[i] >> 3);
        var counts = new Dictionary<int, int>();
        for (int i = 0; i < pixels.Length; i += 4) {
            int bucket = key(i), seen;
            counts.TryGetValue(bucket, out seen);
            counts[bucket] = seen + 1;
        }
        var chosen = counts.OrderByDescending(pair => pair.Value).Take(256).Select(pair => pair.Key).ToArray();
        Func<int, int> red = k => (k >> 10 & 31) * 255 / 31;
        Func<int, int> green = k => (k >> 5 & 31) * 255 / 31;
        Func<int, int> blue = k => (k & 31) * 255 / 31;
        var result = new Bitmap(source.Width, source.Height, PixelFormat.Format8bppIndexed);
        var palette = result.Palette;
        for (int i = 0; i < chosen.Length; i++)
            palette.Entries[i] = Color.FromArgb(red(chosen[i]), green(chosen[i]), blue(chosen[i]));
        result.Palette = palette;
        var nearest = new Dictionary<int, byte>();
        var data = result.LockBits(new Rectangle(0, 0, result.Width, result.Height), ImageLockMode.WriteOnly, PixelFormat.Format8bppIndexed);
        var row = new byte[result.Width];
        for (int y = 0; y < result.Height; y++) {
            for (int x = 0; x < result.Width; x++) {
                int bucket = key((y * result.Width + x) * 4);
                byte index;
                if (!nearest.TryGetValue(bucket, out index)) {
                    int best = int.MaxValue;
                    for (int i = 0; i < chosen.Length; i++) {
                        int dr = red(chosen[i]) - red(bucket), dg = green(chosen[i]) - green(bucket), db = blue(chosen[i]) - blue(bucket);
                        int distance = dr * dr + dg * dg + db * db;
                        if (distance < best) { best = distance; index = (byte)i; }
                    }
                    nearest[bucket] = index;
                }
                row[x] = index;
            }
            Marshal.Copy(row, 0, data.Scan0 + y * data.Stride, row.Length);
        }
        result.UnlockBits(data);
        return result;
    }

    // Bottom-up 32-bit DIB with an empty AND mask, as ICO frames expect.
    public static byte[] IconDib(Bitmap bitmap) {
        int size = bitmap.Width, mask = ((size + 31) / 32) * 4 * size;
        var pixels = Bgra(bitmap);
        var dib = new byte[40 + pixels.Length + mask];
        BitConverter.GetBytes(40).CopyTo(dib, 0);
        BitConverter.GetBytes(size).CopyTo(dib, 4);
        BitConverter.GetBytes(size * 2).CopyTo(dib, 8);
        BitConverter.GetBytes((short)1).CopyTo(dib, 12);
        BitConverter.GetBytes((short)32).CopyTo(dib, 14);
        BitConverter.GetBytes(pixels.Length).CopyTo(dib, 20);
        for (int y = 0; y < size; y++)
            Array.Copy(pixels, (size - 1 - y) * size * 4, dib, 40 + y * size * 4, size * 4);
        return dib;
    }
}
'@

$assets = Join-Path (Split-Path -Parent $PSScriptRoot) 'crates\windowcraft-app\assets'
$source = [System.Drawing.Bitmap]::FromFile((Join-Path $assets 'logo-source.jpg'))
$plate = [LogoTools]::FindPlate($source)
Write-Host "Plate: $plate"
$radius = 0.15

# The head alone reads at small sizes, where the full drawing turns to noise.
$headSize = [int]($plate.Width * 0.62)
$head = New-Object System.Drawing.Rectangle (($plate.Right - $headSize - [int]($plate.Width * 0.015)), ($plate.Top + [int]($plate.Height * 0.09)), $headSize, $headSize)

function Save-Png([System.Drawing.Bitmap] $Bitmap, [string] $Name) {
    $Bitmap.Save((Join-Path $assets $Name), [System.Drawing.Imaging.ImageFormat]::Png)
}

function Get-PngBytes([System.Drawing.Bitmap] $Bitmap) {
    $stream = New-Object System.IO.MemoryStream
    $Bitmap.Save($stream, [System.Drawing.Imaging.ImageFormat]::Png)
    , $stream.ToArray()
}

$frames = New-Object System.Collections.ArrayList
foreach ($size in 16, 20, 24, 32, 40, 48, 64, 256) {
    $crop = if ($size -le 32) { $head } else { $plate }
    $bitmap = [LogoTools]::Render($source, $crop, $size, $radius)
    # Only 256 is PNG: many ICO readers take a smaller PNG frame for a DIB.
    $bytes = if ($size -eq 256) { Get-PngBytes $bitmap } else { [LogoTools]::IconDib($bitmap) }
    [void]$frames.Add(@{ Size = $size; Bytes = [byte[]]$bytes })
    if ($size -eq 64) {
        [IO.File]::WriteAllBytes((Join-Path $assets 'icon-64.rgba'), [LogoTools]::Rgba($bitmap))
        Save-Png $bitmap 'logo-64.png'
    }
    if ($size -eq 256) { Save-Png $bitmap 'logo-256.png' }
    $bitmap.Dispose()
}

$icon = New-Object System.IO.MemoryStream
$writer = New-Object System.IO.BinaryWriter $icon
$writer.Write([uint16]0); $writer.Write([uint16]1); $writer.Write([uint16]$frames.Count)
$offset = 6 + 16 * $frames.Count
foreach ($frame in $frames) {
    $writer.Write([byte]($frame.Size % 256)); $writer.Write([byte]($frame.Size % 256))
    $writer.Write([byte]0); $writer.Write([byte]0)
    $writer.Write([uint16]1); $writer.Write([uint16]32)
    $writer.Write([uint32]$frame.Bytes.Length); $writer.Write([uint32]$offset)
    $offset += $frame.Bytes.Length
}
foreach ($frame in $frames) { $writer.Write($frame.Bytes) }
[IO.File]::WriteAllBytes((Join-Path $assets 'windowcraft.ico'), $icon.ToArray())

function Save-WixBitmap([int] $Width, [int] $Height, [System.Drawing.Rectangle] $Logo, [string] $Name) {
    $canvas = New-Object System.Drawing.Bitmap $Width, $Height, ([System.Drawing.Imaging.PixelFormat]::Format24bppRgb)
    $graphics = [System.Drawing.Graphics]::FromImage($canvas)
    $graphics.Clear([System.Drawing.Color]::White)
    $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $mark = [LogoTools]::Render($source, $plate, $Logo.Width, $radius)
    $graphics.DrawImage($mark, $Logo)
    $graphics.Dispose()
    $mark.Dispose()
    # Eight bits keep each image small inside the MSI.
    $indexed = [LogoTools]::Indexed($canvas)
    $indexed.Save((Join-Path $assets $Name), [System.Drawing.Imaging.ImageFormat]::Bmp)
    $indexed.Dispose()
    $canvas.Dispose()
}

# WixUI draws its text from x=180 on the panel, and over the left of the banner.
Save-WixBitmap 493 58 (New-Object System.Drawing.Rectangle 441, 6, 46, 46) 'wix-banner.bmp'
Save-WixBitmap 493 312 (New-Object System.Drawing.Rectangle 12, 94, 148, 148) 'wix-dialog.bmp'

$source.Dispose()
Get-ChildItem $assets | Select-Object Name, Length | Format-Table -AutoSize
