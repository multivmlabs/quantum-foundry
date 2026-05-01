use std::{fs, path::Path};

use alloy_network::TransactionBuilder;
use alloy_primitives::{Address, B256, Bytes, TxKind, keccak256};
use alloy_rlp::{BufMut, Encodable, Header as RlpHeader};
use eyre::{Result, bail};
use ml_dsa::{KeyGen, MlDsa44, signature::Keypair};
use serde::{Deserialize, Serialize};

use crate::{
    QUANTUM_BOOTSTRAP_SELECTOR, QUANTUM_KEYVAULT_ADDRESS, QUANTUM_TX_TYPE_ID, QuantumWriteRequestV1,
};
use foundry_primitives::QuantumTransactionRequest;

pub const QUANTUM_ML_DSA_SCHEME: u8 = 0x01;
pub const QUANTUM_P256_SCHEME: u8 = 0x02;
pub const QUANTUM_ECDSA_SCHEME: u8 = 0x03;
pub const ML_DSA_PUBLIC_KEY_BYTES: usize = 1312;
pub const ML_DSA_SIGNATURE_BYTES: usize = 2420;
pub const ML_DSA_SEED_BYTES: usize = 32;
pub const DETACHED_CLASSICAL_SIGNATURE_BYTES: usize = 64;

pub const QUANTUM_DETACHED_ARTIFACT_VERSION: u8 = 1;
pub const QUANTUM_DETACHED_SCHEME_P256: &str = "p256";
pub const QUANTUM_DETACHED_SCHEME_ECDSA: &str = "ecdsa";
pub const PHASE0_FOUNDY_BASE_COMMIT: &str = "f1abb2ca347187bb6dea8c3881ca44ce50aab1e7";
pub const PHASE0_QUANTUM_HARNESS_COMMIT: &str = "8f3612c60f9fa66ea3a09eab99a2e0802f373673";
pub const PHASE0_TX_SPAMMER_EVIDENCE_COMMIT: &str = "2c25f14a44b8cc88fc41a65f521f1ba8350e7fa4";
pub const QUANTUM_ADD_KEY_SELECTOR: [u8; 4] = [0x32, 0xbc, 0x29, 0x19];
pub const QUANTUM_REMOVE_KEY_SELECTOR: [u8; 4] = [0xc9, 0x8f, 0x21, 0xf4];
pub const QUANTUM_UPDATE_KEY_AUTH_SELECTOR: [u8; 4] = [0x89, 0x08, 0x15, 0x4b];
pub const QUANTUM_SEND_LIFECYCLE_REJECTION_MESSAGE: &str = "KeyVault lifecycle operations beyond bootstrapKey() require explicit lifecycle transaction submission";
/// Stable rejection message surfaced when a caller tries to simulate a KeyVault
/// lifecycle selector through `cast call` / `eth_call`. Matches the Phase 0 frozen
/// contract in `docs/dev/quantum-phase0-implementation-note.md`.
pub const QUANTUM_CALL_LIFECYCLE_REJECTION_MESSAGE: &str = "KeyVault lifecycle operations (bootstrap/addKey/removeKey/updateKeyAuth) cannot be simulated via eth_call; use explicit lifecycle transaction submission";

/// Fixed gas limit for Quantum KeyVault bootstrap and lifecycle transactions.
///
/// KeyVault bootstrap writes depend on validator-published transient state that
/// `eth_estimateGas` simulation cannot reproduce, so the reference `quantum-send-tx`
/// CLI skips estimation and uses this floor directly. Mirrors `LIFECYCLE_GAS_FLOOR`
/// in `quantum-eth2/bin/send-tx/src/main.rs`.
pub const QUANTUM_LIFECYCLE_GAS_FLOOR: u64 = 2_100_000;

pub const QUANTUM_SEND_UNSUPPORTED_LIFECYCLE_SELECTORS: [[u8; 4]; 3] =
    [QUANTUM_ADD_KEY_SELECTOR, QUANTUM_REMOVE_KEY_SELECTOR, QUANTUM_UPDATE_KEY_AUTH_SELECTOR];

