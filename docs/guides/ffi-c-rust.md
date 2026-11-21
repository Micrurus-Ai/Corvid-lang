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

Build with `corvid build calc.cor --target cdylib --header`. On Windows, copy the DLL, its import library and its matching generated header to the consumer. Include `lib_calc.h` and link the host against `calc.lib`. An exported integer crosses the ABI by value. A string input is a borrowed UTF-8 string valid for the call. The generated signature specifies where the observation handle is returned.

```c
uint64_t observation = 0;
const char *value = echo("hello", &observation);
if (value) {
    puts(value);
    corvid_free_string(value);
}
corvid_observation_release(observation);
```

When using the dynamic catalog, check the status from `corvid_call_agent` before reading its output. An unknown agent returns `CORVID_CALL_AGENT_NOT_FOUND`; invalid JSON arguments return `CORVID_CALL_BAD_ARGS`.

## Calling from Rust

Treat the cdylib as a normal `extern "C"` library. The
`corvid-abi` crate ships `CorvidString` + the helper types so a
Rust embedder doesn't need to redeclare them by hand.

## Using a Rust tool in a packaged service

Ship the platform's tool library alongside the extracted Corvid application. For a Windows library named `host.dll`, run `corvid serve src/main.cor --with-tools-cdylib host.dll` from the extraction directory. A declared tool named `echo_string` uses the exported symbol `__corvid_tool_echo_string`. A missing library fails at startup. A missing symbol leaves that tool without a handler and a call to it fails the request.

The callback receives a borrowed UTF-8 JSON array of arguments and its byte length. Return one JSON value in a NUL-terminated string allocated with Rust's `CString::into_raw` and a compatible allocator. The runtime takes ownership using `CString::from_raw`. Returning a null pointer reports failure. The service keeps the loaded library alive for the process lifetime.

## Pointers to the registry contracts

| Property | Registry id | Class | Where |
|---|---|---|---|
| ABI descriptor + attestation embedded in cdylib | `abi_descriptor.canonical_json_embedded` + `abi_attestation.dsse_envelope_embedded` | Static / RuntimeChecked | `crates/corvid-abi/` |
| Bilateral verifier reconstructs the descriptor | `abi_attestation.bilateral_verifier_reproduces` | RuntimeChecked | `crates/corvid-abi-verify/` |
| Deploy package binds to the cdylib's bytes | `deploy.attestation_chain` (43O) | RuntimeChecked | `crates/corvid-cli/src/deploy_cmd.rs` |
| Upgrade refuses claim regressions | `upgrade.claim_regression_check` (43Q) | RuntimeChecked | `crates/corvid-cli/src/upgrade_cmd.rs` |
