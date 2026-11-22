# Install

The installer targets Windows, macOS and Linux. The available architectures depend on the selected release.

## Candidate runtime prerequisites

The Linux x64 and ARM64 archives use the Python 3.11 shared runtime at CLI startup. On Debian 12, install the `libpython3.11` package before running the installer. Both native architectures have passed the installer, language-server, Python host-tool and relocated web-application checks.

The macOS candidate uses the Python framework at `/Library/Frameworks/Python.framework/Versions/<version>/Python`. Both `corvid` and `corvid-lsp` load it at startup. Use a framework installation of CPython 3.14 on Intel Macs or CPython 3.15 on Apple Silicon. A virtual environment alone does not supply that system path.

The Windows x64 and ARM64 candidates passed with matching-architecture CPython 3.14. The embedded Python bridge uses CPython's stable ABI. Put the Python installation directory containing `python3.dll` on `PATH` before starting Corvid. The ddaeaddc candidate also installed, served a relocated application and uninstalled on the managed Windows x64 test device. If Windows denies permission to start the staged CLI, the installer reports its SHA-256 for administrator review.

Run `corvid doctor` to check the installation and configuration. With a prebuilt CLI, missing Rust tools are informational. Source builds and native server wrappers use Cargo and rustc. Invalid configuration values cause the command to fail.

## Install scripts

For manual downloads from a release that includes `SHA256SUMS.txt`, compare your archive's SHA-256 with the line for its exact filename. On Linux use `sha256sum ARCHIVE`, on macOS use `shasum -a 256 ARCHIVE`, and in PowerShell use `Get-FileHash -Algorithm SHA256 -LiteralPath ARCHIVE`. Replace `ARCHIVE` with the downloaded filename.

A candidate workflow run provides the platform archives and a separate `corvid-archive-checksums` artifact. Extract the GitHub artifact download before hashing the Corvid archive inside it. The Corvid archive contains its project license at `LICENSE`, its Rust dependency notices at `lib/corvid-rust-notices.txt`, and its standard library under `std/`.

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

From the project directory, run `corvid upgrade check .` with the newly installed CLI to scan for known migrations, then run the application's tests. For a project that embeds Corvid libraries, the migration guide also shows how to compare the guarantees in two versions. [Upgrade and migration guide](../meta/upgrade-migrations.md)

The installer stages a candidate under `CORVID_HOME` and runs its version
command before replacing package files. A failed download and source build
leave the existing installation usable. On Linux, if moving a staged package
into place fails, the installer restores the previous files with their access
permissions and links intact. After resolving
the cause, rerun the same install command to retry. Keep your own files outside the
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

When building from a Git checkout, `corvid --version` includes the short commit ID and commit date. Moving to another commit updates the version on the next build. Cargo can reuse the CLI on an unchanged checkout, including a linked worktree.

## Verify the install

```sh
corvid --version
corvid doctor
```

`corvid doctor` reports on the Rust and Wasm toolchains, model and approval configuration, Python runtime and replay storage. The starter below runs without model credentials. On a fresh project, the first traced run creates the trace directory.

Inside a project, `corvid check` and `corvid run` use `src/main.cor`. For a standalone file, pass its path explicitly. If either project file is missing, the CLI identifies the missing manifest or entry file.

```sh
corvid new hello-corvid
cd hello-corvid
corvid check
corvid run
```

The greeting includes the current UTC time. To confirm replay works in your installation, run `corvid trace list`, copy the JSONL path for that run, and run `corvid replay TRACE_PATH --source src/main.cor` with that path. The replay should print the original timestamp.

## Editor setup

The Corvid LSP ships in the same binary. Pick your editor:

- **VS Code**: install the `corvid-lang.corvid` extension.
- **Neovim**: `lua require'lspconfig'.corvid.setup{}` after installing
  `nvim-lspconfig`.
- **JetBrains**: install the official plugin from the marketplace.

The LSP gives you compile-time-error squiggles for every effect-row,
approval, and grounding violation as you type.

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