/// All KeyVault lifecycle selectors that are unsupported through `cast call` /
/// `eth_call`. `bootstrapKey()` is included here because, unlike ordinary sends,
/// bootstrap can never be simulated from the read path.
pub const QUANTUM_CALL_UNSUPPORTED_LIFECYCLE_SELECTORS: [[u8; 4]; 4] = [
    QUANTUM_BOOTSTRAP_SELECTOR,
    QUANTUM_ADD_KEY_SELECTOR,
    QUANTUM_REMOVE_KEY_SELECTOR,
    QUANTUM_UPDATE_KEY_AUTH_SELECTOR,
];

/// Returns `true` if the given calldata targets a KeyVault lifecycle selector
/// that must not be simulated through `cast call` / `eth_call`.
pub fn quantum_call_is_unsupported_lifecycle_calldata(input: &[u8]) -> bool {
    if input.len() < 4 {
        return false;
    }
    let selector = [input[0], input[1], input[2], input[3]];
    QUANTUM_CALL_UNSUPPORTED_LIFECYCLE_SELECTORS.contains(&selector)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetachedArtifactV1 {
    pub version: u8,
    pub scheme: String,
    pub signing_hash: String,
    pub public_key: String,
    pub signature: String,
}

/// Detached cosigner scheme supported by the shared v1 Quantum signer surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DetachedCosignerScheme {
    P256,
    Ecdsa,
}

impl DetachedCosignerScheme {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::P256 => QUANTUM_DETACHED_SCHEME_P256,
            Self::Ecdsa => QUANTUM_DETACHED_SCHEME_ECDSA,
        }
    }

    fn from_artifact_scheme(scheme: &str) -> Result<Self> {
        match scheme {
            QUANTUM_DETACHED_SCHEME_P256 => Ok(Self::P256),
            QUANTUM_DETACHED_SCHEME_ECDSA => Ok(Self::Ecdsa),
            other => {
                bail!(
                    "unsupported Quantum detached cosigner scheme `{other}`; expected one of `p256`, `ecdsa`"
                )
            }
        }
    }
}

/// Parsed detached cosigner artifact ready for composite-signature attachment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetachedCosigner {
    pub scheme: DetachedCosignerScheme,
    pub signing_hash: B256,
    pub public_key: Vec<u8>,
    pub signature: [u8; DETACHED_CLASSICAL_SIGNATURE_BYTES],
}

impl DetachedCosigner {
    /// Parse and validate a v1 detached artifact from raw JSON bytes.
    pub fn from_artifact_json(bytes: &[u8]) -> Result<Self> {
        let artifact: DetachedArtifactV1 = serde_json::from_slice(bytes)
            .map_err(|err| eyre::eyre!("failed to parse Quantum detached artifact: {err}"))?;
        Self::from_artifact(artifact)
    }

    /// Parse and validate a v1 detached artifact loaded from disk.
    pub fn from_artifact_file(path: &Path) -> Result<Self> {
        let bytes = fs::read(path).map_err(|err| {
            eyre::eyre!("failed to read Quantum detached artifact `{}`: {err}", path.display())
        })?;
        Self::from_artifact_json(&bytes)
    }

    /// Validate and normalize a parsed artifact into its runtime form.
    pub fn from_artifact(artifact: DetachedArtifactV1) -> Result<Self> {
        if artifact.version != QUANTUM_DETACHED_ARTIFACT_VERSION {
            bail!(
                "unsupported Quantum detached artifact version `{}`; expected `{}`",
                artifact.version,
                QUANTUM_DETACHED_ARTIFACT_VERSION
            );
        }

        let scheme = DetachedCosignerScheme::from_artifact_scheme(&artifact.scheme)?;
        let signing_hash = parse_hex_b256(&artifact.signing_hash, "signing_hash")?;
        let public_key = parse_hex_bytes(&artifact.public_key, "public_key")?;
        if public_key.is_empty() {
            bail!("Quantum detached artifact public_key must not be empty");
        }
        let signature_bytes = parse_hex_bytes(&artifact.signature, "signature")?;
        let signature =
            <[u8; DETACHED_CLASSICAL_SIGNATURE_BYTES]>::try_from(signature_bytes.as_slice())
                .map_err(|_| {
                    eyre::eyre!(
                        "Quantum detached {} signature must be {} bytes",
                        scheme.as_str(),
                        DETACHED_CLASSICAL_SIGNATURE_BYTES
                    )
                })?;

        Ok(Self { scheme, signing_hash, public_key, signature })
    }

