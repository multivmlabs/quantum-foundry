# Quantum Read-Path Adapter Conformance Checklist

## Purpose

This checklist helps explorer, indexer, analytics, and read-API adapters preserve
Quantum-specific transaction evidence when they decode blocks, transactions,
receipts, logs, and token-transfer views.

This is a read-path document only. It does not define a wallet flow, signing
flow, faucet flow, bridge flow, write transaction flow, production-readiness
claim, security audit, or live-network verification result.

Use this together with:

- [Quantum Phase 0 Implementation Note](./quantum-phase0-implementation-note.md)
- [Quantum Adapter Touchpoints](./quantum-adapter-touchpoints.md)

## Source Boundary

Adapters should treat canonical protocol docs and maintainer-provided public
fixtures as the source of truth. When fixture coverage is partial, report the
gap explicitly instead of implying that untested transaction shapes are covered.

Do not publish raw signatures, public keys, calldata, bytecode, ordinary
addresses, or raw transaction bytes in public conformance reports unless the
maintainers have provided those exact samples as public fixtures.

## P0 Requirements

These fields and behaviors are required for a read adapter to avoid losing core
Quantum transaction evidence.

| Requirement | Minimum behavior | Avoid |
| --- | --- | --- |
| Preserve transaction type `0x7A`. | Store and expose it as a custom EIP-2718 transaction type. | Rejecting the transaction or coercing it into an Ethereum type-2 shape. |
| Preserve explicit `sender`. | Treat `sender` as the account-lane identity supplied by the transaction. | Recovering identity from ECDSA `v/r/s` fields or assuming `from` is signer recovery. |
| Preserve `from` as compatibility data. | Display or expose it only as Ethereum-compatible read-path data when present. | Collapsing `sender`, `from`, cosigner, and fee-payer roles into one identity. |
| Preserve `nonceKey` and `nonce`. | Store queue and sequence dimensions separately. | Flattening the two-dimensional nonce model into a single Ethereum nonce. |
| Preserve `keyId`. | Keep it separate from sender, nonce, and signature fields. | Treating key ID as a nonce alias or hiding it from downstream APIs. |
| Preserve `senderSig` role evidence. | Store presence, role, scheme if known, and redacted size metadata. | Publishing raw signature bytes by default or dropping the field because it is not ECDSA. |
| Preserve receipts and logs. | Index receipt status, log indexes, topics, and token-transfer classifications even for custom transaction types. | Skipping receipt/log extraction when the transaction type is not a known Ethereum type. |
| Preserve unknown fields. | Keep unrecognized Quantum fields under `unknown_fields` or an equivalent extension bucket. | Silently dropping fields during normalization. |

## P1 Requirements

These behaviors make the adapter useful for richer explorer, indexer, and API
surfaces without claiming complete coverage of every future transaction shape.

| Requirement | Minimum behavior | Avoid |
| --- | --- | --- |
| Separate optional signature roles. | Keep sender signature, optional cosigner signature, and optional fee-payer signature evidence distinct. | Merging all signatures into one generic signature field. |
| Preserve optional fee-payer fields. | Store presence and role metadata separately from sender authorization. | Treating fee payer as the account sender. |
| Preserve bootstrap/key material shape. | Record role, presence, and byte length only unless the source fixture is explicitly public. | Publishing raw public-key material in public reports. |
| Preserve call and creation shape. | Store call count, contract-creation flag, selector classes, and redacted calldata or bytecode size. | Flattening batched calls or creation into a single opaque transfer. |
| Preserve millisecond timestamp data. | Retain `timestampMillisPart` or equivalent sub-second timestamp fields when present. | Recomputing block identity from standard Ethereum header fields alone. |
| Warn when fields are hidden. | Surface warnings when a UI, GraphQL schema, or downstream API omits Quantum-only fields. | Letting a green status imply complete Quantum support while fields are inaccessible. |

## P2 Reporting

High-quality adapter reports should be machine-readable enough for maintainers
and downstream teams to repeat.

- Emit pass, fail, warning, and not-covered rows.
- Label evidence source as maintainer-provided fixture, canonical protocol doc,
  upstream source, or manual review.
- Separate facts observed in input/output from unsupported compatibility claims.
- Record schema-drift warnings when new fields appear.
- Include redaction status for each signature, key, calldata, bytecode, address,
  and raw-byte field.

## Scenario Matrix

| Scenario | P0 pass signal | Common failure |
| --- | --- | --- |
| Explorer transaction page | Shows custom type, explicit sender, key ID, nonce dimensions, signature role metadata, and receipt/log facts. | Renders the transaction as a normal ECDSA transfer or hides it as an unknown type. |
| Indexer handler | Handler can route `0x7A` transactions, read Quantum-only fields, and still classify logs/token transfers. | Schema drops unknown fields before handlers see them. |
| Raw JSON API | JSON keeps `type`, `sender`, `nonceKey`, `nonce`, `keyId`, signature-role metadata, receipts, logs, and unknown fields. | Normalization coerces the transaction into a legacy Ethereum shape. |
| GraphQL API | Quantum-only fields are queryable directly or available through an extension bucket. | Schema rejects fields that are not present in standard Ethereum transactions. |
| Token-transfer view | Token activity remains visible when it comes from a custom transaction receipt. | Token activity disappears because the parent transaction type is custom. |
| Public conformance report | Report distinguishes pass, warning, fail, and not-covered rows with source labels and redaction status. | Report implies full adapter support from a bounded fixture set. |

## Redaction Guidance

Allowed public report details:

- field presence,
- role labels,
- byte lengths,
- selector classes,
- topic counts,
- event classes,
- source labels,
- pass, fail, warning, or not-covered status.

Avoid publishing by default:

- raw transaction hashes,
- ordinary addresses,
- signatures,
- public keys,
- calldata,
- bytecode,
- raw transaction bytes,
- raw log payloads.

## Reviewer Checklist

Before treating an adapter report as useful, check:

- Does it preserve `0x7A` without coercion?
- Does it preserve explicit `sender` without ECDSA recovery assumptions?
- Does it keep `nonceKey`, `nonce`, and `keyId` separate?
- Does it preserve signature-role evidence without exposing raw signature bytes?
- Does it index receipts, logs, and token-transfer views for custom transaction
  types?
- Does it preserve unknown fields for forward compatibility?
- Does it clearly label untested scenarios as not covered?
- Does it avoid wallet, signing, production, audit, or live-network claims?
