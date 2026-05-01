# Quantum Phase 0 Implementation Note

## Purpose

This note freezes the exact Phase 0 contracts for the Quantum transaction seam spike in the Foundry fork.

## Frozen Bases

- Foundry fork base: `f1abb2ca347187bb6dea8c3881ca44ce50aab1e7`
- Quantum harness source-of-truth repo: `https://github.com/multivmlabs/quantum-eth.git`
- Quantum harness commit: `8f3612c60f9fa66ea3a09eab99a2e0802f373673`
- Behavioral evidence repo: `https://github.com/multivmlabs/tx-spammer`
- Behavioral evidence commit: `2c25f14a44b8cc88fc41a65f521f1ba8350e7fa4`

## Frozen RPC Contract

- State-changing Quantum submission uses raw `eth_sendRawTransaction`.
- The submitted payload is a native `0x7A` envelope with exactly one call.
- `sender` is explicit and never auto-derived from signer identity.
- `key_id` is explicit and defaults to `0` only for the Phase 0 seam spike.
- `nonce_key` is fixed to `0` in v1.
- Sender-pays only. Fee-payer and sponsorship fields remain out of scope for Phase 0.
- Deploy and future script flows must treat `eth_getTransactionReceipt` as the source of `contractAddress`.
- Ordinary `eth_call` remains the read-path contract.
- KeyVault lifecycle selectors are not simulated through ordinary send/call flows; the stable rejection text is:

```text
KeyVault lifecycle operations (bootstrap/addKey/removeKey/updateKeyAuth) cannot be simulated via eth_call; use explicit lifecycle transaction submission
```

## Frozen KeyVault ABI / Selector Set

- Precompile address: `0x0000000000000000000000000000000000001000`
- `bootstrapKey()`: `0x5e8e7a13`
- `addKey(uint32,bytes,uint8,bytes,uint8,uint8,bytes)`: `0x32bc2919`
- `removeKey(uint32)`: `0xc98f21f4`
- `updateKeyAuth(uint32,bytes,uint8,bytes,uint8)`: `0x8908154b`

## Frozen Signer / Operator Contract

### Shared QuantumWriteContractV1

Every state-changing Quantum write surface must normalize into the same internal contract before signing or broadcast:

- explicit Quantum selection
- explicit `sender`
- auth-lane `key_id`
- `nonce_key = 0`
- one file-backed ML-DSA signer source
- optional bootstrap-only fields
- optional detached artifact for required P256 and ECDSA cosigner flows
- lifecycle target-key / scoped-permission inputs when applicable
- one normalized single-call payload that becomes the native `0x7A` body

### Phase 0 ML-DSA Source

The seam spike accepts exactly one primary signer source:

- `--quantum.primary-seed-file <PATH>`

The file must contain one 32-byte ML-DSA seed encoded as hex, with or without a `0x` prefix. The seed is normalized into bytes before signing.

The Phase 0 implementation uses the public `ml-dsa` crate directly and mirrors the `quantum-eth2` wrapper behavior for deterministic key expansion, signing, and address derivation, so the seam spike does not depend on a private git dependency.

### Detached Artifact Contract (Frozen For Phase 1)

Detached P256 and detached ECDSA flows are frozen as a single versioned artifact schema, even though the Phase 0 code path only proves the primary-only send:

```json
{
  "version": 1,
  "scheme": "p256|ecdsa",
  "signing_hash": "0x...",
  "public_key": "0x...",
  "signature": "0x..."
}
```

Rules:

- the fork owns construction of the request body and canonical signing hash
- detached signers sign exactly that hash
- the artifact `signing_hash` must match the fork-computed hash byte-for-byte
- the schema is shared across `cast send`, `forge create`, and `forge script --broadcast`

## Phase 0 Seam Choice

Phase 0 intentionally does **not** add the full `QuantumNetwork` adapter yet.

Instead, it proves the signer/broadcast seam in `cast send` by:

- using normal Foundry parsing/fill logic to resolve destination, calldata, gas, nonce, and fees
- normalizing those values into `QuantumWriteContractV1`
- signing a native `0x7A` envelope locally with the ML-DSA seed file
- submitting the encoded raw bytes with `eth_sendRawTransaction`
- reusing Foundry’s existing raw-send receipt polling path

This is a spike, not the release architecture. The Phase 1 rule remains:

- `cast send`, `forge create`, and `forge script --broadcast` must converge on one shared Quantum signing and broadcast pipeline

## Pinned Fixture Sources

Tracked fixture directory:

- `testdata/fixtures/quantum/phase0/`

Phase 0 fixture set:

- canonical raw `0x7A` submission example with parity metadata
- deploy receipt example showing `status`, `transactionHash`, and `contractAddress`
- lifecycle simulation rejection example with the stable surfaced message

## Harness Boot Method

The pinned local harness for manual and CI validation remains the Quantum source-of-truth e2e cluster at commit `8f3612c60f9fa66ea3a09eab99a2e0802f373673`.

For Phase 0 closure, the smallest reproducible local proof uses the upstream single-node dev harness. It preserves the same `0x7A` transaction and receipt behavior without requiring the full multi-node cluster:

```bash
# in /Users/ea/repos/quantum-eth2
cargo run --bin quantum-reth -- node --dev --http --http.port 18545 --http.api all
```

Phase 0 validation still relies on the same upstream behavior exercised by:

- `crates/e2e/tests/transactions.rs`
- `crates/e2e/tests/keyvault_lifecycle.rs`

The Foundry fork does not redefine the harness contract locally in Phase 0; it freezes the upstream harness commit and fixture expectations before broader adapter work begins.

