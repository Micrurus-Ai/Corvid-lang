# FFI: C/Rust

Corvid can export functions from a native shared library, link host functions into a native executable, or load host tools into a running service. Generate a header for the library you build so the caller uses the signatures for that target.

## When to use it

- Adding Corvid agents into an existing Rust / C / C++ service.
- Calling Corvid from Go / Java / JS / Swift via their C FFI.
- A signed-build requirement (the cdylib carries an embedded
  DSSE attestation Phase 35-H's bilateral verifier independently
  re-derives).

## Loading host tools into a Corvid service

A Corvid service can call functions in a native shared library. Ship the library for the deployment machine alongside the extracted web bundle. From that directory on Windows, run:

```sh
corvid serve src/main.cor --with-tools-cdylib ./tools.dll
```

For a declaration such as `tool echo_string(value: String) -> String`, export the symbol `__corvid_tool_echo_string`. Its C ABI receives a pointer to a UTF-8 JSON argument array, the byte length, and a user-data pointer. The service passes null for user data. The input bytes are borrowed for the duration of the call. For the argument `"hello"`, the input is `["hello"]` and a valid echo result is the JSON string `"hello"`.

The callback transfers ownership of a NUL-terminated JSON result to the caller. Use a buffer compatible with the Corvid runtime allocator. Hosts using the runtime C API can obtain one with `corvid_alloc_result(content_len)`, fill exactly that many bytes and leave the final NUL in place. The service keeps the loaded library alive for the process lifetime.

An unreadable library fails startup. A library may implement a subset of the declared tools. Calling a tool with no registered handler fails that request with HTTP 500. Other registered tools remain available.

## Linking a Rust function into a native executable

For an integer calculation, declare `tool twice(value: Int) -> Int` in Corvid. Implement the symbol with matching argument and return types in `double.rs`.

```rust
#[no_mangle]
pub extern "C" fn __corvid_tool_twice(value: i64) -> i64 {
    value * 2
}
```

On Windows, build and link the archive from the project directory.

```sh
rustc --crate-type staticlib double.rs -o double.lib
corvid build main.cor --target native --with-tools-lib ./double.lib
```

An agent that returns `twice(21)` prints `42` when the built executable runs. Both integers pass by value. The native linker needs the archive at build time. The executable can then run in another directory without it. An absent archive yields a build error that names the path.

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
C-callable symbol. On the native shared-library boundary, a Corvid `Int` maps to `int64_t`. The direct `String` wrapper accepts a borrowed NUL-terminated UTF-8 string. Pass an output pointer for an observation handle or null if your host does not need observations.

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

The CLI prints the paths of the generated files. For example, building `calc.cor` on Windows with `--target cdylib --header` produces `target/release/calc.dll` and `target/release/lib_calc.h`. Ship the platform-specific library and provide the matching header to C consumers.

A signed binary embeds two symbols:

- `CORVID_ABI_DESCRIPTOR` — the canonical ABI descriptor JSON
  (consumed by `corvid claim --explain` + the bilateral verifier).
- `CORVID_ABI_ATTESTATION` — the DSSE envelope over the descriptor
  (signed with the key from `--sign`).

## Verifying the binary

Before loading the cdylib, verify the embedded attestation:

```sh
corvid receipt verify-abi target/release/libmy_app.so --pubkey /path/to/signing.pub
```

This catches post-link tampering and build-cache drift before the
host runs the binary. The independent bilateral verifier
(`corvid-abi-verify`) rebuilds the descriptor from source and
byte-compares against the embedded one (Phase 35-H).

## Explaining the binary's claims

```sh
corvid claim --explain target/release/libmy_app.so
```

Prints the embedded ABI descriptor + the signature verification
status + the enforced guarantee surface. This is the
operator-facing "what does this binary promise" view; it's also
what `corvid upgrade --check --claims-current <...> --claims-target
<...>` consumes when refusing claim-weakening upgrades (slice 43Q).

## Calling from C

The generated header gives the exact parameter order. For example, an exported `echo(value: String) -> String` has this direct wrapper.

```c
const char* echo(const char* value, uint64_t* out_observation_handle);
```

Keep the input valid during the call. Copy the returned text if you need to keep it, then call `corvid_free_string` once. Release a returned observation handle with `corvid_observation_release`.

For dynamic calls, use `corvid_call_agent` and inspect its status. An unknown agent returns `AgentNotFound`. Invalid arguments return `BadArgs` and may include a JSON error message. Release each returned JSON buffer, including error detail, with `corvid_free_result`. Use the length output to read the supplied bytes before releasing them.

## Calling from Rust

Use the generated host bindings or declare the signatures in the generated C header as Rust `extern "C"` functions. Keep returned strings as raw pointers until you copy their contents and release them with the library's matching function.

## Pointers to the registry contracts

| Property | Registry id | Class | Where |
|---|---|---|---|
| ABI descriptor + attestation embedded in cdylib | `abi_descriptor.canonical_json_embedded` + `abi_attestation.dsse_envelope_embedded` | Static / RuntimeChecked | `crates/corvid-abi/` |
| Bilateral verifier reconstructs the descriptor | `abi_attestation.bilateral_verifier_reproduces` | RuntimeChecked | `crates/corvid-abi-verify/` |
| Deploy package binds to the cdylib's bytes | `deploy.attestation_chain` (43O) | RuntimeChecked | `crates/corvid-cli/src/deploy_cmd.rs` |
| Upgrade refuses claim regressions | `upgrade.claim_regression_check` (43Q) | RuntimeChecked | `crates/corvid-cli/src/upgrade_cmd.rs` |
