# FFI: C/Rust

Corvid compiles to a signed cdylib that any C-ABI host can dlopen.
Phase 22 ships the C ABI + library mode; Phase 20n-B ships the
bare `(ptr, len)` UTF-8 string ABI that lets every language with
a C FFI call Corvid functions without a wrapper.

## When to use it

- Adding Corvid agents into an existing Rust / C / C++ service.
- Calling Corvid from Go / Java / JS / Swift via their C FFI.
- A signed-build requirement (the cdylib carries an embedded
  DSSE attestation Phase 35-H's bilateral verifier independently
  re-derives).

## Exporting an agent

```corvid
effect refund_effect:
    cost: $50.00
    trust: supervisor_required

tool issue_refund(amount: Float, customer_id: String) -> String dangerous uses refund_effect

pub extern "c"
agent refund_handler(amount: Float, customer_id: String) -> String uses refund_effect:
    approve IssueRefund(amount, customer_id)
    return issue_refund(amount, customer_id)
```

`pub extern "c"` is the FFI marker that exports the agent as a
C-callable symbol. The agent's parameter and return types use
the bare `(ptr, len)` UTF-8 ABI for `String` (Phase 20n-B); other
primitive types pass by value with their natural C-ABI shape.

## Building

```sh
corvid build src/lib.cor --target=cdylib --sign /path/to/signing.key
```

Other useful flags:

```sh
--key-id <ID>          # Opaque key identifier embedded in the DSSE envelope's keyid field
--header               # Emit a companion C header alongside the cdylib
--abi-descriptor       # Emit a companion ABI descriptor JSON alongside the cdylib
--all-artifacts        # Emit every supported companion artifact
```

The signing key can also come from the `CORVID_SIGNING_KEY` env
var if `--sign` is given without a path.

Outputs:

```text
target/cdylib/
├── libmy_app.so          # the binary (or .dylib / .dll)
├── my_app.h              # generated C header (with --header)
└── my_app.abi.json       # ABI descriptor companion (with --abi-descriptor)
```

The binary embeds two symbols:

- `CORVID_ABI_DESCRIPTOR` — the canonical ABI descriptor JSON
  (consumed by `corvid claim --explain` + the bilateral verifier).
- `CORVID_ABI_ATTESTATION` — the DSSE envelope over the descriptor
  (signed with the key from `--sign`).

## Verifying the binary

Before loading the cdylib, verify the embedded attestation:

```sh
corvid receipt verify-abi target/cdylib/libmy_app.so --pubkey /path/to/signing.pub
```

This catches post-link tampering and build-cache drift before the
host runs the binary. The independent bilateral verifier
(`corvid-abi-verify`) rebuilds the descriptor from source and
byte-compares against the embedded one (Phase 35-H).

## Explaining the binary's claims

```sh
corvid claim --explain target/cdylib/libmy_app.so
```

Prints the embedded ABI descriptor + the signature verification
status + the enforced guarantee surface. This is the
operator-facing "what does this binary promise" view; it's also
what `corvid upgrade --check --claims-current <...> --claims-target
<...>` consumes when refusing claim-weakening upgrades (slice 43Q).

## Calling from C

The `--header` flag generates a C header with the bare `(ptr, len)`
struct + every `pub extern "c"` symbol. See
[`docs/internals/effect-spec/abi.md`](../internals/effect-spec/abi.md)
for the canonical ABI shape; the per-target memory-ownership
contract is documented at
[`docs/internals/wasm-abi.md`](../internals/wasm-abi.md).

## Calling from Rust

Treat the cdylib as a normal `extern "C"` library. The
`corvid-abi` crate ships `CorvidString` + the helper types so a
Rust embedder doesn't need to redeclare them by hand.

## Pointers to the registry contracts

| Property | Registry id | Class | Where |
|---|---|---|---|
| ABI descriptor + attestation embedded in cdylib | `abi_descriptor.canonical_json_embedded` + `abi_attestation.dsse_envelope_embedded` | Static / RuntimeChecked | `crates/corvid-abi/` |
| Bilateral verifier reconstructs the descriptor | `abi_attestation.bilateral_verifier_reproduces` | RuntimeChecked | `crates/corvid-abi-verify/` |
| Deploy package binds to the cdylib's bytes | `deploy.attestation_chain` (43O) | RuntimeChecked | `crates/corvid-cli/src/deploy_cmd.rs` |
| Upgrade refuses claim regressions | `upgrade.claim_regression_check` (43Q) | RuntimeChecked | `crates/corvid-cli/src/upgrade_cmd.rs` |
