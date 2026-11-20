# Corvid installer for Windows.
#
# One-line invocation:
#   irm https://raw.githubusercontent.com/Micrurus-Ai/Corvid-lang/main/install/install.ps1 | iex
#
# Env var overrides:
#   $env:CORVID_REPO     = "Micrurus-Ai/Corvid-lang"          # source repo
#   $env:CORVID_VERSION  = "latest" | "nightly" | "v0.0.1"    # release tag (slice 35V2-P33-install-script-nightly)
#   $env:CORVID_HOME     = "$env:USERPROFILE\.corvid"         # install root
#   $env:CORVID_SKIP_EDITOR_EXTENSION = "1"                    # opt out of automatic editor support

param([switch]$Uninstall, [switch]$FromSource)

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

$packageEntries = 'bin','lib','std','runtime-py','extensions','src','LICENSE','LICENSE-MIT','LICENSE-APACHE','README.md'
$receiptName = '.corvid-package-files'

function Remove-PackageEntry([string]$Path) {
    $attributes = [IO.File]::GetAttributes($Path)
    if ($attributes -band [IO.FileAttributes]::Directory) {
        # Remove a junction itself without traversing its target.
        if (-not ($attributes -band [IO.FileAttributes]::ReparsePoint)) {
            foreach ($child in [IO.Directory]::EnumerateFileSystemEntries($Path)) {
                Remove-PackageEntry $child
            }
        }
        [IO.Directory]::Delete($Path)
    } else {
        Remove-Item -LiteralPath $Path -Force
    }
}

if ($Uninstall) {
    if (-not (Test-Path -LiteralPath $Root)) { return }
    $Root = (Resolve-Path -LiteralPath $Root).ProviderPath
    $receipt = Join-Path $Root $receiptName
    if (-not (Test-Path -LiteralPath $receipt -PathType Leaf)) {
        throw "No installer receipt at $Root"
    }
    $lock = Join-Path $Root '.corvid-installing'
    New-Item -ItemType Directory -Path $lock -ErrorAction Stop | Out-Null
    try {
        $owned = @(Get-Content -LiteralPath $receipt)
        foreach ($entry in $packageEntries) {
            $target = Join-Path $Root $entry
            if (($owned -contains $entry) -and (Test-Path -LiteralPath $target)) {
                Remove-PackageEntry $target
            }
        }
        $profileState = Join-Path $Root '.corvid-user-environment.json'
        if (Test-Path -LiteralPath $profileState) {
            $changes = Get-Content -LiteralPath $profileState -Raw | ConvertFrom-Json
            if ($changes.addedPath) {
                $bin = Join-Path $Root 'bin'
                $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
                $paths = @($userPath -split ';' | Where-Object { $_.TrimEnd('\') -ine $bin.TrimEnd('\') })
                [Environment]::SetEnvironmentVariable('Path', ($paths -join ';'), 'User')
            }
            if ([Environment]::GetEnvironmentVariable('CORVID_HOME', 'User') -ieq $Root) {
                [Environment]::SetEnvironmentVariable('CORVID_HOME', $changes.previousHome, 'User')
            }
            Remove-Item -LiteralPath $profileState
        }
        Remove-Item -LiteralPath $receipt
    } finally {
        [IO.Directory]::Delete($lock)
    }
    if (-not (Get-ChildItem -LiteralPath $Root -Force)) { [IO.Directory]::Delete($Root) }
    Ok "Uninstalled Corvid from $Root"
    return
}

function Install-EditorSupport {
    if ($env:CORVID_SKIP_EDITOR_EXTENSION -eq '1') {
        Warn2 "Skipping editor support (CORVID_SKIP_EDITOR_EXTENSION=1)"
        return
    }

    $vsix = Join-Path $Root 'extensions\corvid.vsix'
    if (-not (Test-Path $vsix)) {
        Warn2 "Bundled Corvid editor extension not found; syntax highlighting was not installed"
        return
    }

    $editors = @(
        @{ Name = 'VS Code';  Command = 'code' },
        @{ Name = 'Cursor';   Command = 'cursor' },
        @{ Name = 'VSCodium'; Command = 'codium' },
        @{ Name = 'Windsurf'; Command = 'windsurf' }
    )
    $found = $false

    foreach ($editor in $editors) {
        if (-not (Have $editor.Command)) { continue }
        $found = $true
        Step "Installing Corvid syntax highlighting for $($editor.Name)"
        & $editor.Command --install-extension $vsix --force
        if ($LASTEXITCODE -eq 0) {
            Ok "$($editor.Name) will highlight .cor files automatically"
        } else {
            Warn2 "$($editor.Name) extension installation failed (exit $LASTEXITCODE)"
        }
    }

    if (-not $found) {
        Warn2 "No supported editor CLI found; the bundled extension remains at $vsix"
    }
}

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
#   - `latest`  -> the most recent STABLE tag (GitHub's "latest"
#                 pointer, advanced by release.yml's stable runs).
#   - `nightly` -> the most recent nightly-channel tag, resolved via
#                 the GitHub API.
#   - any other -> treated as a literal release tag (e.g. `v0.0.1`,
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
            throw "Could not query GitHub releases API at $apiUrl (rate-limited or offline?). Try `$env:CORVID_VERSION='latest' for the most recent stable, or pin a specific nightly tag with e.g. `$env:CORVID_VERSION='nightly-2026-06-04-abc1234'. Underlying error: $($_.Exception.Message)"
        }
        $nightlyRel = $apiObj | Where-Object { $_.tag_name -like 'nightly-*' } | Select-Object -First 1
        if (-not $nightlyRel) {
            throw "No nightly release found at $apiUrl  - release.yml may not have produced one yet."
        }
        Ok "Resolved nightly tag: $($nightlyRel.tag_name)"
        $url = "https://github.com/$Repo/releases/download/$($nightlyRel.tag_name)/$assetName"
    }
    default {
        $url = "https://github.com/$Repo/releases/download/$Version/$assetName"
    }
}

