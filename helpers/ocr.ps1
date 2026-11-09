# MacroFlow OCR helper for Windows.
# Reads text from one or more images with the built-in Windows OCR engine
# (Windows.Media.Ocr) and prints each result, one line per row of text, with a
# <<<MACROFLOW-NEXT>>> line between images.
#
#   powershell -File ocr.ps1 <image1.png> [image2.png ...]
#
# MacroFlow embeds this script and runs it automatically. It needs Windows 10 or 11
# and Windows PowerShell 5.1 (included with Windows).
# (Keep this file plain ASCII: Windows PowerShell 5.1 reads scripts as ANSI.)

param([Parameter(ValueFromRemainingArguments = $true)][string[]]$ImagePaths)

$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

if (-not $ImagePaths -or $ImagePaths.Count -eq 0) {
    [Console]::Error.WriteLine('usage: ocr.ps1 <image> [image ...]')
    exit 2
}

Add-Type -AssemblyName System.Runtime.WindowsRuntime

$asTaskGeneric = ([System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object {
        $_.Name -eq 'AsTask' -and
        $_.GetParameters().Count -eq 1 -and
        $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1'
    })[0]

function Await($WinRtTask, $ResultType) {
    $asTask = $asTaskGeneric.MakeGenericMethod($ResultType)
    $netTask = $asTask.Invoke($null, @($WinRtTask))
    $netTask.Wait(-1) | Out-Null
    $netTask.Result
}

[Windows.Storage.StorageFile, Windows.Storage, ContentType = WindowsRuntime] | Out-Null
[Windows.Media.Ocr.OcrEngine, Windows.Foundation, ContentType = WindowsRuntime] | Out-Null
[Windows.Graphics.Imaging.BitmapDecoder, Windows.Graphics, ContentType = WindowsRuntime] | Out-Null
[Windows.Graphics.Imaging.SoftwareBitmap, Windows.Graphics, ContentType = WindowsRuntime] | Out-Null

$engine = [Windows.Media.Ocr.OcrEngine]::TryCreateFromUserProfileLanguages()
if ($null -eq $engine) {
    $lang = New-Object Windows.Globalization.Language 'en-US'
    $engine = [Windows.Media.Ocr.OcrEngine]::TryCreateFromLanguage($lang)
}
if ($null -eq $engine) {
    [Console]::Error.WriteLine('No OCR language is installed')
    exit 1
}

$first = $true
foreach ($path in $ImagePaths) {
    if (-not $first) { [Console]::Out.WriteLine('<<<MACROFLOW-NEXT>>>') }
    $first = $false
    try {
        $fullPath = (Resolve-Path -LiteralPath $path).Path
        $file = Await ([Windows.Storage.StorageFile]::GetFileFromPathAsync($fullPath)) ([Windows.Storage.StorageFile])
        $stream = Await ($file.OpenAsync([Windows.Storage.FileAccessMode]::Read)) ([Windows.Storage.Streams.IRandomAccessStream])
        $decoder = Await ([Windows.Graphics.Imaging.BitmapDecoder]::CreateAsync($stream)) ([Windows.Graphics.Imaging.BitmapDecoder])
        $bitmap = Await ($decoder.GetSoftwareBitmapAsync()) ([Windows.Graphics.Imaging.SoftwareBitmap])
        # The OCR engine wants Bgra8 or Gray8 bitmaps.
        $bitmap = [Windows.Graphics.Imaging.SoftwareBitmap]::Convert($bitmap, [Windows.Graphics.Imaging.BitmapPixelFormat]::Bgra8, [Windows.Graphics.Imaging.BitmapAlphaMode]::Premultiplied)
        $result = Await ($engine.RecognizeAsync($bitmap)) ([Windows.Media.Ocr.OcrResult])
        $stream.Dispose()
        foreach ($line in $result.Lines) {
            [Console]::Out.WriteLine($line.Text)
        }
    } catch {
        # One bad image shouldn't lose the others: leave this result empty.
        [Console]::Error.WriteLine($_.Exception.Message)
    }
}
exit 0
