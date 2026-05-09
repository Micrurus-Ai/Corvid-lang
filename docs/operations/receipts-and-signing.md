# Receipts and signed builds

## What signing buys you

A signed Corvid build produces:

1. A **cdylib** with embedded `CORVID_ABI_DESCRIPTOR`.
2. A **DSSE-signed receipt** describing the build inputs (source SHAs,
   dep tree, toolchain version, timestamp).
3. A **claim block** listing every compile-time guarantee the binary
   carries, signed alongside the binary.

The separate-binary verifier `corvid-abi-verify` rebuilds the ABI
descriptor from source through a separate process and byte-compares
it with the cdylib's embedded descriptor. This catches post-link
tampering and build-cache drift. See
[`docs/security/model.md`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/security/model.md)
for the threat model and TCB.

## Producing a signed build

```sh
corvid build src/lib.cor --target=cdylib --sign
```

Outputs:

```
target/cdylib/
├── libmy_app.so          # the binary, descriptor-embedded
├── libmy_app.so.dsse     # DSSE-signed receipt
├── libmy_app.so.claim    # signed claim block
└── MANIFEST.toml         # build inputs + hashes
```

## Verifying a signed build

```sh
corvid receipt verify target/cdylib/libmy_app.so
```

Runs:

1. The DSSE signature check.
2. The separate-binary descriptor reconstruction (`corvid-abi-verify`).
3. The claim-coverage validator (every claim resolves to a guarantee
   id; every signed guarantee is in the registry).
4. The MANIFEST.toml hash check.

Any failure exits non-zero with a diagnostic naming the failing check.

## Signing keys

Corvid does not manage signing keys. Configure your host:

```toml
[signing]
key_path = "${CORVID_SIGNING_KEY_PATH}"   # env var; never commit the path
```

`corvid doctor` verifies the key is present and well-shaped without
printing it.

## Rotating keys

```sh
corvid auth keys rotate --new-key=<path>
```

Past receipts remain valid (signed by the old key, verifiable against
the published key history). New receipts use the new key.

## Public verifiability

Publish your public key alongside the binary. Third parties verify with:

```sh
corvid receipt verify <binary> --pubkey=<key-url>
```

The verifier uses only the public key + the embedded descriptor + the
signed receipt. No host-secret access is required to verify.

## What signing does NOT defend

- A compromised signing key signs malicious binaries that pass
  verification. Host responsibility.
- A compromised compiler toolchain (rustc, the linker) can produce a
  binary that doesn't match its source. Out of scope for v1.0.

The
[security model](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/security/model.md)
enumerates the trusted computing base.