# A workspace inside the target keeps renames on one volume. Exclusive
# creation prevents two installers from replacing the same files.
New-Item -ItemType Directory -Force -Path $Root | Out-Null
$Root = (Resolve-Path -LiteralPath $Root).ProviderPath
$tmp = Join-Path $Root '.corvid-installing'
New-Item -ItemType Directory -Path $tmp -ErrorAction Stop | Out-Null
$stage = Join-Path $tmp 'new'
$previous = Join-Path $tmp 'previous'
$archive = Join-Path $tmp 'archive'
$keepWorkspace = $false
try {
    New-Item -ItemType Directory -Path $stage, $previous, $archive | Out-Null
    $installed = $false

    # --- fast path: prebuilt archive -----------------------------------------
    if (-not $FromSource) {
        try {
            Step "Downloading $assetName"
            $zip = Join-Path $tmp $assetName
            [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
            Invoke-WebRequest -Uri $url -OutFile $zip -UseBasicParsing -ErrorAction Stop
            $installed = $true
        } catch {
            Warn2 "Prebuilt archive unavailable: $($_.Exception.Message)"
            Warn2 "Falling back to cargo source build (this can take 5-15 minutes)..."
        }
    }
    if ($installed) {
        Step "Extracting to $Root"
        Expand-Archive -LiteralPath $zip -DestinationPath $archive -Force

        $extracted = Join-Path $archive "corvid-$target"
        $payload = if (Test-Path $extracted) { $extracted } else { $archive }
        Get-ChildItem -LiteralPath $payload -Force | ForEach-Object {
            Move-Item -LiteralPath $_.FullName -Destination $stage
        }

        Ok "Installed prebuilt binary"
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
                Invoke-WebRequest -Uri "https://win.rustup.rs/$arch" -OutFile $rustupInit -UseBasicParsing
                & $rustupInit -y --default-toolchain stable --profile minimal
            }
            if (Test-Path $cargoBin) { $env:PATH = "$cargoBin;$env:PATH" }
            if (-not (Have 'cargo')) { throw "Rust install completed but cargo is not on PATH. Open a new shell and re-run." }
        }

        if (-not (Have 'git')) {
            throw "git is required for the source build fallback. Install Git for Windows from https://git-scm.com/download/win and re-run."
        }

        $src = Join-Path $stage 'src'
        Step "Cloning $Repo to $src"
        $tag = switch ($Version) {
            'latest' {
                $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest"
                if (-not $release.tag_name) { throw "Latest stable release is missing its tag" }
                $release.tag_name
            }
            'nightly' { $nightlyRel.tag_name }
            default { $Version }
        }
        & git clone --depth 1 --branch $tag "https://github.com/$Repo.git" $src
        if ($LASTEXITCODE -ne 0) { throw "Cannot clone requested Corvid version $Version" }

        Step "Building Corvid, the language server and the native runtime"
        Push-Location $src
        try {
            $buildDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $tmp 'build' }
            $artifacts = Join-Path $buildDir "$target/release"
            $toolsDir = New-Item -ItemType Directory -Path (Join-Path $stage 'bin')
            $libDir = New-Item -ItemType Directory -Path (Join-Path $stage 'lib')
            cargo build --locked --release --target $target --target-dir $buildDir -p corvid-runtime
            if ($LASTEXITCODE -ne 0) { throw "Corvid runtime build failed" }
            Copy-Item -LiteralPath (Join-Path $artifacts 'corvid_runtime.lib') -Destination $libDir.FullName
            cargo build --locked --release --target $target --target-dir $buildDir -p corvid-cli -p corvid-lsp
            if ($LASTEXITCODE -ne 0) { throw "Corvid source build failed" }
            foreach ($exe in 'corvid.exe', 'corvid-lsp.exe') {
                Copy-Item -LiteralPath (Join-Path $artifacts $exe) -Destination $toolsDir.FullName
            }
        } finally {
            Pop-Location
        }
        if (Test-Path -LiteralPath (Join-Path $src 'runtime/python')) {
            Copy-Item -Recurse -LiteralPath (Join-Path $src 'runtime/python') -Destination (Join-Path $stage 'runtime-py')
        }
        foreach ($notice in 'LICENSE','LICENSE-MIT', 'LICENSE-APACHE') {
            if (Test-Path -LiteralPath (Join-Path $src $notice)) {
                Copy-Item -LiteralPath (Join-Path $src $notice) -Destination $stage
            }
        }

        $stdSrc = Join-Path $src 'std'
        if (Test-Path $stdSrc) {
            Copy-Item -Recurse -Force $stdSrc (Join-Path $stage 'std')
        }

        # Source-fallback installs do not have the release archive's bundled
        # VSIX. Package the exact checked-out extension when Node is available
        # so editor highlighting remains automatic on this path too.
        $extensionSrc = Join-Path $src 'extensions\vscode-corvid'
        if ($env:CORVID_SKIP_EDITOR_EXTENSION -eq '1') {
            Ok "Skipped packaging the editor extension"
        } elseif ((Test-Path $extensionSrc) -and (Have 'npm') -and (Have 'npx')) {
            Step "Packaging Corvid editor support"
            $extensionOut = Join-Path $stage 'extensions'
            New-Item -ItemType Directory -Force -Path $extensionOut | Out-Null
            npm ci --prefix $extensionSrc
            if ($LASTEXITCODE -eq 0) {
                Push-Location $extensionSrc
                try {
                    npx --yes '@vscode/vsce@3.6.2' package --out (Join-Path $extensionOut 'corvid.vsix')
                } finally {
                    Pop-Location
                }
            }
            if ($LASTEXITCODE -ne 0) {
                Warn2 "Could not package editor support from source"
            }
        } else {
            Warn2 "Node/npm not found; source fallback could not package editor support"
        }
        Ok "Built from source"
    }

    $candidate = Join-Path $stage 'bin/corvid.exe'
    try {
        & $candidate --version
    } catch {
        $launchError = $_.Exception.Message
        $digest = (Get-FileHash -LiteralPath $candidate -Algorithm SHA256).Hash
        throw "Windows could not start the staged Corvid CLI. SHA-256: $digest. $launchError"
    }
    if ($LASTEXITCODE -eq -1073741515) {
        throw "Windows could not load a required DLL. Check that the matching CPython installation directory is on PATH and uses the same architecture as Corvid, then rerun the installer."
    }
    if ($LASTEXITCODE -ne 0) { throw "Staged corvid failed its version check" }

    if (-not (Test-Path -LiteralPath (Join-Path $Root 'bin/corvid.exe') -PathType Leaf)) {
        foreach ($entry in $packageEntries) {
            if (Test-Path -LiteralPath (Join-Path $Root $entry)) {
                throw "Existing files at $Root overlap the install. Choose a dedicated CORVID_HOME."
            }
        }
    }
    $activated = @()
    $receipt = Join-Path $Root $receiptName
    $hasReceipt = Test-Path -LiteralPath $receipt -PathType Leaf
    if ($hasReceipt) { $previousEntries = @(Get-Content -LiteralPath $receipt) }
    try {
        foreach ($entry in $packageEntries) {
            $current = Join-Path $Root $entry
            $replacement = Join-Path $stage $entry
            if (Test-Path -LiteralPath $current) {
                if ($hasReceipt -and ($previousEntries -notcontains $entry)) {
                    if (Test-Path -LiteralPath $replacement) {
                        throw "Package would overwrite an unmanaged path at $current"
                    }
                    continue
                }
                Move-Item -LiteralPath $current -Destination $previous
            }
            if (Test-Path -LiteralPath $replacement) {
                Move-Item -LiteralPath $replacement -Destination $Root
                $activated += $entry
            }
        }
        $candidateReceipt = Join-Path $tmp 'receipt'
        Set-Content -LiteralPath $candidateReceipt -Value $activated -Encoding ASCII
        if ([IO.File]::Exists($receipt)) {
            [IO.File]::Replace($candidateReceipt, $receipt, (Join-Path $tmp 'old-receipt'))
        } else {
            [IO.File]::Move($candidateReceipt, $receipt)
        }
    } catch {
        # Keep the previous files available if restoration itself encounters an error.
        $keepWorkspace = $true
        foreach ($entry in $activated) {
            Move-Item -LiteralPath (Join-Path $Root $entry) -Destination $stage
        }
        Get-ChildItem -LiteralPath $previous -Force | ForEach-Object {
            Move-Item -LiteralPath $_.FullName -Destination $Root
        }
        $keepWorkspace = $false
        throw
    }
} finally {
    if (-not $keepWorkspace) {
        Remove-PackageEntry $tmp
    }
}

