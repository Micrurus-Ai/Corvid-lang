# Install

## Supported platforms

The release workflow targets Linux, macOS and Windows. Available architectures
and runtime requirements vary by release. The published v0.1.0 Linux x64
archive runs on Ubuntu 24.04 and requires glibc 2.39. For the current
Bookworm build, the CLI's embedded Python integration requires the matching
Python shared library even for pure Corvid programs. On Debian 12, run
`apt-get install libpython3.11` as root before installing the candidate.
The installer checks that the new CLI starts before replacing an existing installation.
The Debian 12 candidate archives passed on native x64 and ARM64 runners.

On macOS and Linux, if the installer leaves your shell profile untouched and Corvid is outside your current `PATH`, it prints two `export` commands. Run them in your current shell before trying `corvid new`. The printed commands include your chosen install directory.

On Windows, setting `CORVID_NO_MODIFY_PROFILE=1` also skips persistent user environment changes. To select that installation in a fresh PowerShell session, run the two `$env:` assignments printed by the installer.

The Windows x64 and ARM64 candidates passed with matching-architecture CPython 3.14. Put the Python installation directory containing `python3.dll` on `PATH` before starting Corvid. The hosted Windows x64 candidate still requires administrator review on the machine where Defender blocked startup.

If Windows denies permission to start the staged CLI, the installer reports the executable's SHA-256 before replacing the current installation. Use that digest to identify the exact file when reviewing the event with your system administrator.

The macOS candidate uses the Python framework at `/Library/Frameworks/Python.framework/Versions/<version>/Python`. Both `corvid` and `corvid-lsp` load it at startup. Use a framework installation of CPython 3.14 on Intel Macs or CPython 3.15 on Apple Silicon. A virtual environment alone does not supply that system path.

Run `corvid doctor` to check the installation and configuration. With a prebuilt CLI, missing Rust tools are informational. Source builds and native server wrappers use Cargo and rustc. Invalid configuration values cause the command to fail.

## Install scripts

For manual downloads from a release that includes `SHA256SUMS.txt`, compare your archive's SHA-256 with the line for its exact filename. On Linux use `sha256sum ARCHIVE`, on macOS use `shasum -a 256 ARCHIVE`, and in PowerShell use `Get-FileHash -Algorithm SHA256 -LiteralPath ARCHIVE`. Replace `ARCHIVE` with the downloaded filename.

Unix (macOS + Linux):

```sh
curl -fsSL https://raw.githubusercontent.com/Micrurus-Ai/Corvid-lang/main/install/install.sh | sh
```

The official archive includes the matching Corvid editor extension and
language server. The installer detects VS Code, Cursor, VSCodium, and
Windsurf and installs the extension automatically, so `.cor` files are
highlighted immediately. Set `CORVID_SKIP_EDITOR_EXTENSION=1` to opt out.

Windows (PowerShell):

```powershell
iwr -useb https://raw.githubusercontent.com/Micrurus-Ai/Corvid-lang/main/install/install.ps1 | iex
```

The installer downloads the platform archive over HTTPS and adds the
installation's `bin` directory to `PATH`. The default root is `~/.corvid`.
If the archive is unavailable, the installer builds the CLI, language server
and native runtime from the selected release tag. It places the tools in
`bin`, the native library in `lib`, the standard library in `std` and the
Python runtime in `runtime-py`. The default `latest` selection resolves to
the latest stable release tag for source builds too.

To build for the host deliberately, set `CORVID_VERSION` to the tag you need
and run `sh install/install.sh --from-source` or
`.\install\install.ps1 -FromSource` from this checkout. This is useful on Linux
when the published binary needs a newer glibc. A source build uses the Rust
and native build tools required by that tag.
On Windows, the installer's direct Rust download selects x64 or ARM64 to
match the target selected by PowerShell.

## Upgrade or roll back

The installer lifecycle and tagged source fallback passed with fixture packages on native x64 and ARM64 runners for Linux, macOS and Windows. See the [native installer run](https://github.com/Micrurus-Ai/Corvid-lang/actions/runs/44812107983).

Set `CORVID_VERSION` to the release tag you want and rerun the same installer.
Use an earlier tag to roll back. Both the download and the source fallback
for a pinned version use the requested tag. Run `corvid --version` afterward.

The installer stages a candidate under `CORVID_HOME` and runs its version
command before replacing package files. A failed download and source build
leave the existing installation usable. Keep your own files outside the
package directories such as `bin`, `lib`, `src` and `std`.

For an isolated installation, set `CORVID_HOME` to a dedicated directory,
`CORVID_NO_MODIFY_PROFILE=1` to skip persistent shell or Windows user
environment changes, and `CORVID_SKIP_EDITOR_EXTENSION=1` to skip editor
integration. Run the executable directly from that directory's `bin` folder.

## From a source checkout

From the repository root, build the CLI with the workspace lockfile and
run the resulting binary through Cargo.

```sh
cargo build --locked --release -p corvid-cli
cargo run --locked --release -p corvid-cli -- --version
```

## Verify the install

```sh
corvid --version
corvid doctor
```

`corvid doctor` reports detected Rust tools, model configuration and runtime
settings. Configure a provider when your application needs one. You can check
and run a local program without a model account.
The prebuilt CLI can run the starter and build a web application without
a Rust toolchain. Cargo and rustc are needed for source builds and native
server wrappers.

From a directory where you keep projects, run the commands printed by the installer.

```sh
corvid new hello-corvid
cd hello-corvid
corvid check
corvid run
```

The starter prints a greeting with the current UTC time. Edit `src/main.cor`
and run `corvid run` again to see your change.

Inside a project, `corvid check` and `corvid run` use `src/main.cor`. For a standalone file, pass its path explicitly. If either project file is missing, the CLI identifies the missing manifest or entry file.

## Editor setup

The Corvid language server runs as `corvid-lsp` over standard input and output.
For a source checkout, build it with `cargo build -p corvid-lsp`. The VS Code
extension accepts an explicit executable path in `corvid.lsp.path`. When your editor inherits `CORVID_HOME`, the extension can use the server from that installation even if `corvid-lsp` is outside its `PATH`. The setting and `CORVID_LSP_PATH` take precedence.

The installer looks for the command-line launchers for VS Code, Cursor,
VSCodium and Windsurf. To install the bundled extension manually, open your
editor's **Install from VSIX** command and choose `extensions/corvid.vsix`
under `CORVID_HOME`. Other editors with LSP support can launch the installed
`corvid-lsp` using stdio.

The editor checks open documents with the same frontend as `corvid check`
and displays compiler warnings separately from errors.

## Uninstall

Run the matching installer with its uninstall switch. From a Corvid checkout,
use one of the following commands. Set `CORVID_HOME` first if you selected
a custom installation directory.

```sh
sh install/install.sh --uninstall
```

```powershell
.\install\install.ps1 -Uninstall
```

The installer records the package directories it placed in the install root.
Uninstall removes those directories and keeps other top-level files. Keep
personal files and edits outside package directories. On Unix, uninstall
removes the shell-profile block that still matches this installation. On
Windows, it removes the user PATH entry the installer added and restores
the previous user-level `CORVID_HOME` when this installation is still selected.
Open a new terminal to pick up the changes. Editor extensions can be removed
from the editor's Extensions panel.

Older installations can lack the package receipt. The uninstaller reports
this before changing any files. For those installations, remove the known
Corvid package files from the dedicated installation directory and remove its
PATH and `CORVID_HOME` settings manually.
