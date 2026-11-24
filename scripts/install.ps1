# Install or update HaryX for Windows (the tray app with the bundled haryx CLI) from GitHub releases:
#
#   irm https://raw.githubusercontent.com/conghai20/haryx/main/scripts/install.ps1 | iex
#
# PowerShell does not mark what it downloads as "from the internet", so the unsigned installer runs
# without the SmartScreen prompt a browser download gets. The installer is checked against the
# release's SHA-256 file before it runs. It installs for the current user only; no admin rights.
#
# Environment (all optional):
#   HARYX_VERSION    install this version (e.g. 0.1.3) instead of the latest release
#   HARYX_NO_OPEN=1  do not start HaryX after installing
#   HARYX_BASE_URL   releases base URL (default https://github.com/conghai20/haryx/releases)

$ErrorActionPreference = 'Stop'
# Windows PowerShell 5.1 downloads many times slower while it draws a progress bar.
$ProgressPreference = 'SilentlyContinue'
Set-StrictMode -Version Latest

$DefaultBaseUrl = 'https://github.com/conghai20/haryx/releases'

# Throw rather than exit: under `irm ... | iex` an exit would close the user's PowerShell window.
function Fail([string]$Message) {
    throw "haryx install: $Message"
}

function Get-Arch {
    switch ($env:PROCESSOR_ARCHITECTURE) {
        'AMD64' { return 'x64' }
        'ARM64' { return 'arm64' }
        default { Fail "no HaryX build for $($env:PROCESSOR_ARCHITECTURE) Windows." }
    }
}

# The latest version, from where /releases/latest redirects (no API call, no rate limit).
function Resolve-Version([string]$BaseUrl) {
    if ($env:HARYX_VERSION) { return $env:HARYX_VERSION.TrimStart('v') }
    try {
        $response = Invoke-WebRequest -Uri "$BaseUrl/latest" -Method Head -UseBasicParsing -MaximumRedirection 5
        # Windows PowerShell 5.1 returns an HttpWebResponse (ResponseUri); PowerShell 7 an
        # HttpResponseMessage (RequestMessage.RequestUri). Strict mode rejects the missing one.
        $base = $response.BaseResponse
        $final = if ($base.PSObject.Properties['ResponseUri']) { $base.ResponseUri } else { $base.RequestMessage.RequestUri }
    } catch {
        Fail "cannot reach $BaseUrl/latest"
    }
    $tag = ($final.AbsoluteUri -split '/tag/')[-1]
    if (-not $tag -or $tag -eq $final.AbsoluteUri) { Fail "no published release found at $BaseUrl" }
    return $tag.TrimStart('v')
}

function Get-File([string]$Url, [string]$Destination) {
    try {
        Invoke-WebRequest -Uri $Url -OutFile $Destination -UseBasicParsing
    } catch {
        Fail "download failed: $Url"
    }
}

# The checksum file's first field is the hash; its file name may be a build path, so it is ignored.
function Test-Checksum([string]$File, [string]$ShaFile) {
    $expected = ((Get-Content -Path $ShaFile -TotalCount 1) -split '\s+')[0].ToLowerInvariant()
    $actual = (Get-FileHash -Path $File -Algorithm SHA256).Hash.ToLowerInvariant()
    if (-not $expected -or $expected -ne $actual) {
        Fail "checksum mismatch for $(Split-Path $File -Leaf); nothing was installed."
    }
}

function Main {
    $base = if ($env:HARYX_BASE_URL) { $env:HARYX_BASE_URL } else { $DefaultBaseUrl }
    $arch = Get-Arch
    $version = Resolve-Version $base
    $name = "HaryX-$version-$arch-setup.exe"
    $work = Join-Path ([IO.Path]::GetTempPath()) ("haryx-" + [Guid]::NewGuid())
    New-Item -ItemType Directory -Path $work | Out-Null
    try {
        Write-Host "Downloading HaryX $version ($arch)..."
        $installer = Join-Path $work $name
        Get-File "$base/download/v$version/$name" $installer
        Get-File "$base/download/v$version/$name.sha256" "$installer.sha256"
        Test-Checksum $installer "$installer.sha256"

        # A running HaryX is closed by the installer itself (NSIS asks it to quit).
        Write-Host 'Installing...'
        $process = Start-Process -FilePath $installer -ArgumentList '/S' -Wait -PassThru
        if ($process.ExitCode -ne 0) { Fail "the installer exited with code $($process.ExitCode)" }

        $exe = Join-Path $env:LOCALAPPDATA 'HaryX\HaryX.exe'
        Write-Host "Installed HaryX $version"
        if ($env:HARYX_NO_OPEN -ne '1' -and (Test-Path $exe)) { Start-Process -FilePath $exe }
    } finally {
        Remove-Item -Recurse -Force -Path $work -ErrorAction SilentlyContinue
    }
}

# Everything runs from here, so a partly downloaded script does nothing.
Main
