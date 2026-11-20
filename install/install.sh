#!/usr/bin/env sh
# Corvid installer for macOS and Linux.
#
# One-line invocation:
#   curl -fsSL https://raw.githubusercontent.com/Micrurus-Ai/Corvid-lang/main/install/install.sh | sh
#
# Env var overrides:
#   CORVID_REPO     (default: Micrurus-Ai/Corvid-lang)
#   CORVID_VERSION  (default: latest; also accepts `nightly` for the
#                    most recent main-branch push, or any literal
#                    release tag like `v0.0.1` /
#                    `nightly-2026-06-04-d23d381`)
#   CORVID_HOME     (default: $HOME/.corvid)
#   CORVID_SKIP_EDITOR_EXTENSION=1 (opt out of automatic editor support)
#
# Canonical source: https://github.com/Micrurus-Ai/Corvid-lang/blob/main/install/install.sh
# This file is also mirrored to Micrurus-Ai/corvid-installer for the
# `iwr corvid-lang.org/install | iex` shortcut; edits MUST land here first.

set -eu

REPO="${CORVID_REPO:-Micrurus-Ai/Corvid-lang}"
VERSION="${CORVID_VERSION:-latest}"
ROOT="${CORVID_HOME:-$HOME/.corvid}"

# --- helpers --------------------------------------------------------------
step()  { printf '\033[36m==> %s\033[0m\n' "$*"; }
ok()    { printf '\033[32m    %s\033[0m\n' "$*"; }
warn()  { printf '\033[33m    %s\033[0m\n' "$*"; }
die()   { printf '\033[31merror: %s\033[0m\n' "$*" >&2; exit 1; }
have()  { command -v "$1" >/dev/null 2>&1; }

package_entries="bin lib std runtime-py extensions src LICENSE LICENSE-MIT LICENSE-APACHE README.md"
receipt=.corvid-package-files

uninstall() {
    [ -d "$ROOT" ] || return 0
    ROOT="$(cd "$ROOT" && pwd -P)"
    [ -f "$ROOT/$receipt" ] || die "no installer receipt at $ROOT"
    lock="$ROOT/.corvid-installing"
    mkdir "$lock" || die "another installer is active at $ROOT"
    trap 'rm -f "$lock/profile"; rmdir "$lock"' EXIT
    trap 'exit 1' INT TERM
    # Edit only the exact three lines added for this installation.
    for rc in "$HOME/.profile" "$HOME/.bashrc" "$HOME/.zshrc"; do
        [ -f "$rc" ] || continue
        [ -f "$ROOT/.corvid-profile-${rc##*/}" ] || continue
        CORVID_REMOVE_LINE="$(cat "$ROOT/.corvid-profile-${rc##*/}")" awk '
            { lines[NR]=$0 }
            END {
                for (i=1; i<=NR; i++) {
                    if (lines[i]=="# corvid" &&
                        lines[i+1]==ENVIRON["CORVID_REMOVE_LINE"] &&
                        lines[i+2]=="export PATH=\"$CORVID_HOME/bin:$PATH\"") { i+=2; continue }
                    print lines[i]
                }
            }' "$rc" > "$lock/profile"
        if ! cmp -s "$rc" "$lock/profile"; then cat "$lock/profile" > "$rc"; fi
        rm -f "$lock/profile" "$ROOT/.corvid-profile-${rc##*/}"
    done
    for entry in $package_entries; do
        if grep -Fxq "$entry" "$ROOT/$receipt"; then rm -rf "${ROOT:?}/$entry"; fi
    done
    rm -f "$ROOT/$receipt"
    rmdir "$lock"
    trap - EXIT INT TERM
    rmdir "$ROOT" 2>/dev/null || :
    ok "Uninstalled Corvid from $ROOT"
}

case "${1-}" in
    --uninstall) uninstall; exit 0 ;;
    --from-source) from_source=1 ;;
    '') from_source=0 ;;
    *) die "unknown installer argument: $1" ;;
esac

install_editor_support() {
    if [ "${CORVID_SKIP_EDITOR_EXTENSION:-0}" = "1" ]; then
        warn "Skipping editor support (CORVID_SKIP_EDITOR_EXTENSION=1)"
        return
    fi

    vsix="$ROOT/extensions/corvid.vsix"
    if [ ! -f "$vsix" ]; then
        warn "Bundled Corvid editor extension not found; syntax highlighting was not installed"
        return
    fi

    found=0
    for editor in code cursor codium windsurf; do
        if ! have "$editor"; then
            continue
        fi
        found=1
        step "Installing Corvid syntax highlighting with $editor"
        if "$editor" --install-extension "$vsix" --force; then
            ok "$editor will highlight .cor files automatically"
        else
            warn "$editor extension installation failed; run: $editor --install-extension \"$vsix\" --force"
        fi
    done

    if [ "$found" -eq 0 ]; then
        warn "No supported editor CLI found; the bundled extension remains at $vsix"
    fi
}

