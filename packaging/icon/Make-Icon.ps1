# Draws the Notchy icon: the notch itself is alive. A glossy black island pill, slightly
# tilted, with one big glowing eye open and the other winking, blush and a small smile, on a
# coral-to-violet sunset tile with a sparkle. Each size is drawn on its own grid so small ones
# stay crisp; at 32 px and below it keeps just the tile, the pill and the two eyes.
#
#   powershell -ExecutionPolicy Bypass -File packaging\icon\Make-Icon.ps1
#
# Writes resources\icon-dark.png, resources\icon.png (256 px) and resources\icon-dark.ico.

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path

function New-RoundRect([single]$x, [single]$y, [single]$w, [single]$h, [single]$r) {
    $path = New-Object System.Drawing.Drawing2D.GraphicsPath
    $d = [math]::Min($r * 2, [math]::Min($w, $h))
    $path.AddArc($x, $y, $d, $d, 180, 90)
    $path.AddArc($x + $w - $d, $y, $d, $d, 270, 90)
    $path.AddArc($x + $w - $d, $y + $h - $d, $d, $d, 0, 90)
    $path.AddArc($x, $y + $h - $d, $d, $d, 90, 90)
    $path.CloseFigure()
    $path
}

# A shape with square top corners and round bottom ones: the notch.
function New-Notch([single]$x, [single]$y, [single]$w, [single]$h, [single]$r) {
    $path = New-Object System.Drawing.Drawing2D.GraphicsPath
    $d = [math]::Min($r * 2, [math]::Min($w, $h * 2))
    $path.AddLine($x, $y, $x + $w, $y)
    $path.AddArc($x + $w - $d, $y + $h - $d, $d, $d, 0, 90)
    $path.AddArc($x, $y + $h - $d, $d, $d, 90, 90)
    $path.CloseFigure()
    $path
}

function Color([int]$a, [int]$r, [int]$g, [int]$b) { [System.Drawing.Color]::FromArgb($a, $r, $g, $b) }