## Manual Verification Request

Request the following proof from the operator / reviewer when closing Phase 0.

### 1. Prepare the funded deterministic seed

The upstream dev harness prefunds dev account `0`, whose ML-DSA seed is `sha256("quantum-ml-dsa-dev-0")`.

```bash
printf '0x4f600dd3c20fb7d9e12a3d51ee15ecc74b92e9c020ad1f795a774e96eb5634f4\n' >/tmp/quantum-dev0.seed
```

Frozen addresses used in the proof:

- sender / dev-0: `0x47872C3e8676384B80648D95bEaC2c0C348eF272`
- recipient example / dev-1: `0x9a9eA6B0e3d2984ddB7e8070f1F8B46Af36BF92C`

### 2. Bootstrap the sender with the upstream harness tool

`cast send --quantum` intentionally does not perform KeyVault lifecycle bootstrap in Phase 0, so the sender must be registered first.

```bash
# in /Users/ea/repos/quantum-eth2
cargo run --bin quantum-send-tx -- bootstrap \
  --rpc-url http://127.0.0.1:18545 \
  --fill \
  --dev-index 0
```

Expected result:

- stdout prints a bootstrap tx hash
- stderr includes `status: 0x1`

Observed locally on 2026-04-15:

- bootstrap tx hash: `0xc7734c6501fb4074900015ba3e1e5c7e0990da78d25073e7029ce7399fbe73d8`

### 3. Prove native `0x7A` submission through `cast send --quantum`

Use `--async` for the manual proof. The Phase 0 seam successfully broadcasts without it, but non-async `cast send` currently trips over Foundry receipt deserialization because the upstream harness returns receipt `type: "Pq"`.

```bash
# in /Users/ea/repos/quantum-foundry
TX_HASH=$(cargo +nightly run --bin cast -- send \
  0x9a9eA6B0e3d2984ddB7e8070f1F8B46Af36BF92C \
  --value 2 \
  --async \
  --rpc-url http://127.0.0.1:18545 \
  --quantum \
  --quantum.sender 0x47872C3e8676384B80648D95bEaC2c0C348eF272 \
  --quantum.primary-seed-file /tmp/quantum-dev0.seed)
printf 'TX_HASH=%s\n' "$TX_HASH"
```

Expected result:

- command exits `0`
- stdout is only the transaction hash

Observed locally on 2026-04-15:

- async tx hash: `0xa300521dff7d22ed26eca888e19c09711cb524d88877f99b42f4258e81b197fe`

### 4. Prove the node accepted it as a PQ-native transaction

Query raw RPC directly so no Ethereum-type deserializer gets in the way.

```bash
curl -s -H 'content-type: application/json' \
  --data "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"eth_getTransactionReceipt\",\"params\":[\"$TX_HASH\"]}" \
  http://127.0.0.1:18545

curl -s -H 'content-type: application/json' \
  --data "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"eth_getTransactionByHash\",\"params\":[\"$TX_HASH\"]}" \
  http://127.0.0.1:18545
```

Expected receipt assertions:

- `result.type == "Pq"`
- `result.status == "0x1"`
- `result.from == "0x47872c3e8676384b80648d95beac2c0c348ef272"`
- `result.to == "0x9a9ea6b0e3d2984ddb7e8070f1f8b46af36bf92c"`

Expected transaction assertions:

- `result.type == "0x7a"`
- `result.sender == "0x47872c3e8676384b80648d95beac2c0c348ef272"`
- `result.keyId == "0x0"`
- `result.nonceKey == "0x0"`

### 5. Prove the frozen lifecycle rejection path in the seam

Provide explicit nonce and gas so the command reaches `QuantumWriteContractV1` validation instead of failing early during RPC gas estimation.

```bash
cargo +nightly run --bin cast -- send \
  0x0000000000000000000000000000000000001000 \
  --data 0x5e8e7a13 \
  --async \
  --rpc-url http://127.0.0.1:18545 \
  --nonce 3 \
  --gas-limit 2100000 \
  --gas-price 15 \
  --priority-gas-price 1 \
  --quantum \
  --quantum.sender 0x47872C3e8676384B80648D95bEaC2c0C348eF272 \
  --quantum.primary-seed-file /tmp/quantum-dev0.seed
```

Expected result:

```text
KeyVault lifecycle operations (bootstrap/addKey/removeKey/updateKeyAuth) cannot be simulated via eth_call; use explicit lifecycle transaction submission
```

Note: if nonce / gas are omitted, Foundry may fail earlier during gas estimation with a generic RPC revert instead of surfacing the frozen seam-level rejection string.

## Evidence Status And Remaining Gaps

Confirmed locally on 2026-04-15:

- upstream dev harness boot recipe is now concrete and reproducible
- native `cast send --quantum` submission does reach `eth_sendRawTransaction` and lands successfully on the harness
- raw node RPC shows transaction `type: "0x7a"` and receipt `type: "Pq"`
- the Phase 0 local lifecycle rejection path is reproducible with explicit nonce / gas

Still open after verification:

- broader golden fixtures are still missing for bootstrap raw shape and lifecycle calldata shape
- non-async `cast send --quantum` is not yet operator-clean because receipt parsing does not recognize upstream receipt `type: "Pq"`
- raw `eth_call` against lifecycle selectors on the live harness currently returns `execution reverted` with data `0x202ce609`, not the friendly frozen rejection string; treat this as an upstream RPC-surfacing mismatch that still needs confirmation before Phase 0 is considered fully evidenced end-to-end