dl() {
    if   have curl; then curl -fsSL --proto '=https' --tlsv1.2 -o "$2" "$1"
    elif have wget; then wget -qO "$2" "$1"
    else die "need curl or wget on PATH"
    fi
}

# --- target detection -----------------------------------------------------
os_kernel="$(uname -s | tr '[:upper:]' '[:lower:]')"
case "$os_kernel" in
    linux*)  os_id='unknown-linux-gnu' ;;
    darwin*) os_id='apple-darwin' ;;
    *)       die "unsupported OS: $os_kernel" ;;
esac

case "$(uname -m)" in
    x86_64|amd64)  arch_id='x86_64' ;;
    aarch64|arm64) arch_id='aarch64' ;;
    *)             die "unsupported architecture: $(uname -m)" ;;
esac

target="${arch_id}-${os_id}"
asset="corvid-${target}.tar.gz"

# Resolve the release URL for three CORVID_VERSION shapes:
#   - `latest`  → the most recent STABLE tag (the existing fast
#                 path). Maps to GitHub's "latest" pointer which
#                 release.yml's stable runs advance.
#   - `nightly` → the most recent nightly-channel tag (slice
#                 `35V2-P33-install-script-nightly`). Queries
#                 the GitHub API for the newest tag matching
#                 `nightly-*` and downloads from it. Nightly
#                 releases are marked as pre-releases by
#                 release.yml so they don't pollute the
#                 "latest" pointer.
#   - any other → treated as a literal release tag (e.g.
#                 `v0.0.1` for a pinned stable, or
#                 `nightly-2026-06-04-d23d381` for a pinned
#                 nightly). Maps to
#                 `releases/download/<tag>/...`.
case "$VERSION" in
    latest)
        url="https://github.com/${REPO}/releases/latest/download/${asset}"
        ;;
    nightly)
        step "Resolving most recent nightly release"
        api="https://api.github.com/repos/${REPO}/releases?per_page=30"
        # Pull the first `tag_name` whose value matches
        # `nightly-*` from the API response. `per_page=30` gives
        # us several days of headroom — even if every page-1
        # entry is somehow stable, the most recent nightly is
        # almost certainly within reach.
        if have curl; then
            api_body="$(curl -fsSL --proto '=https' --tlsv1.2 "$api" 2>/dev/null || true)"
        elif have wget; then
            api_body="$(wget -qO- "$api" 2>/dev/null || true)"
        else
            api_body=""
        fi
        # Parse `tag_name` lines without depending on `jq`
        # (which the installer doesn't require its user to
        # have on PATH). Match the JSON quoting GitHub uses.
        nightly_tag="$(printf '%s\n' "$api_body" \
            | grep -E '"tag_name"[[:space:]]*:[[:space:]]*"nightly-[^"]+"' \
            | head -n 1 \
            | sed -E 's/.*"tag_name"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/')"
        if [ -z "$nightly_tag" ]; then
            die "no nightly release found at ${api} — release.yml may not have produced one yet, or the GitHub API rate-limited the unauthenticated request. Try `CORVID_VERSION=latest` for the most recent stable, or pin a specific nightly tag with e.g. \`CORVID_VERSION=nightly-2026-06-04-abc1234\`."
        fi
        ok "Resolved nightly tag: $nightly_tag"
        url="https://github.com/${REPO}/releases/download/${nightly_tag}/${asset}"
        ;;
    *)
        url="https://github.com/${REPO}/releases/download/${VERSION}/${asset}"
        ;;
esac

# Stage on the destination filesystem. Exclusive creation also serializes installers.
mkdir -p "$ROOT"
ROOT="$(cd "$ROOT" && pwd -P)"
tmp="$ROOT/.corvid-installing"
mkdir "$tmp" || die "installation already in progress or interrupted at $tmp"
stage="$tmp/new"
mkdir "$stage" "$tmp/previous" "$tmp/activated" "$tmp/archive"

