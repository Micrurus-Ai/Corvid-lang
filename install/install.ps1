# Corvid installer for Windows.
#
# One-line invocation:
#   irm https://raw.githubusercontent.com/Micrurus-Ai/Corvid-lang/main/install/install.ps1 | iex
#
# Env var overrides:
#   $env:CORVID_REPO     = "Micrurus-Ai/Corvid-lang"          # source repo
#   $env:CORVID_VERSION  = "latest" | "nightly" | "v0.0.1"    # release tag (slice 35V2-P33-install-script-nightly)
#   $env:CORVID_HOME     = "$env:USERPROFILE\.corvid"         # install root

$ErrorActionPreference = 'Stop'

if ($PSVersionTable.PSVersion.Major -lt 5) {
    Write-Error "PowerShell 5.1 or later required. Current: $($PSVersionTable.PSVersion)"
}

# --- config ---------------------------------------------------------------
$Repo    = if ($env:CORVID_REPO)    { $env:CORVID_REPO }    else { 'Micrurus-Ai/Corvid-lang' }
$Version = if ($env:CORVID_VERSION) { $env:CORVID_VERSION } else { 'latest' }
$Root    = if ($env:CORVID_HOME)    { $env:CORVID_HOME }    else { Join-Path $env:USERPROFILE '.corvid' }

# --- helpers --------------------------------------------------------------
function Step($m) { Write-Host "==> $m" -ForegroundColor Cyan }
function Ok($m)   { Write-Host "    $m" -ForegroundColor Green }
function Warn2($m){ Write-Host "    $m" -ForegroundColor Yellow }

function Have($name) { [bool](Get-Command $name -ErrorAction SilentlyContinue) }

# --- target detection -----------------------------------------------------
$arch = switch ($env:PROCESSOR_ARCHITECTURE) {
    'AMD64' { 'x86_64' }
    'ARM64' { 'aarch64' }
    default { throw "Unsupported architecture: $env:PROCESSOR_ARCHITECTURE" }
}
$target    = "$arch-pc-windows-msvc"
$assetName = "corvid-$target.zip"

