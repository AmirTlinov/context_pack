$ErrorActionPreference = "Stop"

$Repo = if ($env:CONTEXT_PACK_REPO) { $env:CONTEXT_PACK_REPO } else { "AmirTlinov/context_pack" }
$Version = if ($env:CONTEXT_PACK_VERSION) { $env:CONTEXT_PACK_VERSION } else { "latest" }
$InstallDir = if ($env:CONTEXT_PACK_INSTALL_DIR) { $env:CONTEXT_PACK_INSTALL_DIR } else { Join-Path $HOME ".local\bin" }
$BinaryName = "mcp-context-pack.exe"

function Resolve-Target {
    $arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
    switch ($arch.ToString()) {
        "X64" { return "x86_64-pc-windows-msvc" }
        default { throw "Unsupported architecture '$arch' (supported: X64)." }
    }
}

function Resolve-Tag {
    param([string]$Repo, [string]$Version)

    if ($Version -ne "latest") {
        return $Version
    }

    $api = "https://api.github.com/repos/$Repo/releases/latest"
    $json = Invoke-RestMethod -Uri $api -Method Get
    if (-not $json.tag_name) {
        throw "Failed to resolve latest release tag for '$Repo'."
    }
    return $json.tag_name
}

$target = Resolve-Target
$tag = Resolve-Tag -Repo $Repo -Version $Version
$archiveName = "mcp-context-pack-$target.zip"
$checksumsName = "checksums.sha256"
$downloadUrl = "https://github.com/$Repo/releases/download/$tag/$archiveName"
$checksumsUrl = "https://github.com/$Repo/releases/download/$tag/$checksumsName"

$tmpDir = Join-Path ([System.IO.Path]::GetTempPath()) ([System.Guid]::NewGuid().ToString())
New-Item -Path $tmpDir -ItemType Directory -Force | Out-Null

try {
    $archivePath = Join-Path $tmpDir $archiveName
    $checksumsPath = Join-Path $tmpDir $checksumsName
    Write-Host "→ Downloading $archiveName ($tag) from $Repo"
    Invoke-WebRequest -Uri $downloadUrl -OutFile $archivePath
    Invoke-WebRequest -Uri $checksumsUrl -OutFile $checksumsPath

    $checksumsContent = Get-Content -Path $checksumsPath
    $line = $checksumsContent | Where-Object { $_ -match [regex]::Escape($archiveName) + '$' } | Select-Object -First 1
    if (-not $line) {
        throw "Checksum for '$archiveName' not found in '$checksumsName'."
    }

    $expectedHash = ($line -split '\s+')[0].ToLowerInvariant()
    $actualHash = (Get-FileHash -Path $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualHash -ne $expectedHash) {
        throw "Checksum mismatch for '$archiveName'. Expected '$expectedHash', got '$actualHash'."
    }

    Expand-Archive -Path $archivePath -DestinationPath $tmpDir -Force
    $binaryPath = Join-Path $tmpDir $BinaryName
    if (-not (Test-Path $binaryPath)) {
        throw "Binary '$BinaryName' not found in archive '$archiveName'."
    }

    New-Item -Path $InstallDir -ItemType Directory -Force | Out-Null
    Copy-Item -Path $binaryPath -Destination (Join-Path $InstallDir $BinaryName) -Force

    Write-Host "✓ Installed $BinaryName to $InstallDir"
    Write-Host "  If needed, add this directory to PATH:"
    Write-Host "  $InstallDir"
}
finally {
    Remove-Item -Path $tmpDir -Recurse -Force -ErrorAction SilentlyContinue
}