# --- PATH wiring ----------------------------------------------------------
$bin = Join-Path $Root 'bin'
if ($env:CORVID_NO_MODIFY_PROFILE -ne '1') {
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $onPath = ($userPath -split ';' | Where-Object { $_ -and ($_.TrimEnd('\') -ieq $bin.TrimEnd('\')) })
    $profileState = Join-Path $Root '.corvid-user-environment.json'
    if (-not (Test-Path -LiteralPath $profileState)) {
        @{
            addedPath = -not $onPath
            previousHome = [Environment]::GetEnvironmentVariable('CORVID_HOME', 'User')
        } | ConvertTo-Json | Set-Content -LiteralPath $profileState -Encoding UTF8
    }
    if (-not $onPath) {
        Step "Adding $bin to user PATH"
        $newPath = if ([string]::IsNullOrEmpty($userPath)) { $bin } else { "$userPath;$bin" }
        [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
    } else {
        Ok "$bin already on user PATH"
    }
    [Environment]::SetEnvironmentVariable('CORVID_HOME', $Root, 'User')
}
$env:PATH = "$bin;$env:PATH"

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

# --- editor support -------------------------------------------------------
# Official release archives carry a version-matched VSIX. Installing it
# here makes opening any `.cor` file colorful immediately; no separate
# Marketplace search or manual extension step is required.
Install-EditorSupport

# --- verify ---------------------------------------------------------------
$exe = Join-Path $bin 'corvid.exe'
if (Test-Path $exe) {
    Step "corvid doctor"
    try { & $exe doctor } catch { Warn2 "doctor reported issues; see output above" }
} else {
    Warn2 "corvid.exe not found at $exe -- install may be incomplete"
}


Write-Host ""
Write-Host "Corvid installed at $Root" -ForegroundColor Green
if ($env:CORVID_NO_MODIFY_PROFILE -eq '1') {
    $quotedRoot = $Root.Replace("'", "''")
    Write-Host "To use this installation in a PowerShell session, run:" -ForegroundColor Green
    Write-Host "  `$env:CORVID_HOME = '$quotedRoot'"
    Write-Host '  $env:Path = (Join-Path $env:CORVID_HOME bin) + ";" + $env:Path'
    Write-Host "Then try:" -ForegroundColor Green
} else {
    Write-Host "Open a new terminal, then try:" -ForegroundColor Green
}
Write-Host "  corvid new hello-corvid"
Write-Host "  cd hello-corvid"
Write-Host "  corvid check src/main.cor"
Write-Host "  corvid run src/main.cor"
