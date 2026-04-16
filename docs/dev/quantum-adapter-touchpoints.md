# Quantum Adapter Touchpoints

## Purpose

This document freezes the intended Quantum patch surface on top of upstream Foundry commit `f1abb2ca347187bb6dea8c3881ca44ce50aab1e7` so later rebases can distinguish expected adapter work from incidental drift.

Use this together with `quantum-phase0-implementation-note.md` when widening the Quantum integration.

## Frozen Base

- Upstream Foundry base: `f1abb2ca347187bb6dea8c3881ca44ce50aab1e7`
- Quantum harness commit: `8f3612c60f9fa66ea3a09eab99a2e0802f373673`
- Phase 0 contract reference: `docs/dev/quantum-phase0-implementation-note.md`

## Landed Touchpoints

These files are already intentionally diverged from upstream as part of the Phase 0 seam spike or the landed Phase 1 adapter work.

- `.github/workflows/ci-tempo.yml`
  - carries the frozen Quantum fork base and harness commit in CI metadata
- `docs/dev/README.md`
  - indexes the Quantum implementation note and this touchpoint manifest
- `docs/dev/quantum-phase0-implementation-note.md`
  - freezes the Phase 0 RPC, signer, and operator contract
- `crates/cli/src/opts/mod.rs`
- `crates/cli/src/opts/transaction.rs`
- `crates/cli/src/opts/quantum.rs`
  - add explicit Quantum CLI selection and the Phase 0 signer input contract
- `crates/cast/src/lib.rs`
- `crates/cast/src/cmd/send.rs`
- `crates/cast/src/quantum.rs`
  - implement the Phase 0 seam spike that normalizes a `cast send` flow into `QuantumWriteContractV1`, signs a native `0x7A` envelope, and submits it with `eth_sendRawTransaction`
- `crates/common/src/transactions/builder.rs`
  - carries the shared `QuantumWriteRequestV1` model, canonical `0x7A` body encoding, and v1 fail-closed validation rules
- `crates/cli/src/opts/network.rs`
  - adds the first-class `Quantum` network enum value while keeping v1 selection explicit instead of chain-ID inferred
- `crates/cast/src/args.rs`
  - wires `--network quantum` through `cast` read/decode dispatch and fails closed on raw paths that still need a real `QuantumNetwork` adapter
- `crates/cast/src/cmd/da_estimate.rs`
  - rejects `--network quantum` explicitly instead of silently falling back to Ethereum behavior

## Reserved Phase 1 Adapter Patch Points

These are the Phase 1 surfaces that should absorb the shared adapter work instead of spreading Quantum fields across unrelated codepaths.

- `crates/cast/src/cmd/send.rs`
  - replace the Phase 0 local-only seam with the shared Quantum adapter path while keeping Quantum selection explicit

## Reserved Follow-On Write Surfaces

These files already match the plan's Tempo-based seam inventory and should stay the only intended write-path expansion points after the shared adapter exists.

- `crates/cast/src/tx.rs`
  - request preparation, fill behavior, and raw-send plumbing shared by `cast`
- `crates/forge/src/cmd/create.rs`
  - CREATE translation, explicit Quantum dispatch, and receipt-derived contract address handling
- `crates/script/src/broadcast.rs`
  - shared scripted broadcast path and fail-closed rejection for unsupported Quantum script shapes

## Rebase Rule

If a future rebase requires Quantum changes outside the files listed here, treat that as an intentional design decision and update this manifest in the same change.
