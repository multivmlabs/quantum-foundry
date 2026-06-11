<br>
<br>

<p align="center">
  <a href="https://quantum.systems">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://quantum.systems/images/logo-white.svg">
      <img alt="tempo combomark" src="https://quantum.systems/images/logo-white.svg" width="auto" height="120">
    </picture>
  </a>
</p>

<br>
<br>

# Quantum Foundry

Quantum is a post-quantum-ready EVM execution environment with a dedicated native transaction type (`0x7A`), explicit account lanes, and an ML-DSA-44 primary signer with optional detached classical cosigners.

`Quantum Foundry` is a custom fork of [Foundry](https://github.com/foundry-rs/foundry) that integrates Quantum's native envelope, KeyVault lifecycle UX, and detached-cosigner contract directly into the familiar Foundry developer workflow.

This fork is a drop-in replacement for upstream Foundry while Quantum-specific features are being stabilized; it tracks Foundry commit `f1abb2ca347187bb6dea8c3881ca44ce50aab1e7` and the Quantum harness commit `8f3612c60f9fa66ea3a09eab99a2e0802f373673`. See [`docs/dev/quantum-phase0-implementation-note.md`](./docs/dev/quantum-phase0-implementation-note.md) for the frozen RPC, signer, and ABI contract.

## Installation

```sh
curl -L https://raw.githubusercontent.com/multivmlabs/quantum-foundry/HEAD/foundryup/install | bash
foundryup --network quantum
```

This installs the Quantum-enabled `forge`, `cast`, `anvil`, and `chisel` from this fork's GitHub Releases into `~/.foundry-quantum/bin/`. The separate directory means quantum-foundry coexists with an existing upstream Foundry install at `~/.foundry/` — neither installer overwrites the other's binaries. If both are on your `PATH`, the one listed earlier wins for commands like `forge` and `cast`.

<details>
<summary>Building from source (contributors and unsupported platforms)</summary>

```sh
cargo build --release -p forge -p cast -p anvil -p chisel
```

The `target/release` binaries are drop-in replacements for upstream `forge`, `cast`, `anvil`, and `chisel`.

</details>

## Devnet

Quantum's public devnet is available for early testing. **Note: the devnet is unstable — state may be wiped, chain ID may change, and downtime should be expected. A public testnet is targeted for mid-2026.**

| Property           | Value                                 |
| ------------------ | ------------------------------------- |
| **Network Name**   | Quantum Devnet                        |
| **Chain ID**       | `1337`                                |
| **HTTP URL**       | `https://devnet2.rpc.quantum.systems` |
| **Block Explorer** | `https://quantumscan.org/`            |

Example:

```sh
cast block-number --rpc-url https://devnet2.rpc.quantum.systems
```

## Changeset

Key Quantum extensions on top of upstream Foundry:

- In `cast send`:
  - `--quantum`: opt into the explicit Quantum adapter path (selection is explicit in v1, not inferred from chain ID).
  - `--quantum.sender <ADDRESS>`: explicit Quantum account-lane address. Quantum writes never auto-derive the sender from the signing key.
  - `--quantum.key-id <KEY_ID>`: account-lane key ID; defaults to `0` for ordinary v1 flows.
  - `--quantum.primary-seed-file <PATH>`: canonical v1 ML-DSA-44 signer seed (single 32-byte hex seed, with or without `0x`).
  - `--quantum.cosigner-artifact <PATH>`: optional detached v1 cosigner artifact JSON; schemes `p256` and `ecdsa` are supported and the artifact's `signing_hash` must match the fork-computed Quantum signing hash byte-for-byte.
  - KeyVault lifecycle selectors (`bootstrapKey`, `addKey`, `removeKey`, `updateKeyAuth`) are rejected by the `cast send` pre-build guard and must be submitted through `cast quantum` instead.

- In `cast quantum` (new subcommand group for KeyVault lifecycle UX):
  - `cast quantum bootstrap`: primary-only `bootstrapKey()` through the shared `0x7A` signing pipeline.
  - `cast quantum add-key`: `addKey(...)` with `--auth-key-id` (signer lane) distinct from `--target-key-id` (entry being added) so the two lanes cannot be confused.
  - `cast quantum remove-key`: `removeKey(uint32)`.
  - `cast quantum update-key-auth`: `updateKeyAuth(...)` with the same auth-lane / target-lane separation.
  - All lifecycle writes auto-apply the fixed `QUANTUM_LIFECYCLE_GAS_FLOOR` (2,100,000) because validator-published transient state cannot be reproduced by `eth_estimateGas`.

- In `cast call`:
  - Fails closed on KeyVault lifecycle selectors with the frozen `QUANTUM_CALL_LIFECYCLE_REJECTION_MESSAGE`, preserving ordinary read paths on standard RPC simulation.

- In `forge create`:
  - `--quantum` and the `--quantum.*` flags route CREATE through the shared Quantum adapter with the same explicit sender / key-id / seed / cosigner contract as `cast send`.

- In `forge script`:
  - `--quantum` intentionally fails closed in v1: scripted Quantum broadcasts are not yet wired through the native `0x7A` adapter. Split scripted write flows into individual `cast send --quantum` / `forge create --quantum` invocations, or rerun without `--quantum` against an Ethereum-compatible network.

- Additionally:
  - A shared `QuantumWriteRequestV1` write contract with fail-closed v1 validation: `nonce_key` must be `0`, multi-call bundles are rejected, and lifecycle-selector misuse is caught before signing.
  - Detached cosigner artifact v1 (`version = 1`, `scheme`, `signing_hash`, `public_key`, `signature`) with composite-signature RLP layout using scheme bytes `0x01` (ML-DSA), `0x02` (P256), `0x03` (ECDSA).
  - KeyVault lifecycle calldata builders derived from a shared `sol!` interface whose selectors are asserted byte-for-byte against the Phase 0 frozen constants.
  - A pinned golden fixture (`testdata/fixtures/quantum/phase0/raw-send-primary.json`) that locks the raw `0x7A` envelope bytes end-to-end.

See [`docs/dev/quantum-adapter-touchpoints.md`](./docs/dev/quantum-adapter-touchpoints.md) for the full manifest of Quantum-modified files.

<br>
<br>

<div align="center">
  <img src=".github/assets/banner.png" alt="Foundry banner" />

&nbsp;

[![Github Actions][gha-badge]][gha-url] [![Telegram Chat][tg-badge]][tg-url] [![Telegram Support][tg-support-badge]][tg-support-url]

[gha-badge]: https://img.shields.io/github/actions/workflow/status/foundry-rs/foundry/test.yml?branch=master&style=flat-square
[gha-url]: https://github.com/foundry-rs/foundry/actions
[tg-badge]: https://img.shields.io/endpoint?color=neon&logo=telegram&label=chat&style=flat-square&url=https%3A%2F%2Ftg.sumanjay.workers.dev%2Ffoundry_rs
[tg-url]: https://t.me/foundry_rs
[tg-support-badge]: https://img.shields.io/endpoint?color=neon&logo=telegram&label=support&style=flat-square&url=https%3A%2F%2Ftg.sumanjay.workers.dev%2Ffoundry_support
[tg-support-url]: https://t.me/foundry_support

**[Install](https://getfoundry.sh/getting-started/installation)**
| [Docs][foundry-docs]
| [Benchmarks](https://www.getfoundry.sh/benchmarks)
| [Developer Guidelines](./docs/dev/README.md)
| [Contributing](./CONTRIBUTING.md)
| [Crate Docs](https://foundry-rs.github.io/foundry)

</div>

---

Blazing fast, portable and modular toolkit for Ethereum application development, written in Rust.

- [**Forge**](https://getfoundry.sh/forge) — Build, test, fuzz, debug and deploy Solidity contracts.
- [**Cast**](https://getfoundry.sh/cast) — Swiss Army knife for interacting with EVM smart contracts, sending transactions and getting chain data.
- [**Anvil**](https://getfoundry.sh/anvil) — Fast local Ethereum development node.
- [**Chisel**](https://getfoundry.sh/chisel) — Fast, utilitarian and verbose Solidity REPL.

![Demo](.github/assets/demo.gif)

## Installation

```sh
curl -L https://foundry.paradigm.xyz | bash
foundryup
```

See the [installation guide](https://getfoundry.sh/getting-started/installation) for more details.

## Getting Started

Initialize a new project, build and test:

```sh
forge init counter && cd counter
forge build
forge test
```

Interact with a live network:

```sh
cast block-number --rpc-url https://eth.merkle.io
cast balance vitalik.eth --ether --rpc-url https://eth.merkle.io
```

Fork mainnet locally:

```sh
anvil --fork-url https://eth.merkle.io
```

Read the [Foundry Docs][foundry-docs] to learn more.

## Contributing

Contributions are welcome and highly appreciated. To get started, check out the [contributing guidelines](./CONTRIBUTING.md).

Join our [Telegram][tg-url] to chat about the development of Foundry.

## Support

Having trouble? Check the [Foundry Docs][foundry-docs], join the [support Telegram][tg-support-url], or [open an issue](https://github.com/foundry-rs/foundry/issues/new).

#### License

<sup>
Licensed under either of <a href="LICENSE-APACHE">Apache License, Version
2.0</a> or <a href="LICENSE-MIT">MIT license</a> at your option.
</sup>

<br>

<sub>
Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in these crates by you, as defined in the Apache-2.0 license,
shall be dual licensed as above, without any additional terms or conditions.
</sub>

[foundry-docs]: https://getfoundry.sh