cleanup() {
    result=$?
    trap - EXIT INT TERM
    if [ -f "$tmp/switching" ]; then
        for marker in "$tmp/activated"/*; do
            [ -d "$marker" ] || continue
            entry=${marker##*/}
            if [ -e "$ROOT/$entry" ]; then
                mv "$ROOT/$entry" "$stage/" || exit 1
            fi
        done
        for old in "$tmp/previous"/*; do
            [ -e "$old" ] || [ -L "$old" ] || continue
            mv "$old" "$ROOT/" || exit 1
        done
    fi
    rm -rf "$tmp"
    exit "$result"
}
trap cleanup EXIT
trap 'exit 1' INT TERM

installed=0

# --- fast path: prebuilt archive -----------------------------------------
if [ "$from_source" -eq 1 ]; then
    step "Building from the selected source release"
elif dl "$url" "$tmp/$asset" 2>"$tmp/dl.log"; then
    step "Extracting to $ROOT"
    tar -xzf "$tmp/$asset" -C "$tmp/archive"
    payload="$tmp/archive/corvid-${target}"
    [ -d "$payload" ] || payload="$tmp/archive"
    for entry in "$payload"/*; do
        [ -e "$entry" ] || continue
        mv "$entry" "$stage/"
    done
    installed=1
    ok "Staged prebuilt binary"
else
    warn "Prebuilt archive unavailable for $target"
    warn "Falling back to cargo source build (this can take 5-15 minutes)..."
fi

# --- fallback path: build from source ------------------------------------
if [ "$installed" -eq 0 ]; then
    if ! have cargo; then
        step "Installing Rust toolchain via rustup..."
        dl 'https://sh.rustup.rs' "$tmp/rustup-init.sh"
        sh "$tmp/rustup-init.sh" -y --default-toolchain stable --profile minimal
        # shellcheck disable=SC1091
        . "$HOME/.cargo/env"
    fi
    have git || die "git is required for the source build fallback. Install git and re-run."

    src="$stage/src"
    step "Cloning $REPO to $src"
    case "$VERSION" in
        latest)
            dl "https://api.github.com/repos/${REPO}/releases/latest" "$tmp/release.json" ||
                die "cannot resolve the latest stable release; set CORVID_VERSION to a release tag"
            source_tag="$(sed -nE 's/.*"tag_name"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/p' "$tmp/release.json" | head -n 1)"
            [ -n "$source_tag" ] || die "latest stable release is missing its tag"
            ;;
        nightly) source_tag="$nightly_tag" ;;
        *) source_tag="$VERSION" ;;
    esac
    git clone --depth 1 --branch "$source_tag" "https://github.com/${REPO}.git" "$src"

    step "Building Corvid, the language server and the native runtime"
    (
        cd "$src"
        build_dir="${CARGO_TARGET_DIR:-$tmp/build}"
        artifacts="$build_dir/$target/release"
        mkdir -p "$stage/bin" "$stage/lib"
        cargo build --locked --release --target "$target" --target-dir "$build_dir" -p corvid-runtime
        cp "$artifacts/libcorvid_runtime.a" "$stage/lib/"
        cargo build --locked --release --target "$target" --target-dir "$build_dir" -p corvid-cli -p corvid-lsp
        cp "$artifacts/corvid" "$artifacts/corvid-lsp" "$stage/bin/"
    )
    if [ -d "$src/runtime/python" ]; then
        cp -R "$src/runtime/python" "$stage/runtime-py"
    fi
    for notice in LICENSE LICENSE-MIT LICENSE-APACHE; do
        if [ -f "$src/$notice" ]; then cp "$src/$notice" "$stage/"; fi
    done

    if [ -d "$src/std" ]; then
        rm -rf "$stage/std"
        cp -R "$src/std" "$stage/std"
    fi

    # Source-fallback installs do not have the release archive's bundled
    # VSIX. Package the exact checked-out extension when Node is available
    # so editor highlighting remains automatic on this path too.
    extension_src="$src/extensions/vscode-corvid"
    if [ "${CORVID_SKIP_EDITOR_EXTENSION:-0}" = 1 ]; then
        ok "Skipped packaging the editor extension"
    elif [ -d "$extension_src" ] && have npm && have npx; then
        step "Packaging Corvid editor support"
        mkdir -p "$stage/extensions"
        if npm ci --prefix "$extension_src" \
            && (cd "$extension_src" \
                && npx --yes @vscode/vsce@3.6.2 package \
                    --out "$stage/extensions/corvid.vsix"); then
            ok "Packaged Corvid editor extension"
        else
            warn "Could not package editor support from source"
        fi
    else
        warn "Node/npm not found; source fallback could not package editor support"
    fi
    ok "Built from source"
fi

# Switch only package-owned paths. Other project and user data stays in place.
if "$stage/bin/corvid" --version 2>"$tmp/version.err"; then
    cat "$tmp/version.err" >&2
else
    cat "$tmp/version.err" >&2
    if [ "$os_kernel" = linux ] && grep -Fq 'libpython3.11.so.1.0: cannot open shared object file' "$tmp/version.err"; then
        die "Corvid requires the Python 3.11 shared runtime. On Debian 12, run 'apt-get install libpython3.11' as root, then rerun the installer."
    fi
    die "staged corvid failed its version check"
fi
if [ ! -f "$ROOT/bin/corvid" ]; then
    for entry in $package_entries; do
        if [ -e "$ROOT/$entry" ] || [ -L "$ROOT/$entry" ]; then
            die "existing files at $ROOT overlap the install. Choose a dedicated CORVID_HOME."
        fi
    done
fi
touch "$tmp/switching"
for entry in $package_entries; do
    if [ -e "$ROOT/$entry" ] || [ -L "$ROOT/$entry" ]; then
        if [ -f "$ROOT/$receipt" ] && ! grep -Fxq "$entry" "$ROOT/$receipt"; then
            [ ! -e "$stage/$entry" ] || die "package would overwrite an unmanaged path: $ROOT/$entry"
            continue
        fi
        mv "$ROOT/$entry" "$tmp/previous/"
    fi
    if [ -e "$stage/$entry" ]; then
        mkdir "$tmp/activated/$entry"
        mv "$stage/$entry" "$ROOT/"
    fi
done
for entry in $package_entries; do
    if [ -d "$tmp/activated/$entry" ]; then printf '%s\n' "$entry"; fi
done > "$tmp/receipt"
mv "$tmp/receipt" "$ROOT/$receipt"
rm "$tmp/switching"

# --- PATH wiring ----------------------------------------------------------
bin="$ROOT/bin"
quoted_root="$(printf '%s' "$ROOT" | sed "s/'/'\\\\''/g")"
home_line="export CORVID_HOME='$quoted_root'"
path_ready=0
case "${CORVID_NO_MODIFY_PROFILE:-0}:${PATH-}:" in
    *":$bin:"*) path_ready=1; ok "$bin already on PATH" ;;
    1:*) : ;;
    *)
        step "Adding $bin to PATH in shell rc files"
        for rc in "$HOME/.profile" "$HOME/.bashrc" "$HOME/.zshrc"; do
            [ -f "$rc" ] || continue
            if [ -f "$ROOT/.corvid-profile-${rc##*/}" ] &&
                grep -Fxq "$home_line" "$rc" &&
                grep -Fxq 'export PATH="$CORVID_HOME/bin:$PATH"' "$rc"; then
                path_ready=1
            elif ! grep -v '^[[:space:]]*#' "$rc" | grep -q 'CORVID_HOME'; then
                {
                    if [ -s "$rc" ] && [ -n "$(tail -c 1 "$rc")" ]; then printf '\n'; fi
                    printf '# corvid\n'
                    printf '%s\n' "$home_line"
                    printf 'export PATH="$CORVID_HOME/bin:$PATH"\n'
                } >> "$rc"
                printf '%s\n' "$home_line" > "$ROOT/.corvid-profile-${rc##*/}"
                ok "  updated $rc"
                path_ready=1
            fi
        done
        ;;