function Render([int]$size) {
    $bmp = New-Object System.Drawing.Bitmap $size, $size, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = 'AntiAlias'
    $g.PixelOffsetMode = 'HighQuality'
    $g.CompositingQuality = 'HighQuality'
    $g.Clear([System.Drawing.Color]::Transparent)
    $s = $size / 256.0
    $small = $size -le 32

    # The tile: a rounded square in a sunset gradient, coral at the top left to violet.
    $inset = if ($small) { 0.5 } else { 8 * $s }
    $tile = New-Object System.Drawing.RectangleF $inset, $inset, ($size - 2 * $inset), ($size - 2 * $inset)
    $bg = New-RoundRect $tile.X $tile.Y $tile.Width $tile.Height (58 * $s)
    $sunset = New-Object System.Drawing.Drawing2D.LinearGradientBrush $tile, (Color 255 255 128 92), (Color 255 120 72 236), 55.0
    $blend = New-Object System.Drawing.Drawing2D.ColorBlend 3
    $blend.Colors = @((Color 255 255 150 96), (Color 255 245 86 140), (Color 255 116 74 240))
    $blend.Positions = @([single]0, [single]0.5, [single]1)
    $sunset.InterpolationColors = $blend
    $g.FillPath($sunset, $bg)
    $g.SetClip($bg)
    if (-not $small) {
        # A soft light from the top left, like a glossy app tile.
        $shine = New-Object System.Drawing.Drawing2D.GraphicsPath
        $shine.AddEllipse((-60 * $s), (-90 * $s), (260 * $s), (200 * $s))
        $shineBrush = New-Object System.Drawing.Drawing2D.PathGradientBrush $shine
        $shineBrush.CenterColor = Color 70 255 255 255
        $shineBrush.SurroundColors = @(Color 0 255 255 255)
        $g.FillPath($shineBrush, $shine)
    }

    # The living notch: a glossy black pill, a little tilted, with a soft shadow below it.
    $cx = 128 * $s; $cy = 132 * $s
    $pillW = if ($small) { 220 * $s } else { 184 * $s }
    $pillH = if ($small) { 130 * $s } else { 104 * $s }
    $save = $g.Save()
    $g.TranslateTransform($cx, $cy)
    $g.RotateTransform($(if ($small) { 0 } else { -8 }))
    if (-not $small) {
        $shadow = New-RoundRect (-$pillW / 2 + 6 * $s) (-$pillH / 2 + 14 * $s) $pillW $pillH ($pillH / 2)
        $g.FillPath((New-Object System.Drawing.SolidBrush (Color 70 40 10 60)), $shadow)
    }
    $pill = New-RoundRect (-$pillW / 2) (-$pillH / 2) $pillW $pillH ($pillH / 2)
    $g.FillPath((New-Object System.Drawing.SolidBrush (Color 255 12 12 18)), $pill)
    if (-not $small) {
        # Gloss along the top of the pill and a thin bright rim: the edge light.
        $gloss = New-RoundRect (-$pillW / 2 + 30 * $s) (-$pillH / 2 + 7 * $s) ($pillW - 60 * $s) (12 * $s) (6 * $s)
        $g.FillPath((New-Object System.Drawing.SolidBrush (Color 18 255 255 255)), $gloss)
        $g.DrawPath((New-Object System.Drawing.Pen (Color 150 255 236 250), ([single](2.2 * $s))), $pill)
    }

    # The face: one big glowing eye open, the other winking, blush and a tiny smile.
    $glow = Color 255 140 236 255
    $eyeY = if ($small) { -16 * $s } else { -12 * $s }
    $eyeDx = if ($small) { 46 * $s } else { 38 * $s }
    $eyeW = if ($small) { 40 * $s } else { 28 * $s }
    $eyeH = if ($small) { 62 * $s } else { 44 * $s }
    # Left eye, open: a halo, then the bright capsule and a sparkle.
    if (-not $small) {
        $halo = New-Object System.Drawing.Drawing2D.GraphicsPath
        $halo.AddEllipse((-$eyeDx - 30 * $s), ($eyeY - 34 * $s), (60 * $s), (68 * $s))
        $haloBrush = New-Object System.Drawing.Drawing2D.PathGradientBrush $halo
        $haloBrush.CenterColor = Color 120 120 225 255
        $haloBrush.SurroundColors = @(Color 0 120 225 255)
        $g.FillPath($haloBrush, $halo)
    }
    $g.FillPath((New-Object System.Drawing.SolidBrush $glow), (New-RoundRect (-$eyeDx - $eyeW / 2) ($eyeY - $eyeH / 2) $eyeW $eyeH ($eyeW / 2)))
    if (-not $small) {
        $g.FillEllipse((New-Object System.Drawing.SolidBrush (Color 255 255 255 255)), (-$eyeDx + 1 * $s), ($eyeY - 14 * $s), (8 * $s), (10 * $s))
    }
    # Right eye, winking: a happy arc.
    $wink = New-Object System.Drawing.Pen $glow, ([single]$(if ($small) { 16 * $s } else { 9 * $s }))
    $wink.StartCap = 'Round'; $wink.EndCap = 'Round'
    $arcW = if ($small) { 52 * $s } else { 38 * $s }
    $g.DrawArc($wink, ($eyeDx - $arcW / 2), ($eyeY - 6 * $s), $arcW, ($arcW * 0.8), 200, 140)
    if (-not $small) {
        # Blush under each eye and a small smile.
        $blush = New-Object System.Drawing.SolidBrush (Color 170 255 120 160)
        $g.FillEllipse($blush, (-$eyeDx - 20 * $s), ($eyeY + 24 * $s), (26 * $s), (12 * $s))
        $g.FillEllipse($blush, ($eyeDx - 6 * $s), ($eyeY + 24 * $s), (26 * $s), (12 * $s))
        $smile = New-Object System.Drawing.Pen (Color 255 140 236 255), ([single](5 * $s))
        $smile.StartCap = 'Round'; $smile.EndCap = 'Round'
        $g.DrawArc($smile, (-12 * $s), ($eyeY + 18 * $s), (24 * $s), (16 * $s), 20, 140)
    }
    $g.Restore($save)

    if (-not $small) {
        # A little four-point sparkle at the top right: the "alive" moment.
        $sx = 204 * $s; $sy = 58 * $s; $r = 20 * $s; $k = 5 * $s
        $spark = New-Object System.Drawing.Drawing2D.GraphicsPath
        $spark.AddPolygon(@(
            (New-Object System.Drawing.PointF $sx, ($sy - $r)),
            (New-Object System.Drawing.PointF ($sx + $k), ($sy - $k)),
            (New-Object System.Drawing.PointF ($sx + $r), $sy),
            (New-Object System.Drawing.PointF ($sx + $k), ($sy + $k)),
            (New-Object System.Drawing.PointF $sx, ($sy + $r)),
            (New-Object System.Drawing.PointF ($sx - $k), ($sy + $k)),
            (New-Object System.Drawing.PointF ($sx - $r), $sy),
            (New-Object System.Drawing.PointF ($sx - $k), ($sy - $k))
        ))
        $g.FillPath((New-Object System.Drawing.SolidBrush (Color 245 255 255 255)), $spark)
        $g.FillEllipse((New-Object System.Drawing.SolidBrush (Color 200 255 255 255)), (226 * $s), (86 * $s), (7 * $s), (7 * $s))
    }
    $g.ResetClip()
    $g.Dispose()
    $bmp
}

