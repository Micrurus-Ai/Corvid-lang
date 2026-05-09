# Security model overview

## TL;DR

Corvid's threat model and trusted computing base are documented in
[`docs/security/model.md`](https://github.com/Corvid-lang/Corvid-lang/blob/main/docs/security/model.md).
This page is the executive summary.

## What Corvid defends against

- **Reachability of dangerous calls without authorization.** A binary
  with a missing `approve` does not compile.
- **Silent loss of provenance.** A `Grounded<T>` cannot be unwrapped
  silently; every unwrap is recorded.
- **Effect-row bypass via aliasing.** The source-fuzz corpus exercises
  four classes of attempted bypass; each is caught with the matching
  `guarantee_id`.
- **Build-cache drift and post-link tampering.** A separate-binary ABI
  descriptor verifier rebuilds the descriptor from source through a
  separate process and byte-compares it with the cdylib's embedded
  descriptor.

## What Corvid does NOT defend against

- Compromise of the host signing key (operator responsibility).
- Compromise of the rustc toolchain (out of scope for v1.0).
- Cryptographic primitive failures (we use ed25519, SHA-256, DSSE as
  standardized primitives, not redesigns).
- Formal mechanized proof of the type system (post-v1.0 research
  agenda).
- True second-implementation TCB shrinkage (separate
  parser/resolver/typechecker reaching `AbiDescriptor` independently;
  post-v1.0 consideration).

## How to verify

```sh
corvid contract list                    # the full guarantee registry
corvid claim --explain <id>             # rationale for a specific claim
corvid build --sign                     # signed build, verifier-attested
corvid receipt verify <path>            # third-party receipt verification
```

## Reporting a vulnerability

The security policy and disclosure address live at
[SECURITY.md](https://github.com/Corvid-lang/Corvid-lang/blob/main/SECURITY.md)
in the repo.
