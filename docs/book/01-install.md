# Install

## Supported platforms

Corvid runs first-class on macOS, Linux, and Windows. The CI matrix
(`platform-parity`) runs the installer, `corvid doctor`, and the WASM
parity harness on all three on every push.

## Install scripts

Unix (macOS + Linux):

```sh
curl -fsSL https://corvid-lang.org/install.sh | sh
```

Windows (PowerShell):

```powershell
iwr -useb https://corvid-lang.org/install.ps1 | iex
```

The script downloads the platform-appropriate signed release, installs
`corvid` into `~/.corvid/bin`, and adds it to `PATH`. The release tarball
is signed (ed25519); the installer verifies the signature before
extracting.

## From source via cargo

If you have Rust 1.78+ installed:

```sh
cargo install corvid-cli
```

This pulls from crates.io and builds locally. Slower than the install
script, but useful if you are auditing the supply chain.

## Verify the install

```sh
corvid --version
corvid doctor
```

`corvid doctor` checks:

- A C toolchain is installed (the codegen path needs `cc`).
- A WASM toolchain is installed (only required if you target WASM).
- An LLM provider key is configured (only required for prompts that call
  hosted models).
- Replay storage is writable.
- Approval configuration is well-formed.

If `corvid doctor` reports anything red, fix it before going further.
Every check has a one-line "how to fix" hint inline.

## Editor setup

The Corvid LSP ships in the same binary. Pick your editor:

- **VS Code**: install the `corvid-lang.corvid` extension.
- **Neovim**: `lua require'lspconfig'.corvid.setup{}` after installing
  `nvim-lspconfig`.
- **JetBrains**: install the official plugin from the marketplace.

The LSP gives you compile-time-error squiggles for every effect-row,
approval, and grounding violation as you type.

## Uninstall

```sh
corvid self uninstall
```

Removes the binary, removes the PATH entry, and prompts before deleting
any project-local replay caches.
