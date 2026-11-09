param(
    [string]$Target = "target\debug",
    [string]$Root = $env:GSTREAMER_1_0_ROOT_MSVC_X86_64
)

$ErrorActionPreference = "Stop"

if (-not $Root) {
    $Root = "C:\gstreamer\1.0\msvc_x86_64\"
}
if (-not (Test-Path (Join-Path $Root "bin"))) {
    throw "GStreamer runtime not found in $Root. Install gstreamer-1.0-msvc-x86_64 or set GSTREAMER_1_0_ROOT_MSVC_X86_64."
}

$plugins = @(
    "gstcoreelements", "gstnice", "gstapp", "gstaudioconvert", "gstaudioresample", "gstgio",
    "gstogg", "gstopengl", "gstopus", "gstplayback", "gsttheora", "gsttypefindfunctions",
    "gstvideoconvertscale", "gstvolume", "gstvorbis", "gstaudiofx", "gstaudioparsers",
    "gstautodetect", "gstdeinterlace", "gstid3demux", "gstinterleave", "gstisomp4",
    "gstmatroska", "gstrtp", "gstrtpmanager", "gstvideofilter", "gstvpx", "gstwavparse",
    "gstaudiobuffersplit", "gstdtls", "gstid3tag", "gstproxy", "gstvideoparsersbad",
    "gstwebrtc", "gstlibav", "gstwasapi"
)

New-Item -ItemType Directory -Force -Path $Target | Out-Null
Copy-Item -Path (Join-Path $Root "bin\*.dll") -Destination $Target -Force

$missing = @()
foreach ($plugin in $plugins) {
    $source = Join-Path $Root "lib\gstreamer-1.0\$plugin.dll"
    if (Test-Path $source) {
        Copy-Item -Path $source -Destination $Target -Force
    } else {
        $missing += $plugin
    }
}

if ($missing.Count -gt 0) {
    Write-Warning ("GStreamer plugins not found: " + ($missing -join ", "))
}
Write-Host "GStreamer copied to $Target"