# Resolve the release URL for three CORVID_VERSION shapes (mirrors
# install.sh's logic; see that script's matching block for the
# rationale). Slice `35V2-P33-install-script-nightly`.
#   - `latest`  → the most recent STABLE tag (GitHub's "latest"
#                 pointer, advanced by release.yml's stable runs).
#   - `nightly` → the most recent nightly-channel tag, resolved via
#                 the GitHub API.
#   - any other → treated as a literal release tag (e.g. `v0.0.1`,
#                 or `nightly-2026-06-04-d23d381` for a pinned
#                 nightly).
switch ($Version) {
    'latest' {
        $url = "https://github.com/$Repo/releases/latest/download/$assetName"
    }
    'nightly' {
        Step "Resolving most recent nightly release"
        [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
        try {
            $apiUrl  = "https://api.github.com/repos/$Repo/releases?per_page=30"
            $apiResp = Invoke-WebRequest -Uri $apiUrl -UseBasicParsing -ErrorAction Stop
            $apiObj  = $apiResp.Content | ConvertFrom-Json
        } catch {
            throw "Could not query GitHub releases API at $apiUrl (rate-limited or offline?). Try \$env:CORVID_VERSION='latest' for the most recent stable, or pin a specific nightly tag with e.g. \$env:CORVID_VERSION='nightly-2026-06-04-abc1234'. Underlying error: $($_.Exception.Message)"
        }
        $nightlyRel = $apiObj | Where-Object { $_.tag_name -like 'nightly-*' } | Select-Object -First 1
        if (-not $nightlyRel) {
            throw "No nightly release found at $apiUrl — release.yml may not have produced one yet."
        }
        Ok "Resolved nightly tag: $($nightlyRel.tag_name)"
        $url = "https://github.com/$Repo/releases/download/$($nightlyRel.tag_name)/$assetName"
    }
    default {
        $url = "https://github.com/$Repo/releases/download/$Version/$assetName"
    }
}

# --- prepare install root -------------------------------------------------
if (Test-Path $Root) {
    Step "Removing previous install at $Root"
    Remove-Item -Recurse -Force $Root
}
New-Item -ItemType Directory -Force -Path $Root | Out-Null

$tmp = Join-Path $env:TEMP ("corvid-install-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $tmp | Out-Null

$installed = $false

# --- fast path: prebuilt archive -----------------------------------------
try {
    Step "Downloading $assetName"
    $zip = Join-Path $tmp $assetName
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    Invoke-WebRequest -Uri $url -OutFile $zip -UseBasicParsing -ErrorAction Stop

    Step "Extracting to $Root"
    Expand-Archive -Path $zip -DestinationPath $tmp -Force

    $extracted = Join-Path $tmp "corvid-$target"
    $payload = if (Test-Path $extracted) { $extracted } else { $tmp }
    Get-ChildItem -Path $payload -Force | Where-Object { $_.FullName -ne $zip } | ForEach-Object {
        Move-Item -Path $_.FullName -Destination $Root -Force
    }

    $installed = $true
    Ok "Installed prebuilt binary"
} catch {
    Warn2 "Prebuilt archive unavailable: $($_.Exception.Message)"
    Warn2 "Falling back to cargo source build (this can take 5-15 minutes)..."
}

# --- fallback path: build from source ------------------------------------
if (-not $installed) {
    if (-not (Have 'cargo')) {
        Step "Installing Rust toolchain..."
        $cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
        if (Have 'winget') {
            winget install --id Rustlang.Rustup --silent --accept-source-agreements --accept-package-agreements
        } else {
            $rustupInit = Join-Path $tmp 'rustup-init.exe'
            Invoke-WebRequest -Uri 'https://win.rustup.rs/x86_64' -OutFile $rustupInit -UseBasicParsing
            & $rustupInit -y --default-toolchain stable --profile minimal
        }
        if (Test-Path $cargoBin) { $env:PATH = "$cargoBin;$env:PATH" }
        if (-not (Have 'cargo')) { throw "Rust install completed but cargo is not on PATH. Open a new shell and re-run." }
    }

    if (-not (Have 'git')) {
        throw "git is required for the source build fallback. Install Git for Windows from https://git-scm.com/download/win and re-run."
    }

    $src = Join-Path $Root 'src'
    Step "Cloning $Repo to $src"
    git clone --depth 1 "https://github.com/$Repo.git" $src

    Step "Building corvid (this is the slow part; first build downloads many crates)"
    Push-Location $src
    try {
        cargo install --path crates/corvid-cli --locked --root $Root
    } finally {
        Pop-Location
    }

    $stdSrc = Join-Path $src 'std'
    if (Test-Path $stdSrc) {
        Copy-Item -Recurse -Force $stdSrc (Join-Path $Root 'std')
    }
    Ok "Built from source"
}

# --- PATH wiring ----------------------------------------------------------
$bin = Join-Path $Root 'bin'
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
$onPath = ($userPath -split ';' | Where-Object { $_ -and ($_.TrimEnd('\') -ieq $bin.TrimEnd('\')) })
if (-not $onPath) {
    Step "Adding $bin to user PATH"
    $newPath = if ([string]::IsNullOrEmpty($userPath)) { $bin } else { "$userPath;$bin" }
    [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
} else {
    Ok "$bin already on user PATH"
}
$env:PATH = "$bin;$env:PATH"

[Environment]::SetEnvironmentVariable('CORVID_HOME', $Root, 'User')
$env:CORVID_HOME = $Root

# GitHub Actions: each step starts a fresh shell that does not
# inherit our updates to the user PATH or $env:PATH. Appending to
# $env:GITHUB_PATH / $env:GITHUB_ENV is how a step makes PATH and
# environment variables visible to the rest of the job.
if ($env:GITHUB_PATH -and (Test-Path $env:GITHUB_PATH)) {
    Add-Content -Path $env:GITHUB_PATH -Value $bin
    Ok "Appended $bin to `$env:GITHUB_PATH for subsequent steps"
}
if ($env:GITHUB_ENV -and (Test-Path $env:GITHUB_ENV)) {
    Add-Content -Path $env:GITHUB_ENV -Value "CORVID_HOME=$Root"
}

# --- verify ---------------------------------------------------------------
$exe = Join-Path $bin 'corvid.exe'
if (Test-Path $exe) {
    Step "corvid doctor"
    try { & $exe doctor } catch { Warn2 "doctor reported issues; see output above" }
} else {
    Warn2 "corvid.exe not found at $exe -- install may be incomplete"
}

Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue

Write-Host ""
Write-Host "Corvid installed at $Root" -ForegroundColor Green
Write-Host "Open a NEW terminal, then try:" -ForegroundColor Green
Write-Host "  corvid tour --list"
Write-Host "  corvid check examples\refund_bot_demo\refund.cor"