    fn into_signing_key_signature(self) -> SigningKeySignature {
        match self.scheme {
            DetachedCosignerScheme::P256 => SigningKeySignature::P256 { signature: self.signature },
            DetachedCosignerScheme::Ecdsa => {
                SigningKeySignature::Ecdsa { signature: self.signature }
            }
        }
    }
}

fn parse_hex_bytes(value: &str, field: &str) -> Result<Vec<u8>> {
    let trimmed = value.trim();
    let hex = trimmed.strip_prefix("0x").or_else(|| trimmed.strip_prefix("0X")).unwrap_or(trimmed);
    alloy_primitives::hex::decode(hex)
        .map_err(|err| eyre::eyre!("Quantum detached artifact `{field}` is not valid hex: {err}"))
}

fn parse_hex_b256(value: &str, field: &str) -> Result<B256> {
    let bytes = parse_hex_bytes(value, field)?;
    if bytes.len() != 32 {
        bail!("Quantum detached artifact `{field}` must be 32 bytes");
    }
    Ok(B256::from_slice(&bytes))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuantumSignedPayload {
    pub raw_transaction: Vec<u8>,
    pub signing_hash: B256,
    pub sender: Address,
    pub key_id: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuantumPhase0RawFixture {
    pub tx_type: String,
    pub sender: String,
    pub key_id: u32,
    pub nonce_key: String,
    pub chain_id: u64,
    pub signing_hash: String,
    pub raw_transaction: String,
    pub raw_transaction_hash: String,
    pub foundry_base_commit: String,
    pub quantum_harness_commit: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CompositeSignature {
    primary: SigningKeySignature,
    cosigner: Option<SigningKeySignature>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SigningKeySignature {
    MlDsa44 { signature: Box<[u8; ML_DSA_SIGNATURE_BYTES]> },
    P256 { signature: [u8; DETACHED_CLASSICAL_SIGNATURE_BYTES] },
    Ecdsa { signature: [u8; DETACHED_CLASSICAL_SIGNATURE_BYTES] },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct QuantumSigned {
    tx: QuantumWriteRequestV1,
    sender_sig: CompositeSignature,
}

pub fn parse_seed_file(path: &Path) -> Result<[u8; ML_DSA_SEED_BYTES]> {
    let contents = fs::read_to_string(path)?;
    let trimmed = contents.trim();
    let hex = trimmed.strip_prefix("0x").or_else(|| trimmed.strip_prefix("0X")).unwrap_or(trimmed);
    let bytes = alloy_primitives::hex::decode(hex)?;
    let seed = <[u8; ML_DSA_SEED_BYTES]>::try_from(bytes.as_slice()).map_err(|_| {
        eyre::eyre!(
            "Quantum ML-DSA seed file must contain exactly {ML_DSA_SEED_BYTES} bytes of hex"
        )
    })?;
    Ok(seed)
}

pub fn derive_primary_pubkey(primary_seed: [u8; ML_DSA_SEED_BYTES]) -> Bytes {
    let keypair = <MlDsa44 as KeyGen>::from_seed(&ml_dsa::B32::from(primary_seed));
    Bytes::copy_from_slice(keypair.verifying_key().encode().as_ref())
}

pub fn sign_quantum_transaction_request(
    tx: QuantumTransactionRequest,
    primary_seed: [u8; ML_DSA_SEED_BYTES],
) -> Result<QuantumSignedPayload> {
    sign_quantum_transaction_request_with_cosigner(tx, primary_seed, None)
}

pub fn sign_quantum_transaction_request_with_cosigner(
    mut tx: QuantumTransactionRequest,
    primary_seed: [u8; ML_DSA_SEED_BYTES],
    cosigner: Option<DetachedCosigner>,
) -> Result<QuantumSignedPayload> {
    // For bootstrap writes, a caller-supplied `init_primary_pubkey` must match
    // the key derived from the signing seed; otherwise the operator would
    // initialize a key they cannot sign with. Auto-fill when omitted.
    if quantum_transaction_request_is_bootstrap(&tx) {
        let derived = derive_primary_pubkey(primary_seed);
        match tx.init_primary_pubkey.as_ref() {
            None => tx.init_primary_pubkey = Some(derived),
            Some(provided) if provided != &derived => {
                bail!(
                    "Quantum bootstrap init_primary_pubkey does not match the public key derived from the signing seed"
                );
            }
            Some(_) => {}
        }
    }

    let request = QuantumWriteRequestV1::from_quantum_transaction_request(&tx)?;
    sign_quantum_write_request_with_cosigner(request, primary_seed, cosigner)
}

pub fn sign_quantum_write_request(
    request: QuantumWriteRequestV1,
    primary_seed: [u8; ML_DSA_SEED_BYTES],
) -> Result<QuantumSignedPayload> {
    sign_quantum_write_request_with_cosigner(request, primary_seed, None)
}

pub fn sign_quantum_write_request_with_cosigner(
    request: QuantumWriteRequestV1,
    primary_seed: [u8; ML_DSA_SEED_BYTES],
    cosigner: Option<DetachedCosigner>,
) -> Result<QuantumSignedPayload> {
    validate_quantum_write_request(&request)?;

    let signing_hash = request.signature_hash();

    if let Some(ref cosigner) = cosigner
        && cosigner.signing_hash != signing_hash
    {
        bail!(
            "Quantum detached cosigner signing_hash does not match the request signing hash; refusing to attach cosigner"
        );
    }

    let seed = ml_dsa::B32::from(primary_seed);
    let keypair = <MlDsa44 as KeyGen>::from_seed(&seed);
    let ml_sig = keypair
        .signing_key()
        .sign_deterministic(signing_hash.as_ref(), &[])
        .map_err(|err| eyre::eyre!("failed to sign Quantum transaction: {err}"))?;
    let encoded_sig = ml_sig.encode();
    let encoded_sig: &[u8] = encoded_sig.as_ref();
    let primary = SigningKeySignature::MlDsa44 {
        signature: Box::new(
            <[u8; ML_DSA_SIGNATURE_BYTES]>::try_from(encoded_sig)
                .expect("ML-DSA signature length is fixed"),
        ),
    };

    let cosigner_sig = cosigner.map(DetachedCosigner::into_signing_key_signature);

    let signed = QuantumSigned {
        tx: request.clone(),
        sender_sig: CompositeSignature { primary, cosigner: cosigner_sig },
    };

    let mut raw_transaction = Vec::with_capacity(signed.encode_2718_len());
    signed.encode_2718(&mut raw_transaction);

    Ok(QuantumSignedPayload {
        raw_transaction,
        signing_hash,
        sender: request.sender,
        key_id: request.key_id,
    })
}

#[cfg(test)]
fn derive_address_from_seed(seed: [u8; ML_DSA_SEED_BYTES]) -> Address {
    let pubkey = derive_primary_pubkey(seed);
    debug_assert_eq!(pubkey.len(), ML_DSA_PUBLIC_KEY_BYTES);
    Address::from_slice(&keccak256(pubkey)[12..])
}

pub fn make_phase0_fixture(payload: &QuantumSignedPayload) -> QuantumPhase0RawFixture {
    QuantumPhase0RawFixture {
        tx_type: format!("0x{QUANTUM_TX_TYPE_ID:02x}"),
        sender: format!("{:#x}", payload.sender),
        key_id: payload.key_id,
        nonce_key: "0x0".to_string(),
        chain_id: 1337,
        signing_hash: format!("{:#x}", payload.signing_hash),
        raw_transaction: format!("0x{}", alloy_primitives::hex::encode(&payload.raw_transaction)),
        raw_transaction_hash: format!("{:#x}", keccak256(&payload.raw_transaction)),
        foundry_base_commit: PHASE0_FOUNDY_BASE_COMMIT.to_string(),
        quantum_harness_commit: PHASE0_QUANTUM_HARNESS_COMMIT.to_string(),
    }
}

pub fn quantum_transaction_request_is_bootstrap(tx: &QuantumTransactionRequest) -> bool {
    matches!(TransactionBuilder::kind(tx), Some(TxKind::Call(to)) if to == QUANTUM_KEYVAULT_ADDRESS)
        && TransactionBuilder::input(tx).is_some_and(|input| quantum_is_bootstrap_calldata(input))
}

pub fn quantum_is_bootstrap_calldata(input: &[u8]) -> bool {
    input.starts_with(&QUANTUM_BOOTSTRAP_SELECTOR)
}

pub fn quantum_is_unsupported_lifecycle_calldata(input: &[u8]) -> bool {
    if input.len() < 4 {
        return false;
    }
    let selector = [input[0], input[1], input[2], input[3]];
    QUANTUM_SEND_UNSUPPORTED_LIFECYCLE_SELECTORS.contains(&selector)
}

fn validate_quantum_write_request(request: &QuantumWriteRequestV1) -> Result<()> {
    request.validate_v1()
}

impl QuantumSigned {
    fn rlp_inner_length(&self) -> usize {
        self.tx.encoded_fields_length()
            + self.sender_sig.length()
            + option_as_list_length::<CompositeSignature>(None)
    }

    fn rlp_encode_inner(&self, out: &mut dyn BufMut) {
        self.tx.encode_fields(out);
        self.sender_sig.encode(out);
        encode_option_as_list::<CompositeSignature>(None, out);
    }

    fn encode_2718_len(&self) -> usize {
        let inner_len = self.rlp_inner_length();
        1 + alloy_rlp::length_of_length(inner_len) + inner_len
    }

    fn encode_2718(&self, out: &mut dyn BufMut) {
        out.put_u8(QUANTUM_TX_TYPE_ID);
        let inner_len = self.rlp_inner_length();
        RlpHeader { list: true, payload_length: inner_len }.encode(out);
        self.rlp_encode_inner(out);
    }
}

impl SigningKeySignature {
    const fn wire_size(&self) -> usize {
        match self {
            Self::MlDsa44 { .. } => 1 + ML_DSA_SIGNATURE_BYTES,
            Self::P256 { .. } | Self::Ecdsa { .. } => 1 + DETACHED_CLASSICAL_SIGNATURE_BYTES,
        }
    }
}

impl Encodable for SigningKeySignature {
    fn encode(&self, out: &mut dyn BufMut) {
        let mut bytes = Vec::with_capacity(self.wire_size());
        match self {
            Self::MlDsa44 { signature } => {
                bytes.push(QUANTUM_ML_DSA_SCHEME);
                bytes.extend_from_slice(signature.as_ref());
            }
            Self::P256 { signature } => {
                bytes.push(QUANTUM_P256_SCHEME);
                bytes.extend_from_slice(signature.as_ref());
            }
            Self::Ecdsa { signature } => {
                bytes.push(QUANTUM_ECDSA_SCHEME);
                bytes.extend_from_slice(signature.as_ref());
            }
        }
        bytes.as_slice().encode(out);
    }

    fn length(&self) -> usize {
        let wire = self.wire_size();
        wire + alloy_rlp::length_of_length(wire)
    }
}

impl Encodable for CompositeSignature {
    fn encode(&self, out: &mut dyn BufMut) {
        let payload_length = self.primary.length() + option_as_list_length(self.cosigner.as_ref());
        RlpHeader { list: true, payload_length }.encode(out);
        self.primary.encode(out);
        encode_option_as_list(self.cosigner.as_ref(), out);
    }

    fn length(&self) -> usize {
        let payload_length = self.primary.length() + option_as_list_length(self.cosigner.as_ref());
        alloy_rlp::length_of_length(payload_length) + payload_length
    }
}

fn encode_option_as_list<T: Encodable>(value: Option<&T>, out: &mut dyn BufMut) {
    match value {
        Some(value) => {
            let payload_length = value.length();
            RlpHeader { list: true, payload_length }.encode(out);
            value.encode(out);
        }
        None => {
            RlpHeader { list: true, payload_length: 0 }.encode(out);
        }
    }
}

fn option_as_list_length<T: Encodable>(value: Option<&T>) -> usize {
    match value {
        Some(value) => {
            let payload_length = value.length();
            alloy_rlp::length_of_length(payload_length) + payload_length
        }
        None => 1,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use alloy_primitives::{U256, b256};
    use serde_json::Value;

    use super::*;
    use crate::{QuantumBootstrapFieldsV1, QuantumSingleCall};

    fn fixture_seed_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/fixtures/quantum/phase0/primary-seed.hex")
    }

    fn raw_fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/fixtures/quantum/phase0/raw-send-primary.json")
    }

    fn canonical_phase0_fixture() -> QuantumPhase0RawFixture {
        let seed = parse_seed_file(&fixture_seed_path()).unwrap();
        let payload = sign_quantum_write_request(
            QuantumWriteRequestV1 {
                sender: derive_address_from_seed(seed),
                key_id: 0,
                nonce: 0,
                chain_id: 1337,
                max_priority_fee_per_gas: 1_000_000_000,
                max_fee_per_gas: 20_000_000_000,
                gas_limit: 21_000,
                call: QuantumSingleCall {
                    kind: TxKind::Call(Address::repeat_byte(0x33)),
                    value: U256::from(1u64),
                    input: Bytes::new(),
                },
                access_list: Default::default(),
                bootstrap: None,
            },
            seed,
        )
        .unwrap();

        make_phase0_fixture(&payload)
    }

    #[test]
    fn quantum_shared_signer_accepts_lifecycle_selectors_for_cast_quantum_path() {
        // The shared signer must sign KeyVault lifecycle selectors — those are the
        // legitimate payload of `cast quantum add-key / remove-key / update-key-auth`.
        // CLI-surface policy (rejecting lifecycle selectors on `cast send`) is enforced
        // at the `cast send` pre-build guard, not at the shared write-request validator.
        let seed = parse_seed_file(&fixture_seed_path()).unwrap();
        let payload = sign_quantum_write_request(
            QuantumWriteRequestV1 {
                sender: Address::repeat_byte(0x22),
                key_id: 0,
                nonce: 0,
                chain_id: 1337,
                max_priority_fee_per_gas: 1,
                max_fee_per_gas: 1,
                gas_limit: 21_000,
                call: QuantumSingleCall {
                    kind: TxKind::Call(QUANTUM_KEYVAULT_ADDRESS),
                    value: U256::ZERO,
                    input: Bytes::from(QUANTUM_ADD_KEY_SELECTOR.to_vec()),
                },
                access_list: Default::default(),
                bootstrap: None,
            },
            seed,
        )
        .unwrap();

        assert_eq!(payload.sender, Address::repeat_byte(0x22));
        assert_eq!(&payload.raw_transaction[0..1], &[QUANTUM_TX_TYPE_ID]);
    }

    #[test]
    fn quantum_transaction_request_bootstrap_derives_primary_pubkey() {
        let seed = parse_seed_file(&fixture_seed_path()).unwrap();
        let request = QuantumTransactionRequest {
            inner: alloy_rpc_types::TransactionRequest::default()
                .with_chain_id(1337)
                .with_nonce(0)
                .with_to(QUANTUM_KEYVAULT_ADDRESS)
                .with_gas_limit(21_000)
                .with_max_fee_per_gas(1)
                .with_max_priority_fee_per_gas(1)
                .with_input(Bytes::from(QUANTUM_BOOTSTRAP_SELECTOR.to_vec())),
            sender: Some(Address::repeat_byte(0x22)),
            key_id: Some(0),
            nonce_key: Some(U256::ZERO),
            init_primary_pubkey: None,
            init_cosigner_pubkey: None,
        };

        let payload = sign_quantum_transaction_request(request, seed).unwrap();
        assert_eq!(payload.sender, Address::repeat_byte(0x22));
    }

    #[test]
    fn quantum_transaction_request_bootstrap_rejects_mismatched_primary_pubkey() {
        let seed = parse_seed_file(&fixture_seed_path()).unwrap();
        let request = QuantumTransactionRequest {
            inner: alloy_rpc_types::TransactionRequest::default()
                .with_chain_id(1337)
                .with_nonce(0)
                .with_to(QUANTUM_KEYVAULT_ADDRESS)
                .with_gas_limit(21_000)
                .with_max_fee_per_gas(1)
                .with_max_priority_fee_per_gas(1)
                .with_input(Bytes::from(QUANTUM_BOOTSTRAP_SELECTOR.to_vec())),
            sender: Some(Address::repeat_byte(0x22)),
            key_id: Some(0),
            nonce_key: Some(U256::ZERO),
            init_primary_pubkey: Some(Bytes::from_static(&[0xAAu8; 32])),
            init_cosigner_pubkey: None,
        };

        let err = sign_quantum_transaction_request(request, seed).unwrap_err();
        assert!(err.to_string().contains("init_primary_pubkey"), "unexpected error: {err}");
    }

    #[test]
    fn generated_fixture_is_valid_json_shape() {
        let fixture = canonical_phase0_fixture();
        let value: Value = serde_json::to_value(fixture).unwrap();
        assert_eq!(value["tx_type"], "0x7a");
        assert_eq!(value["key_id"], 0);
        assert_eq!(value["nonce_key"], "0x0");
        assert!(value["raw_transaction"].as_str().unwrap().starts_with("0x7a"));
    }

    #[test]
    fn generated_fixture_matches_checked_in_phase0_example() {
        let expected = canonical_phase0_fixture();
        let actual: QuantumPhase0RawFixture =
            serde_json::from_str(&fs::read_to_string(raw_fixture_path()).unwrap()).unwrap();

        assert_eq!(actual, expected);
    }

    fn simple_transfer_request(sender: Address) -> QuantumWriteRequestV1 {
        QuantumWriteRequestV1 {
            sender,
            key_id: 0,
            nonce: 0,
            chain_id: 1337,
            max_priority_fee_per_gas: 1,
            max_fee_per_gas: 1,
            gas_limit: 21_000,
            call: QuantumSingleCall {
                kind: TxKind::Call(Address::repeat_byte(0x44)),
                value: U256::from(1u64),
                input: Bytes::new(),
            },
            access_list: Default::default(),
            bootstrap: None,
        }
    }

    fn artifact_for(scheme: &str, signing_hash: B256) -> DetachedArtifactV1 {
        DetachedArtifactV1 {
            version: QUANTUM_DETACHED_ARTIFACT_VERSION,
            scheme: scheme.to_string(),
            signing_hash: format!("{signing_hash:#x}"),
            public_key: format!("0x{}", alloy_primitives::hex::encode([0x11u8; 33])),
            signature: format!(
                "0x{}",
                alloy_primitives::hex::encode([0x22u8; DETACHED_CLASSICAL_SIGNATURE_BYTES])
            ),
        }
    }

    #[test]
    fn detached_artifact_rejects_unknown_version() {
        let mut artifact = artifact_for(QUANTUM_DETACHED_SCHEME_P256, B256::ZERO);
        artifact.version = 2;
        let err = DetachedCosigner::from_artifact(artifact).unwrap_err();
        assert!(err.to_string().contains("unsupported Quantum detached artifact version"));
    }

    #[test]
    fn detached_artifact_rejects_unknown_scheme() {
        let artifact = artifact_for("rsa", B256::ZERO);
        let err = DetachedCosigner::from_artifact(artifact).unwrap_err();
        assert!(err.to_string().contains("unsupported Quantum detached cosigner scheme"));
    }

    #[test]
    fn detached_artifact_rejects_wrong_signature_length() {
        let mut artifact = artifact_for(QUANTUM_DETACHED_SCHEME_P256, B256::ZERO);
        artifact.signature = "0x1234".to_string();
        let err = DetachedCosigner::from_artifact(artifact).unwrap_err();
        assert!(err.to_string().contains("signature must be"));
    }

    #[test]
    fn detached_cosigner_attach_fails_when_signing_hash_mismatches() {
        let seed = parse_seed_file(&fixture_seed_path()).unwrap();
        let request = simple_transfer_request(derive_address_from_seed(seed));
        let wrong_hash = B256::repeat_byte(0xaa);
        let cosigner =
            DetachedCosigner::from_artifact(artifact_for(QUANTUM_DETACHED_SCHEME_P256, wrong_hash))
                .unwrap();

        let err =
            sign_quantum_write_request_with_cosigner(request, seed, Some(cosigner)).unwrap_err();
        assert!(err.to_string().contains("signing_hash does not match"));
    }

    #[test]
    fn detached_p256_cosigner_is_attached_to_composite_signature() {
        let seed = parse_seed_file(&fixture_seed_path()).unwrap();
        let sender = derive_address_from_seed(seed);
        let request = simple_transfer_request(sender);
        let signing_hash = request.signature_hash();

        let cosigner = DetachedCosigner::from_artifact(artifact_for(
            QUANTUM_DETACHED_SCHEME_P256,
            signing_hash,
        ))
        .unwrap();
        let signed =
            sign_quantum_write_request_with_cosigner(request.clone(), seed, Some(cosigner.clone()))
                .unwrap();

        let raw = &signed.raw_transaction;
        assert_eq!(raw[0], QUANTUM_TX_TYPE_ID);
        assert!(raw
            .windows(1 + DETACHED_CLASSICAL_SIGNATURE_BYTES)
            .any(|window| window[0] == QUANTUM_P256_SCHEME
                && window[1..] == cosigner.signature));

        let primary_only = sign_quantum_write_request(request, seed).unwrap();
        assert!(signed.raw_transaction.len() > primary_only.raw_transaction.len());

        // Pinned golden values protect the composite RLP envelope against drift:
        // any change to field ordering, scheme-byte placement, or cosigner layout
        // flips these constants and fails the regression.
        assert_eq!(signed.raw_transaction.len(), 2563);
        assert_eq!(primary_only.raw_transaction.len(), 2495);
        assert_eq!(
            keccak256(&signed.raw_transaction),
            b256!("8c6ef4e59a3ea673f21c2c7e87e1f02337a77d3aedea6a09862244da7034149a"),
        );
    }

    #[test]
    fn detached_ecdsa_cosigner_uses_ecdsa_scheme_byte() {
        let seed = parse_seed_file(&fixture_seed_path()).unwrap();
        let sender = derive_address_from_seed(seed);
        let request = simple_transfer_request(sender);
        let signing_hash = request.signature_hash();

        let cosigner = DetachedCosigner::from_artifact(artifact_for(
            QUANTUM_DETACHED_SCHEME_ECDSA,
            signing_hash,
        ))
        .unwrap();
        let signed =
            sign_quantum_write_request_with_cosigner(request, seed, Some(cosigner.clone()))
                .unwrap();

        assert!(signed.raw_transaction.windows(1 + DETACHED_CLASSICAL_SIGNATURE_BYTES).any(
            |window| { window[0] == QUANTUM_ECDSA_SCHEME && window[1..] == cosigner.signature }
        ));
    }

    #[test]
    fn bootstrap_fixture_still_validates_v1_rules() {
        let bootstrap = QuantumWriteRequestV1 {
            sender: Address::repeat_byte(0x22),
            key_id: 0,
            nonce: 0,
            chain_id: 1337,
            max_priority_fee_per_gas: 1,
            max_fee_per_gas: 1,
            gas_limit: 21_000,
            call: QuantumSingleCall {
                kind: TxKind::Call(QUANTUM_KEYVAULT_ADDRESS),
                value: U256::ZERO,
                input: Bytes::from(QUANTUM_BOOTSTRAP_SELECTOR.to_vec()),
            },
            access_list: Default::default(),
            bootstrap: Some(QuantumBootstrapFieldsV1 {
                init_primary_pubkey: Bytes::from(vec![1, 2, 3]),
                init_cosigner_pubkey: None,
            }),
        };

        assert!(bootstrap.validate_v1().is_ok());
    }
}
