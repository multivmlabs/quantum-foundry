use std::{fs, path::Path};

use alloy_eips::eip2930::AccessList;
use alloy_primitives::{Address, B256, Bytes, TxKind, U256, address, keccak256};
use alloy_rlp::{BufMut, Encodable, Header as RlpHeader};
use eyre::{Result, bail, ensure};
use ml_dsa::{KeyGen, MlDsa44};
use serde::{Deserialize, Serialize};

#[cfg(test)]
use ml_dsa::signature::Keypair;

pub const QUANTUM_TX_TYPE_ID: u8 = 0x7A;
pub const QUANTUM_ML_DSA_SCHEME: u8 = 0x01;
pub const ML_DSA_PUBLIC_KEY_BYTES: usize = 1312;
pub const ML_DSA_SIGNATURE_BYTES: usize = 2420;
pub const ML_DSA_SEED_BYTES: usize = 32;
pub const PHASE0_FOUNDY_BASE_COMMIT: &str = "f1abb2ca347187bb6dea8c3881ca44ce50aab1e7";
pub const PHASE0_QUANTUM_HARNESS_COMMIT: &str = "8f3612c60f9fa66ea3a09eab99a2e0802f373673";
pub const PHASE0_TX_SPAMMER_EVIDENCE_COMMIT: &str = "2c25f14a44b8cc88fc41a65f521f1ba8350e7fa4";
pub const KEYVAULT_ADDRESS: Address = address!("0000000000000000000000000000000000001000");
pub const LIFECYCLE_REJECTION_MESSAGE: &str = "KeyVault lifecycle operations (bootstrap/addKey/removeKey/updateKeyAuth) cannot be simulated via eth_call; use explicit lifecycle transaction submission";
pub const LIFECYCLE_SELECTORS: [[u8; 4]; 4] = [
    [0x5e, 0x8e, 0x7a, 0x13],
    [0x32, 0xbc, 0x29, 0x19],
    [0xc9, 0x8f, 0x21, 0xf4],
    [0x89, 0x08, 0x15, 0x4b],
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuantumWriteContractV1 {
    pub sender: Address,
    pub key_id: u32,
    pub nonce: u64,
    pub chain_id: u64,
    pub max_priority_fee_per_gas: u128,
    pub max_fee_per_gas: u128,
    pub gas_limit: u64,
    pub kind: TxKind,
    pub value: U256,
    pub input: Bytes,
    pub access_list: AccessList,
    pub primary_seed: [u8; ML_DSA_SEED_BYTES],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetachedArtifactV1 {
    pub version: u8,
    pub scheme: String,
    pub signing_hash: String,
    pub public_key: String,
    pub signature: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuantumSignedPayload {
    pub raw_transaction: Vec<u8>,
    pub signing_hash: B256,
    pub sender: Address,
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
struct QuantumCall {
    to: TxKind,
    value: U256,
    input: Bytes,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct QuantumTransaction {
    chain_id: u64,
    sender: Address,
    nonce_key: U256,
    nonce: u64,
    key_id: u32,
    max_priority_fee_per_gas: u128,
    max_fee_per_gas: u128,
    gas_limit: u64,
    calls: Vec<QuantumCall>,
    access_list: AccessList,
    fee_payer: Option<Address>,
    fee_payer_key_id: Option<u32>,
    init_primary_pubkey: Option<Bytes>,
    init_cosigner_pubkey: Option<Bytes>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CompositeSignature {
    primary: SigningKeySignature,
    cosigner: Option<SigningKeySignature>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SigningKeySignature {
    MlDsa44 {
        signature: Box<[u8; ML_DSA_SIGNATURE_BYTES]>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct QuantumSigned {
    tx: QuantumTransaction,
    sender_sig: CompositeSignature,
}

pub fn parse_seed_file(path: &Path) -> Result<[u8; ML_DSA_SEED_BYTES]> {
    let contents = fs::read_to_string(path)?;
    let trimmed = contents.trim();
    let hex = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .unwrap_or(trimmed);
    let bytes = alloy_primitives::hex::decode(hex)?;
    let seed = <[u8; ML_DSA_SEED_BYTES]>::try_from(bytes.as_slice()).map_err(|_| {
        eyre::eyre!(
            "Quantum ML-DSA seed file must contain exactly {ML_DSA_SEED_BYTES} bytes of hex"
        )
    })?;
    Ok(seed)
}

pub fn build_phase0_payload(contract: QuantumWriteContractV1) -> Result<QuantumSignedPayload> {
    contract.validate_phase0()?;

    let tx = QuantumTransaction {
        chain_id: contract.chain_id,
        sender: contract.sender,
        nonce_key: U256::ZERO,
        nonce: contract.nonce,
        key_id: contract.key_id,
        max_priority_fee_per_gas: contract.max_priority_fee_per_gas,
        max_fee_per_gas: contract.max_fee_per_gas,
        gas_limit: contract.gas_limit,
        calls: vec![QuantumCall {
            to: contract.kind,
            value: contract.value,
            input: contract.input,
        }],
        access_list: contract.access_list,
        fee_payer: None,
        fee_payer_key_id: None,
        init_primary_pubkey: None,
        init_cosigner_pubkey: None,
    };

    let signing_hash = tx.signature_hash();
    let seed = ml_dsa::B32::from(contract.primary_seed);
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

    let signed = QuantumSigned {
        tx,
        sender_sig: CompositeSignature {
            primary,
            cosigner: None,
        },
    };

    let mut raw_transaction = Vec::with_capacity(signed.encode_2718_len());
    signed.encode_2718(&mut raw_transaction);

    Ok(QuantumSignedPayload {
        raw_transaction,
        signing_hash,
        sender: contract.sender,
    })
}

#[cfg(test)]
fn derive_address_from_seed(seed: [u8; ML_DSA_SEED_BYTES]) -> Address {
    let keypair = <MlDsa44 as KeyGen>::from_seed(&ml_dsa::B32::from(seed));
    let verifying_key = keypair.verifying_key().encode();
    let verifying_key: &[u8] = verifying_key.as_ref();
    debug_assert_eq!(verifying_key.len(), ML_DSA_PUBLIC_KEY_BYTES);
    Address::from_slice(&keccak256(verifying_key)[12..])
}

pub fn make_phase0_fixture(payload: &QuantumSignedPayload, key_id: u32) -> QuantumPhase0RawFixture {
    QuantumPhase0RawFixture {
        tx_type: format!("0x{QUANTUM_TX_TYPE_ID:02x}"),
        sender: format!("{:#x}", payload.sender),
        key_id,
        nonce_key: "0x0".to_string(),
        chain_id: 1337,
        signing_hash: format!("{:#x}", payload.signing_hash),
        raw_transaction: format!("0x{}", alloy_primitives::hex::encode(&payload.raw_transaction)),
        raw_transaction_hash: format!("{:#x}", keccak256(&payload.raw_transaction)),
        foundry_base_commit: PHASE0_FOUNDY_BASE_COMMIT.to_string(),
        quantum_harness_commit: PHASE0_QUANTUM_HARNESS_COMMIT.to_string(),
    }
}

impl QuantumWriteContractV1 {
    fn validate_phase0(&self) -> Result<()> {
        ensure!(self.sender != Address::ZERO, "Quantum sender must be explicit and non-zero");
        ensure!(self.key_id == 0, "Phase 0 only supports key_id = 0");
        ensure!(matches!(self.kind, TxKind::Call(_)), "Phase 0 seam only supports single-call send flows");
        ensure!(self.max_priority_fee_per_gas <= self.max_fee_per_gas, "max priority fee must not exceed max fee");
        ensure!(self.access_list.is_empty(), "Phase 0 seam does not support access lists");

        if let TxKind::Call(to) = self.kind
            && to == KEYVAULT_ADDRESS
            && is_lifecycle_selector(&self.input)
        {
            bail!(LIFECYCLE_REJECTION_MESSAGE);
        }

        Ok(())
    }
}

fn is_lifecycle_selector(input: &[u8]) -> bool {
    if input.len() < 4 {
        return false;
    }
    let selector = [input[0], input[1], input[2], input[3]];
    LIFECYCLE_SELECTORS.contains(&selector)
}

impl QuantumTransaction {
    fn rlp_encode_fields(&self, out: &mut dyn BufMut) {
        self.chain_id.encode(out);
        self.sender.encode(out);
        self.nonce_key.encode(out);
        self.nonce.encode(out);
        self.key_id.encode(out);
        self.max_priority_fee_per_gas.encode(out);
        self.max_fee_per_gas.encode(out);
        self.gas_limit.encode(out);
        self.calls.encode(out);
        self.access_list.encode(out);
        encode_option_as_list(self.fee_payer.as_ref(), out);
        encode_option_as_list(self.fee_payer_key_id.as_ref(), out);
        encode_option_as_list(self.init_primary_pubkey.as_ref(), out);
        encode_option_as_list(self.init_cosigner_pubkey.as_ref(), out);
    }

    fn rlp_encoded_fields_length(&self) -> usize {
        self.chain_id.length()
            + self.sender.length()
            + self.nonce_key.length()
            + self.nonce.length()
            + self.key_id.length()
            + self.max_priority_fee_per_gas.length()
            + self.max_fee_per_gas.length()
            + self.gas_limit.length()
            + self.calls.length()
            + self.access_list.length()
            + option_as_list_length(self.fee_payer.as_ref())
            + option_as_list_length(self.fee_payer_key_id.as_ref())
            + option_as_list_length(self.init_primary_pubkey.as_ref())
            + option_as_list_length(self.init_cosigner_pubkey.as_ref())
    }

    fn encode_for_signing(&self, out: &mut dyn BufMut) {
        out.put_u8(QUANTUM_TX_TYPE_ID);
        let payload_len = self.rlp_encoded_fields_length();
        RlpHeader {
            list: true,
            payload_length: payload_len,
        }
        .encode(out);
        self.rlp_encode_fields(out);
    }

    fn signature_hash(&self) -> B256 {
        let mut buf = Vec::new();
        self.encode_for_signing(&mut buf);
        keccak256(buf)
    }
}

impl Encodable for QuantumTransaction {
    fn encode(&self, out: &mut dyn BufMut) {
        let payload_length = self.rlp_encoded_fields_length();
        RlpHeader {
            list: true,
            payload_length,
        }
        .encode(out);
        self.rlp_encode_fields(out);
    }

    fn length(&self) -> usize {
        let payload_length = self.rlp_encoded_fields_length();
        alloy_rlp::length_of_length(payload_length) + payload_length
    }
}

impl QuantumSigned {
    fn rlp_inner_length(&self) -> usize {
        self.tx.rlp_encoded_fields_length()
            + self.sender_sig.length()
            + option_as_list_length::<CompositeSignature>(None)
    }

    fn rlp_encode_inner(&self, out: &mut dyn BufMut) {
        self.tx.rlp_encode_fields(out);
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
        RlpHeader {
            list: true,
            payload_length: inner_len,
        }
        .encode(out);
        self.rlp_encode_inner(out);
    }
}

impl Encodable for QuantumCall {
    fn encode(&self, out: &mut dyn BufMut) {
        let payload_len = self.to.length() + self.value.length() + self.input.length();
        RlpHeader {
            list: true,
            payload_length: payload_len,
        }
        .encode(out);
        self.to.encode(out);
        self.value.encode(out);
        self.input.encode(out);
    }

    fn length(&self) -> usize {
        let payload_len = self.to.length() + self.value.length() + self.input.length();
        alloy_rlp::length_of_length(payload_len) + payload_len
    }
}

impl SigningKeySignature {
    fn wire_size(&self) -> usize {
        match self {
            Self::MlDsa44 { .. } => 1 + ML_DSA_SIGNATURE_BYTES,
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
        RlpHeader {
            list: true,
            payload_length,
        }
        .encode(out);
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
            RlpHeader {
                list: true,
                payload_length,
            }
            .encode(out);
            value.encode(out);
        }
        None => {
            RlpHeader {
                list: true,
                payload_length: 0,
            }
            .encode(out);
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

    use serde_json::Value;

    use super::*;

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
        let payload = build_phase0_payload(QuantumWriteContractV1 {
            sender: derive_address_from_seed(seed),
            key_id: 0,
            nonce: 0,
            chain_id: 1337,
            max_priority_fee_per_gas: 1_000_000_000,
            max_fee_per_gas: 20_000_000_000,
            gas_limit: 21_000,
            kind: TxKind::Call(Address::repeat_byte(0x33)),
            value: U256::from(1u64),
            input: Bytes::new(),
            access_list: AccessList::default(),
            primary_seed: seed,
        })
        .unwrap();

        make_phase0_fixture(&payload, 0)
    }

    #[test]
    fn upstream_minimal_body_hashes_match_source_of_truth() {
        let tx = QuantumTransaction {
            chain_id: 1337,
            sender: Address::ZERO,
            nonce_key: U256::ZERO,
            nonce: 42,
            key_id: 0,
            max_priority_fee_per_gas: 1_000_000_000,
            max_fee_per_gas: 50_000_000_000,
            gas_limit: 21_000,
            calls: vec![QuantumCall {
                to: TxKind::Call(Address::ZERO),
                value: U256::from(1_000_000_000_000_000_000u128),
                input: Bytes::from_static(b"hello"),
            }],
            access_list: AccessList::default(),
            fee_payer: None,
            fee_payer_key_id: None,
            init_primary_pubkey: None,
            init_cosigner_pubkey: None,
        };

        let mut buf = Vec::new();
        tx.encode(&mut buf);
        assert_eq!(
            format!("{:#x}", keccak256(&buf)),
            "0xd039fbca7d51e653c90e8b84adb8fa6e30929dffa7eb41dbd6ec40594ce3ad4e"
        );
        assert_eq!(
            format!("{:#x}", tx.signature_hash()),
            "0x909fe4db64c4605eb394b9de4d064bce0ab6d718b32050f00d80d7f525753b7d"
        );
    }

    #[test]
    fn phase0_payload_enforces_explicit_sender() {
        let seed = parse_seed_file(&fixture_seed_path()).unwrap();
        let err = build_phase0_payload(QuantumWriteContractV1 {
            sender: Address::ZERO,
            key_id: 0,
            nonce: 0,
            chain_id: 1337,
            max_priority_fee_per_gas: 1,
            max_fee_per_gas: 1,
            gas_limit: 21_000,
            kind: TxKind::Call(Address::repeat_byte(0x11)),
            value: U256::ZERO,
            input: Bytes::new(),
            access_list: AccessList::default(),
            primary_seed: seed,
        })
        .unwrap_err();

        assert!(err.to_string().contains("explicit and non-zero"));
    }

    #[test]
    fn phase0_payload_rejects_lifecycle_selectors() {
        let seed = parse_seed_file(&fixture_seed_path()).unwrap();
        let err = build_phase0_payload(QuantumWriteContractV1 {
            sender: Address::repeat_byte(0x22),
            key_id: 0,
            nonce: 0,
            chain_id: 1337,
            max_priority_fee_per_gas: 1,
            max_fee_per_gas: 1,
            gas_limit: 21_000,
            kind: TxKind::Call(KEYVAULT_ADDRESS),
            value: U256::ZERO,
            input: Bytes::from(LIFECYCLE_SELECTORS[0].to_vec()),
            access_list: AccessList::default(),
            primary_seed: seed,
        })
        .unwrap_err();

        assert_eq!(err.to_string(), LIFECYCLE_REJECTION_MESSAGE);
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
}