esac

# GitHub Actions: each step starts a fresh non-interactive shell that
# does not source ~/.profile / ~/.bashrc / ~/.zshrc, so writing to
# those files (above) does not propagate PATH to the next step.
# Appending to $GITHUB_PATH / $GITHUB_ENV is how a step makes PATH
# and env vars visible to the rest of the job.
if [ -n "${GITHUB_PATH-}" ] && [ -w "${GITHUB_PATH}" ]; then
    printf '%s\n' "$bin" >> "$GITHUB_PATH"
    ok "Appended $bin to \$GITHUB_PATH for subsequent steps"
fi
if [ -n "${GITHUB_ENV-}" ] && [ -w "${GITHUB_ENV}" ]; then
    printf 'CORVID_HOME=%s\n' "$ROOT" >> "$GITHUB_ENV"
fi

export CORVID_HOME="$ROOT"
export PATH="$bin:$PATH"

# --- editor support -------------------------------------------------------
# Official release archives carry a version-matched VSIX. Installing it
# here makes opening any `.cor` file colorful immediately; no separate
# Marketplace search or manual extension step is required.
install_editor_support

# --- verify ---------------------------------------------------------------
if [ -x "$bin/corvid" ]; then
    step "corvid doctor"
    "$bin/corvid" doctor || warn "doctor reported issues; see output above"
else
    warn "corvid not found at $bin/corvid -- install may be incomplete"
fi

printf '\n\033[32mCorvid installed at %s\033[0m\n' "$ROOT"
if [ "$path_ready" -eq 1 ]; then
    printf 'Open a shell with the updated PATH, then try:\n'
else
    printf 'To use Corvid in your current shell, run:\n'
    printf '  %s\n' "$home_line" 'export PATH="$CORVID_HOME/bin:$PATH"'
    printf 'Then try:\n'
fi
printf '  corvid new hello-corvid\n'
printf '  cd hello-corvid\n'
printf '  corvid check\n'
printf '  corvid run\n'