function Png-Bytes($bmp) {
    $stream = New-Object System.IO.MemoryStream
    $bmp.Save($stream, [System.Drawing.Imaging.ImageFormat]::Png)
    $stream.ToArray()
}

# The 256 px PNGs for the tray and the About page.
$big = Render 256
$big.Save((Join-Path $root 'resources\icon-dark.png'), [System.Drawing.Imaging.ImageFormat]::Png)
$big.Save((Join-Path $root 'resources\icon.png'), [System.Drawing.Imaging.ImageFormat]::Png)

# A classic icon image: a 32-bit bitmap (bottom-up, height doubled for the mask) plus an
# all-clear AND mask. Every tool and every part of Windows reads these.
function Dib-Bytes($bmp) {
    $size = $bmp.Width
    $stream = New-Object System.IO.MemoryStream
    $w = New-Object System.IO.BinaryWriter $stream
    $w.Write([uint32]40); $w.Write([int32]$size); $w.Write([int32]($size * 2))
    $w.Write([uint16]1); $w.Write([uint16]32); $w.Write([uint32]0)
    $w.Write([uint32]($size * $size * 4)); $w.Write([int32]0); $w.Write([int32]0)
    $w.Write([uint32]0); $w.Write([uint32]0)
    for ($y = $size - 1; $y -ge 0; $y--) {
        for ($x = 0; $x -lt $size; $x++) {
            $c = $bmp.GetPixel($x, $y)
            $w.Write([byte]$c.B); $w.Write([byte]$c.G); $w.Write([byte]$c.R); $w.Write([byte]$c.A)
        }
    }
    $maskRow = [int]([math]::Ceiling($size / 32.0) * 4)
    $w.Write((New-Object byte[] ($maskRow * $size)))
    $w.Flush()
    $stream.ToArray()
}

# The .ico: classic bitmaps for every size, and a PNG for 256 px (as Windows itself does).
$sizes = 16, 20, 24, 32, 40, 48, 64, 128, 256
$images = New-Object 'System.Collections.Generic.List[byte[]]'
foreach ($size in $sizes) {
    $bmp = Render $size
    if ($size -ge 256) { $images.Add([byte[]](Png-Bytes $bmp)) } else { $images.Add([byte[]](Dib-Bytes $bmp)) }
}
$out = New-Object System.IO.MemoryStream
$writer = New-Object System.IO.BinaryWriter $out
$writer.Write([uint16]0); $writer.Write([uint16]1); $writer.Write([uint16]$sizes.Count)
$offset = 6 + 16 * $sizes.Count
for ($i = 0; $i -lt $sizes.Count; $i++) {
    $dim = if ($sizes[$i] -ge 256) { 0 } else { $sizes[$i] }
    $writer.Write([byte]$dim); $writer.Write([byte]$dim)
    $writer.Write([byte]0); $writer.Write([byte]0)
    $writer.Write([uint16]1); $writer.Write([uint16]32)
    $writer.Write([uint32]$images[$i].Length); $writer.Write([uint32]$offset)
    $offset += $images[$i].Length
}
foreach ($image in $images) { $writer.Write($image) }
$writer.Flush()
[IO.File]::WriteAllBytes((Join-Path $root 'resources\icon-dark.ico'), $out.ToArray())
Write-Host "Wrote icon-dark.png, icon.png and icon-dark.ico ($($sizes -join ', ') px)"
