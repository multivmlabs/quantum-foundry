use std::sync::OnceLock;

use alloy_consensus::{
    Receipt, ReceiptWithBloom, Transaction, TransactionEnvelope, TxReceipt,
    transaction::{SignerRecoverable, TxHashRef},
};
use alloy_eips::{
    Typed2718,
    eip2718::{Decodable2718, Eip2718Error, Encodable2718, IsTyped2718},
    eip2930::AccessList,
    eip7702::SignedAuthorization,
};
use alloy_network::{BuildResult, Network, NetworkTransactionBuilder, TransactionBuilder};
use alloy_primitives::{Address, B256, Bytes, ChainId, TxHash, TxKind, U256, keccak256};
use alloy_provider::fillers::{
    ChainIdFiller, GasFiller, JoinFill, NonceFiller, RecommendedFillers,
};
use alloy_rlp::{BufMut, Decodable, Encodable, Header as RlpHeader};
use alloy_rpc_types::Log;
use alloy_rpc_types_eth::{
    Block, Header, Transaction as RpcTransaction, TransactionReceipt, TransactionRequest,
};
use serde::{Deserialize, Serialize};

pub const QUANTUM_TX_TYPE_ID: u8 = 0x7A;
const QUANTUM_RECEIPT_TYPE_LABEL: &str = "Pq";

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[repr(u8)]
pub enum QuantumTxType {
    #[default]
    Pq = QUANTUM_TX_TYPE_ID,
}

impl core::fmt::Display for QuantumTxType {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "0x{QUANTUM_TX_TYPE_ID:02x}")
    }
}

impl Typed2718 for QuantumTxType {
    fn ty(&self) -> u8 {
        *self as u8
    }
}

impl From<QuantumTxType> for u8 {
    fn from(value: QuantumTxType) -> Self {
        value as Self
    }
}

impl TryFrom<u8> for QuantumTxType {
    type Error = Eip2718Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            QUANTUM_TX_TYPE_ID => Ok(Self::Pq),
            other => Err(Eip2718Error::UnexpectedType(other)),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuantumTransactionRequest {
    #[serde(flatten)]
    pub inner: TransactionRequest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender: Option<Address>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "alloy_serde::quantity::opt")]
    pub key_id: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nonce_key: Option<U256>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub init_primary_pubkey: Option<Bytes>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub init_cosigner_pubkey: Option<Bytes>,
}

impl From<TransactionRequest> for QuantumTransactionRequest {
    fn from(value: TransactionRequest) -> Self {
        Self { inner: value, ..Default::default() }
    }
}

impl AsRef<TransactionRequest> for QuantumTransactionRequest {
    fn as_ref(&self) -> &TransactionRequest {
        &self.inner
    }
}

impl AsMut<TransactionRequest> for QuantumTransactionRequest {
    fn as_mut(&mut self) -> &mut TransactionRequest {
        &mut self.inner
    }
}

impl From<QuantumTxEnvelope> for QuantumTransactionRequest {
    fn from(value: QuantumTxEnvelope) -> Self {
        Self {
            inner: TransactionRequest::default()
                .with_chain_id(value.chain_id)
                .with_nonce(value.nonce)
                .with_gas_limit(value.gas_limit)
                .with_max_fee_per_gas(value.max_fee_per_gas)
                .with_max_priority_fee_per_gas(value.max_priority_fee_per_gas)
                .with_kind(value.call.kind)
                .with_value(value.call.value)
                .with_input(value.call.input.clone())
                .with_access_list(value.access_list.clone()),
            sender: Some(value.sender),
            key_id: Some(value.key_id),
            nonce_key: Some(value.nonce_key),
            init_primary_pubkey: value.init_primary_pubkey,
            init_cosigner_pubkey: value.init_cosigner_pubkey,
        }
    }
}

impl From<RpcTransaction<QuantumTxEnvelope>> for QuantumTransactionRequest {
    fn from(tx: RpcTransaction<QuantumTxEnvelope>) -> Self {
        tx.inner.into_inner().into()
    }
}

macro_rules! eth_tb {
    ($method:ident, $self:expr $(, $arg:expr)*) => {
        <TransactionRequest as alloy_network::TransactionBuilder>::$method($self $(, $arg)*)
    };
}

impl alloy_network::TransactionBuilder for QuantumTransactionRequest {
    fn chain_id(&self) -> Option<ChainId> {
        eth_tb!(chain_id, &self.inner)
    }

    fn set_chain_id(&mut self, chain_id: ChainId) {
        eth_tb!(set_chain_id, &mut self.inner, chain_id)
    }

    fn nonce(&self) -> Option<u64> {
        eth_tb!(nonce, &self.inner)
    }

    fn set_nonce(&mut self, nonce: u64) {
        eth_tb!(set_nonce, &mut self.inner, nonce)
    }

    fn take_nonce(&mut self) -> Option<u64> {
        eth_tb!(take_nonce, &mut self.inner)
    }

    fn input(&self) -> Option<&Bytes> {
        eth_tb!(input, &self.inner)
    }

    fn set_input<T: Into<Bytes>>(&mut self, input: T) {
        eth_tb!(set_input, &mut self.inner, input)
    }

    fn set_input_kind<T: Into<Bytes>>(
        &mut self,
        input: T,
        kind: alloy_rpc_types::TransactionInputKind,
    ) {
        eth_tb!(set_input_kind, &mut self.inner, input, kind)
    }

    fn from(&self) -> Option<Address> {
        eth_tb!(from, &self.inner)
    }

    fn set_from(&mut self, from: Address) {
        eth_tb!(set_from, &mut self.inner, from)
    }

    fn kind(&self) -> Option<TxKind> {
        eth_tb!(kind, &self.inner)
    }

    fn clear_kind(&mut self) {
        eth_tb!(clear_kind, &mut self.inner)
    }

    fn set_kind(&mut self, kind: TxKind) {
        eth_tb!(set_kind, &mut self.inner, kind)
    }

    fn value(&self) -> Option<U256> {
        eth_tb!(value, &self.inner)
    }

    fn set_value(&mut self, value: U256) {
        eth_tb!(set_value, &mut self.inner, value)
    }

    fn gas_price(&self) -> Option<u128> {
        eth_tb!(gas_price, &self.inner)
    }

    fn set_gas_price(&mut self, gas_price: u128) {
        eth_tb!(set_gas_price, &mut self.inner, gas_price)
    }

    fn max_fee_per_gas(&self) -> Option<u128> {
        eth_tb!(max_fee_per_gas, &self.inner)
    }

    fn set_max_fee_per_gas(&mut self, max_fee_per_gas: u128) {
        eth_tb!(set_max_fee_per_gas, &mut self.inner, max_fee_per_gas)
    }

    fn max_priority_fee_per_gas(&self) -> Option<u128> {
        eth_tb!(max_priority_fee_per_gas, &self.inner)
    }

    fn set_max_priority_fee_per_gas(&mut self, max_priority_fee_per_gas: u128) {
        eth_tb!(set_max_priority_fee_per_gas, &mut self.inner, max_priority_fee_per_gas)
    }

    fn gas_limit(&self) -> Option<u64> {
        eth_tb!(gas_limit, &self.inner)
    }

    fn set_gas_limit(&mut self, gas_limit: u64) {
        eth_tb!(set_gas_limit, &mut self.inner, gas_limit)
    }

    fn access_list(&self) -> Option<&AccessList> {
        eth_tb!(access_list, &self.inner)
    }

    fn set_access_list(&mut self, access_list: AccessList) {
        eth_tb!(set_access_list, &mut self.inner, access_list)
    }
}

impl NetworkTransactionBuilder<QuantumNetwork> for QuantumTransactionRequest {
    fn complete_type(&self, _ty: QuantumTxType) -> Result<(), Vec<&'static str>> {
        let mut missing = Vec::new();
        if self.sender.is_none() {
            missing.push("sender");
        }
        if self.inner.chain_id.is_none() {
            missing.push("chainId");
        }
        if self.inner.nonce.is_none() {
            missing.push("nonce");
        }
        if self.inner.gas.is_none() {
            missing.push("gas");
        }
        if self.inner.max_fee_per_gas.is_none() {
            missing.push("maxFeePerGas");
        }
        if self.inner.max_priority_fee_per_gas.is_none() {
            missing.push("maxPriorityFeePerGas");
        }
        if self.inner.to.is_none() {
            missing.push("to");
        }
        if missing.is_empty() { Ok(()) } else { Err(missing) }
    }

    fn can_submit(&self) -> bool {
        self.complete_type(QuantumTxType::Pq).is_ok()
    }

    fn can_build(&self) -> bool {
        self.can_submit()
    }

    fn output_tx_type(&self) -> QuantumTxType {
        QuantumTxType::Pq
    }

    fn output_tx_type_checked(&self) -> Option<QuantumTxType> {
        self.complete_type(QuantumTxType::Pq).ok().map(|()| QuantumTxType::Pq)
    }

    fn prep_for_submission(&mut self) {
        self.inner.transaction_type = Some(QUANTUM_TX_TYPE_ID);
        self.nonce_key.get_or_insert(U256::ZERO);
        self.key_id.get_or_insert(0);
    }

    fn build_unsigned(self) -> BuildResult<QuantumTxEnvelope, QuantumNetwork> {
        Err(alloy_network::TransactionBuilderError::<QuantumNetwork>::custom(std::io::Error::other(
            "building unsigned Quantum envelopes is not supported; use the explicit Quantum raw-signing flow",
        ))
        .into_unbuilt(self))
    }

    async fn build<W: alloy_network::NetworkWallet<QuantumNetwork>>(
        self,
        _wallet: &W,
    ) -> Result<QuantumTxEnvelope, alloy_network::TransactionBuilderError<QuantumNetwork>> {
        Err(alloy_network::TransactionBuilderError::custom(std::io::Error::other(
            "signing via QuantumNetwork wallet is not supported",
        )))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct QuantumSingleCall {
    kind: TxKind,
    value: U256,
    input: Bytes,
}

impl Encodable for QuantumSingleCall {
    fn encode(&self, out: &mut dyn BufMut) {
        let payload_len = self.kind.length() + self.value.length() + self.input.length();
        RlpHeader { list: true, payload_length: payload_len }.encode(out);
        self.kind.encode(out);
        self.value.encode(out);
        self.input.encode(out);
    }

    fn length(&self) -> usize {
        let payload_len = self.kind.length() + self.value.length() + self.input.length();
        alloy_rlp::length_of_length(payload_len) + payload_len
    }
}

fn encode_single_call_list(call: &QuantumSingleCall, out: &mut dyn BufMut) {
    let payload_length = call.length();
    RlpHeader { list: true, payload_length }.encode(out);
    call.encode(out);
}

fn single_call_list_length(call: &QuantumSingleCall) -> usize {
    let payload_length = call.length();
    alloy_rlp::length_of_length(payload_length) + payload_length
}

fn decode_single_call_list(buf: &mut &[u8]) -> alloy_rlp::Result<QuantumSingleCall> {
    let header = RlpHeader::decode(buf)?;
    if !header.list {
        return Err(alloy_rlp::Error::UnexpectedString);
    }
    let remaining = buf.len();
    let call = QuantumSingleCall::decode(buf)?;
    let consumed = remaining - buf.len();
    if consumed != header.payload_length {
        return Err(alloy_rlp::Error::ListLengthMismatch {
            expected: header.payload_length,
            got: consumed,
        });
    }
    Ok(call)
}

impl Decodable for QuantumSingleCall {
    fn decode(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        let header = RlpHeader::decode(buf)?;
        if !header.list {
            return Err(alloy_rlp::Error::UnexpectedString);
        }
        let remaining = buf.len();
        let kind = TxKind::decode(buf)?;
        let value = U256::decode(buf)?;
        let input = Bytes::decode(buf)?;
        let consumed = remaining - buf.len();
        if consumed != header.payload_length {
            return Err(alloy_rlp::Error::ListLengthMismatch {
                expected: header.payload_length,
                got: consumed,
            });
        }
        Ok(Self { kind, value, input })
    }
}

fn encode_empty_list(out: &mut dyn BufMut) {
    RlpHeader { list: true, payload_length: 0 }.encode(out);
}

fn decode_empty_list(buf: &mut &[u8]) -> alloy_rlp::Result<()> {
    let header = RlpHeader::decode(buf)?;
    if !header.list {
        return Err(alloy_rlp::Error::UnexpectedString);
    }
    if header.payload_length != 0 {
        return Err(alloy_rlp::Error::Custom("expected empty list for reserved field"));
    }
    Ok(())
}

fn encode_optional_list_bytes(value: Option<&Bytes>, out: &mut dyn BufMut) {
    match value {
        // Preserve the raw RLP list framing produced by `decode_list_bytes`.
        // `<[u8] as Encodable>::encode` would re-wrap with a string header and
        // break decode→encode idempotence (changes tx hash).
        Some(value) => out.put_slice(value.as_ref()),
        None => encode_empty_list(out),
    }
}

fn optional_list_bytes_length(value: Option<&Bytes>) -> usize {
    value.map_or(1, |v| v.len())
}

fn decode_optional_list_bytes(buf: &mut &[u8]) -> alloy_rlp::Result<Option<Bytes>> {
    let raw = decode_list_bytes(buf)?;
    if raw.len() == 1 && raw[0] == alloy_rlp::EMPTY_LIST_CODE { Ok(None) } else { Ok(Some(raw)) }
}

fn decode_list_bytes(buf: &mut &[u8]) -> alloy_rlp::Result<Bytes> {
    let start = *buf;
    let header = RlpHeader::decode(buf)?;
    if !header.list {
        return Err(alloy_rlp::Error::UnexpectedString);
    }
    let header_len = start.len() - buf.len();
    let total_len = header_len + header.payload_length;
    let raw = Bytes::copy_from_slice(&start[..total_len]);
    *buf = &start[total_len..];
    Ok(raw)
}

/// Encode an optional pubkey as `list(string(bytes))`, matching the signing-path
/// encoding in `QuantumWriteRequestV1`. Paired with `decode_option_pubkey_list`,
/// which strips the outer list + inner string framing so stored bytes carry
/// pubkey payload only.
fn encode_option_pubkey_list(value: Option<&Bytes>, out: &mut dyn BufMut) {
    match value {
        Some(value) => {
            let payload_length = value.length();
            RlpHeader { list: true, payload_length }.encode(out);
            value.encode(out);
        }
        None => encode_empty_list(out),
    }
}

fn option_pubkey_list_length(value: Option<&Bytes>) -> usize {
    match value {
        Some(value) => {
            let payload_length = value.length();
            alloy_rlp::length_of_length(payload_length) + payload_length
        }
        None => 1,
    }
}

fn decode_option_pubkey_list(buf: &mut &[u8]) -> alloy_rlp::Result<Option<Bytes>> {
    let header = RlpHeader::decode(buf)?;
    if !header.list {
        return Err(alloy_rlp::Error::UnexpectedString);
    }
    if header.payload_length == 0 {
        return Ok(None);
    }
    let start_len = buf.len();
    let pubkey = Bytes::decode(buf)?;
    let consumed = start_len - buf.len();
    if consumed != header.payload_length {
        return Err(alloy_rlp::Error::ListLengthMismatch {
            expected: header.payload_length,
            got: consumed,
        });
    }
    Ok(Some(pubkey))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuantumTxEnvelope {
    chain_id: ChainId,
    sender: Address,
    nonce_key: U256,
    nonce: u64,
    key_id: u32,
    max_priority_fee_per_gas: u128,
    max_fee_per_gas: u128,
    gas_limit: u64,
    call: QuantumSingleCall,
    access_list: AccessList,
    init_primary_pubkey: Option<Bytes>,
    init_cosigner_pubkey: Option<Bytes>,
    sender_sig: Bytes,
    fee_payer_sig: Option<Bytes>,
    hash: OnceLock<B256>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QuantumTxEnvelopeSerde {
    #[serde(rename = "type", with = "alloy_serde::quantity")]
    ty: u8,
    #[serde(with = "alloy_serde::quantity")]
    chain_id: ChainId,
    sender: Address,
    nonce_key: U256,
    #[serde(with = "alloy_serde::quantity")]
    nonce: u64,
    #[serde(with = "alloy_serde::quantity")]
    key_id: u32,
    #[serde(with = "alloy_serde::quantity")]
    max_priority_fee_per_gas: u128,
    #[serde(with = "alloy_serde::quantity")]
    max_fee_per_gas: u128,
    #[serde(rename = "gas", with = "alloy_serde::quantity")]
    gas_limit: u64,
    #[serde(default)]
    to: TxKind,
    value: U256,
    input: Bytes,
    access_list: AccessList,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    init_primary_pubkey: Option<Bytes>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    init_cosigner_pubkey: Option<Bytes>,
    sender_sig: Bytes,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fee_payer_sig: Option<Bytes>,
    hash: B256,
}

impl QuantumTxEnvelope {
    pub const fn sender(&self) -> Address {
        self.sender
    }

    pub const fn key_id(&self) -> u32 {
        self.key_id
    }

    pub const fn nonce_key(&self) -> U256 {
        self.nonce_key
    }

    #[allow(clippy::too_many_arguments)]
    pub const fn from_signed_parts(
        chain_id: ChainId,
        sender: Address,
        nonce_key: U256,
        nonce: u64,
        key_id: u32,
        max_priority_fee_per_gas: u128,
        max_fee_per_gas: u128,
        gas_limit: u64,
        kind: TxKind,
        value: U256,
        input: Bytes,
        access_list: AccessList,
        init_primary_pubkey: Option<Bytes>,
        init_cosigner_pubkey: Option<Bytes>,
        sender_sig: Bytes,
        fee_payer_sig: Option<Bytes>,
    ) -> Self {
        Self {
            chain_id,
            sender,
            nonce_key,
            nonce,
            key_id,
            max_priority_fee_per_gas,
            max_fee_per_gas,
            gas_limit,
            call: QuantumSingleCall { kind, value, input },
            access_list,
            init_primary_pubkey,
            init_cosigner_pubkey,
            sender_sig,
            fee_payer_sig,
            hash: OnceLock::new(),
        }
    }

    fn encoded_fields_length(&self) -> usize {
        self.chain_id.length()
            + self.sender.length()
            + self.nonce_key.length()
            + self.nonce.length()
            + self.key_id.length()
            + self.max_priority_fee_per_gas.length()
            + self.max_fee_per_gas.length()
            + self.gas_limit.length()
            + single_call_list_length(&self.call)
            + self.access_list.length()
            + 1
            + 1
            + option_pubkey_list_length(self.init_primary_pubkey.as_ref())
            + option_pubkey_list_length(self.init_cosigner_pubkey.as_ref())
    }

    fn encode_fields(&self, out: &mut dyn BufMut) {
        self.chain_id.encode(out);
        self.sender.encode(out);
        self.nonce_key.encode(out);
        self.nonce.encode(out);
        self.key_id.encode(out);
        self.max_priority_fee_per_gas.encode(out);
        self.max_fee_per_gas.encode(out);
        self.gas_limit.encode(out);
        encode_single_call_list(&self.call, out);
        self.access_list.encode(out);
        encode_empty_list(out);
        encode_empty_list(out);
        encode_option_pubkey_list(self.init_primary_pubkey.as_ref(), out);
        encode_option_pubkey_list(self.init_cosigner_pubkey.as_ref(), out);
    }

    fn encode_inner(&self, out: &mut dyn BufMut) {
        self.encode_fields(out);
        out.put_slice(self.sender_sig.as_ref());
        encode_optional_list_bytes(self.fee_payer_sig.as_ref(), out);
    }

    fn inner_length(&self) -> usize {
        self.encoded_fields_length()
            + self.sender_sig.length()
            + optional_list_bytes_length(self.fee_payer_sig.as_ref())
    }

    fn hash_value(&self) -> B256 {
        *self.hash.get_or_init(|| {
            let mut buf = Vec::with_capacity(self.encode_2718_len());
            self.encode_2718(&mut buf);
            keccak256(&buf)
        })
    }

    fn decode_inner(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        let chain_id = ChainId::decode(buf)?;
        let sender = Address::decode(buf)?;
        let nonce_key = U256::decode(buf)?;
        let nonce = u64::decode(buf)?;
        let key_id = u32::decode(buf)?;
        let max_priority_fee_per_gas = u128::decode(buf)?;
        let max_fee_per_gas = u128::decode(buf)?;
        let gas_limit = u64::decode(buf)?;
        let call = decode_single_call_list(buf)?;
        let access_list = AccessList::decode(buf)?;
        // Reserved fee-payer placeholders are always encoded as empty lists.
        // Reject non-empty values so decode→re-encode cannot silently change
        // the envelope hash.
        decode_empty_list(buf)?;
        decode_empty_list(buf)?;
        let init_primary_pubkey = decode_option_pubkey_list(buf)?;
        let init_cosigner_pubkey = decode_option_pubkey_list(buf)?;
        let sender_sig = decode_list_bytes(buf)?;
        let fee_payer_sig = decode_optional_list_bytes(buf)?;

        Ok(Self::from_signed_parts(
            chain_id,
            sender,
            nonce_key,
            nonce,
            key_id,
            max_priority_fee_per_gas,
            max_fee_per_gas,
            gas_limit,
            call.kind,
            call.value,
            call.input,
            access_list,
            init_primary_pubkey,
            init_cosigner_pubkey,
            sender_sig,
            fee_payer_sig,
        ))
    }
}

impl Serialize for QuantumTxEnvelope {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        QuantumTxEnvelopeSerde {
            ty: QUANTUM_TX_TYPE_ID,
            chain_id: self.chain_id,
            sender: self.sender,
            nonce_key: self.nonce_key,
            nonce: self.nonce,
            key_id: self.key_id,
            max_priority_fee_per_gas: self.max_priority_fee_per_gas,
            max_fee_per_gas: self.max_fee_per_gas,
            gas_limit: self.gas_limit,
            to: self.call.kind,
            value: self.call.value,
            input: self.call.input.clone(),
            access_list: self.access_list.clone(),
            init_primary_pubkey: self.init_primary_pubkey.clone(),
            init_cosigner_pubkey: self.init_cosigner_pubkey.clone(),
            sender_sig: self.sender_sig.clone(),
            fee_payer_sig: self.fee_payer_sig.clone(),
            hash: self.hash_value(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for QuantumTxEnvelope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let helper = QuantumTxEnvelopeSerde::deserialize(deserializer)?;
        if helper.ty != QUANTUM_TX_TYPE_ID {
            return Err(serde::de::Error::custom("unexpected Quantum transaction type"));
        }
        Ok(Self::from_signed_parts(
            helper.chain_id,
            helper.sender,
            helper.nonce_key,
            helper.nonce,
            helper.key_id,
            helper.max_priority_fee_per_gas,
            helper.max_fee_per_gas,
            helper.gas_limit,
            helper.to,
            helper.value,
            helper.input,
            helper.access_list,
            helper.init_primary_pubkey,
            helper.init_cosigner_pubkey,
            helper.sender_sig,
            helper.fee_payer_sig,
        ))
    }
}

impl Transaction for QuantumTxEnvelope {
    fn chain_id(&self) -> Option<ChainId> {
        Some(self.chain_id)
    }

    fn nonce(&self) -> u64 {
        self.nonce
    }

    fn gas_limit(&self) -> u64 {
        self.gas_limit
    }

    fn gas_price(&self) -> Option<u128> {
        None
    }

    fn max_fee_per_gas(&self) -> u128 {
        self.max_fee_per_gas
    }

    fn max_priority_fee_per_gas(&self) -> Option<u128> {
        Some(self.max_priority_fee_per_gas)
    }

    fn max_fee_per_blob_gas(&self) -> Option<u128> {
        None
    }

    fn priority_fee_or_price(&self) -> u128 {
        self.max_priority_fee_per_gas
    }

    fn effective_gas_price(&self, base_fee: Option<u64>) -> u128 {
        match base_fee {
            Some(base_fee) => {
                let base_fee = base_fee as u128;
                if self.max_fee_per_gas <= base_fee {
                    self.max_fee_per_gas
                } else {
                    (self.max_fee_per_gas - base_fee + base_fee)
                        .min(self.max_priority_fee_per_gas + base_fee)
                }
            }
            None => self.max_fee_per_gas,
        }
    }

    fn is_dynamic_fee(&self) -> bool {
        true
    }

    fn kind(&self) -> TxKind {
        self.call.kind
    }

    fn is_create(&self) -> bool {
        self.call.kind.is_create()
    }

    fn value(&self) -> U256 {
        self.call.value
    }

    fn input(&self) -> &Bytes {
        &self.call.input
    }

    fn access_list(&self) -> Option<&AccessList> {
        Some(&self.access_list)
    }

    fn blob_versioned_hashes(&self) -> Option<&[B256]> {
        None
    }

    fn authorization_list(&self) -> Option<&[SignedAuthorization]> {
        None
    }
}

impl TransactionEnvelope for QuantumTxEnvelope {
    type TxType = QuantumTxType;

    fn tx_type(&self) -> Self::TxType {
        QuantumTxType::Pq
    }
}

impl Typed2718 for QuantumTxEnvelope {
    fn ty(&self) -> u8 {
        QUANTUM_TX_TYPE_ID
    }
}

impl IsTyped2718 for QuantumTxEnvelope {
    fn is_type(ty: u8) -> bool {
        ty == QUANTUM_TX_TYPE_ID
    }
}

impl TxHashRef for QuantumTxEnvelope {
    fn tx_hash(&self) -> &TxHash {
        self.hash.get_or_init(|| {
            let mut buf = Vec::with_capacity(self.encode_2718_len());
            self.encode_2718(&mut buf);
            keccak256(&buf)
        })
    }
}

impl core::hash::Hash for QuantumTxEnvelope {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.tx_hash().hash(state)
    }
}

impl Encodable2718 for QuantumTxEnvelope {
    fn type_flag(&self) -> Option<u8> {
        Some(QUANTUM_TX_TYPE_ID)
    }

    fn encode_2718_len(&self) -> usize {
        let inner_len = self.inner_length();
        1 + alloy_rlp::length_of_length(inner_len) + inner_len
    }

    fn encode_2718(&self, out: &mut dyn BufMut) {
        out.put_u8(QUANTUM_TX_TYPE_ID);
        let inner_len = self.inner_length();
        RlpHeader { list: true, payload_length: inner_len }.encode(out);
        self.encode_inner(out);
    }
}

impl Decodable2718 for QuantumTxEnvelope {
    fn typed_decode(ty: u8, buf: &mut &[u8]) -> Result<Self, Eip2718Error> {
        if ty != QUANTUM_TX_TYPE_ID {
            return Err(Eip2718Error::UnexpectedType(ty));
        }
        let header = RlpHeader::decode(buf).map_err(Eip2718Error::RlpError)?;
        if !header.list {
            return Err(Eip2718Error::RlpError(alloy_rlp::Error::UnexpectedString));
        }
        let remaining = buf.len();
        let decoded = Self::decode_inner(buf).map_err(Eip2718Error::RlpError)?;
        let consumed = remaining - buf.len();
        if consumed != header.payload_length {
            return Err(Eip2718Error::RlpError(alloy_rlp::Error::ListLengthMismatch {
                expected: header.payload_length,
                got: consumed,
            }));
        }
        Ok(decoded)
    }

    fn fallback_decode(_buf: &mut &[u8]) -> Result<Self, Eip2718Error> {
        Err(Eip2718Error::UnexpectedType(0))
    }
}

impl SignerRecoverable for QuantumTxEnvelope {
    fn recover_signer(&self) -> Result<Address, alloy_consensus::crypto::RecoveryError> {
        Ok(self.sender)
    }

    fn recover_signer_unchecked(&self) -> Result<Address, alloy_consensus::crypto::RecoveryError> {
        Ok(self.sender)
    }
}

impl Encodable for QuantumTxEnvelope {
    fn encode(&self, out: &mut dyn BufMut) {
        self.network_encode(out)
    }

    fn length(&self) -> usize {
        self.network_len()
    }
}

impl Decodable for QuantumTxEnvelope {
    fn decode(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        Self::network_decode(buf).map_err(|_| alloy_rlp::Error::Custom("QuantumTxEnvelope decode"))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuantumReceiptEnvelope<T = Log> {
    inner: ReceiptWithBloom<Receipt<T>>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QuantumReceiptEnvelopeSerde<T> {
    #[serde(
        rename = "type",
        serialize_with = "serialize_quantum_receipt_type",
        deserialize_with = "deserialize_quantum_receipt_type"
    )]
    _ty: (),
    #[serde(flatten)]
    inner: ReceiptWithBloom<Receipt<T>>,
}

fn serialize_quantum_receipt_type<S>(_: &(), serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(QUANTUM_RECEIPT_TYPE_LABEL)
}

fn deserialize_quantum_receipt_type<'de, D>(deserializer: D) -> Result<(), D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if matches!(value.as_str(), "Pq" | "pq" | "PQ" | "0x7a" | "0x7A") {
        Ok(())
    } else {
        Err(serde::de::Error::custom("unexpected Quantum receipt type"))
    }
}

impl<T> Serialize for QuantumReceiptEnvelope<T>
where
    T: Clone + Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        QuantumReceiptEnvelopeSerde { _ty: (), inner: self.inner.clone() }.serialize(serializer)
    }
}

impl<'de, T> Deserialize<'de> for QuantumReceiptEnvelope<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let helper = QuantumReceiptEnvelopeSerde::deserialize(deserializer)?;
        Ok(Self { inner: helper.inner })
    }
}

impl<T> TxReceipt for QuantumReceiptEnvelope<T>
where
    T: Clone + core::fmt::Debug + PartialEq + Eq + Send + Sync,
{
    type Log = T;

    fn status_or_post_state(&self) -> alloy_consensus::Eip658Value {
        self.inner.receipt.status
    }

    fn status(&self) -> bool {
        self.inner.receipt.status.coerce_status()
    }

    fn bloom(&self) -> alloy_primitives::Bloom {
        self.inner.logs_bloom
    }

    fn bloom_cheap(&self) -> Option<alloy_primitives::Bloom> {
        Some(self.inner.logs_bloom)
    }

    fn cumulative_gas_used(&self) -> u64 {
        self.inner.receipt.cumulative_gas_used
    }

    fn logs(&self) -> &[T] {
        &self.inner.receipt.logs
    }
}

impl<T> Typed2718 for QuantumReceiptEnvelope<T> {
    fn ty(&self) -> u8 {
        QUANTUM_TX_TYPE_ID
    }
}

impl<T> Encodable2718 for QuantumReceiptEnvelope<T>
where
    T: Clone + core::fmt::Debug + PartialEq + Eq + Send + Sync + Encodable,
{
    fn encode_2718_len(&self) -> usize {
        1 + self.inner.length()
    }

    fn encode_2718(&self, out: &mut dyn BufMut) {
        out.put_u8(QUANTUM_TX_TYPE_ID);
        self.inner.encode(out);
    }
}

impl<T> Decodable2718 for QuantumReceiptEnvelope<T>
where
    T: Decodable + Clone + core::fmt::Debug + PartialEq + Eq + Send + Sync,
{
    fn typed_decode(ty: u8, buf: &mut &[u8]) -> Result<Self, Eip2718Error> {
        if ty != QUANTUM_TX_TYPE_ID {
            return Err(Eip2718Error::UnexpectedType(ty));
        }
        ReceiptWithBloom::decode(buf).map(|inner| Self { inner }).map_err(Eip2718Error::RlpError)
    }

    fn fallback_decode(buf: &mut &[u8]) -> Result<Self, Eip2718Error> {
        ReceiptWithBloom::decode(buf).map(|inner| Self { inner }).map_err(Eip2718Error::RlpError)
    }
}

impl<T> Encodable for QuantumReceiptEnvelope<T>
where
    T: Clone + core::fmt::Debug + PartialEq + Eq + Send + Sync + Encodable,
{
    fn encode(&self, out: &mut dyn BufMut) {
        self.encode_2718(out)
    }

    fn length(&self) -> usize {
        self.encode_2718_len()
    }
}

impl<T> Decodable for QuantumReceiptEnvelope<T>
where
    T: Decodable + Clone + core::fmt::Debug + PartialEq + Eq + Send + Sync,
{
    fn decode(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        Self::decode_2718(buf).map_err(Into::into)
    }
}

pub type QuantumTxReceipt = TransactionReceipt<QuantumReceiptEnvelope<Log>>;

#[derive(Debug, Clone, Copy)]
pub struct QuantumNetwork;

impl Network for QuantumNetwork {
    type TxType = QuantumTxType;
    type TxEnvelope = QuantumTxEnvelope;
    type UnsignedTx = QuantumTxEnvelope;
    type ReceiptEnvelope = QuantumReceiptEnvelope<alloy_primitives::Log>;
    type Header = alloy_consensus::Header;
    type TransactionRequest = QuantumTransactionRequest;
    type TransactionResponse = RpcTransaction<QuantumTxEnvelope>;
    type ReceiptResponse = QuantumTxReceipt;
    type HeaderResponse = Header;
    type BlockResponse = Block<Self::TransactionResponse, Self::HeaderResponse>;
}

impl RecommendedFillers for QuantumNetwork {
    type RecommendedFillers = JoinFill<GasFiller, JoinFill<NonceFiller, ChainIdFiller>>;

    fn recommended_fillers() -> Self::RecommendedFillers {
        Default::default()
    }
}

#[cfg(test)]
mod tests {
    use alloy_network::ReceiptResponse as _;
    use alloy_provider::network::eip2718::Decodable2718 as _;

    use super::*;

    fn raw_fixture() -> serde_json::Value {
        serde_json::from_str(include_str!(
            "../../../../testdata/fixtures/quantum/phase0/raw-send-primary.json"
        ))
        .unwrap()
    }

    #[test]
    fn decodes_phase0_raw_transaction_fixture() {
        let value = raw_fixture();
        let raw = value["raw_transaction"].as_str().unwrap();
        let bytes = alloy_primitives::hex::decode(raw).unwrap();
        let tx = QuantumTxEnvelope::decode_2718(&mut bytes.as_slice()).unwrap();

        assert_eq!(tx.ty(), QUANTUM_TX_TYPE_ID);
        let expected_sender: Address = value["sender"].as_str().unwrap().parse().unwrap();
        assert_eq!(tx.sender(), expected_sender);
        assert_eq!(tx.key_id(), value["key_id"].as_u64().unwrap() as u32);
        assert_eq!(tx.nonce_key(), U256::ZERO);
    }

    #[test]
    fn envelope_bootstrap_pubkey_round_trips_to_request_as_semantic_bytes() {
        // Exercise the decoded envelope path by round-tripping the phase-0
        // fixture, which has `init_primary_pubkey = None`, and confirm the
        // request view also sees `None` (not the raw RLP empty-list marker).
        let fixture = raw_fixture();
        let raw = fixture["raw_transaction"].as_str().unwrap();
        let bytes = alloy_primitives::hex::decode(raw).unwrap();
        let decoded = QuantumTxEnvelope::decode_2718(&mut bytes.as_slice()).unwrap();
        assert!(decoded.init_primary_pubkey.is_none());

        let request: QuantumTransactionRequest = decoded.into();
        assert!(request.init_primary_pubkey.is_none());
        assert!(request.init_cosigner_pubkey.is_none());
    }

    #[test]
    fn round_trips_quantum_transaction_request_json() {
        let request = QuantumTransactionRequest {
            inner: TransactionRequest::default()
                .with_chain_id(1337)
                .with_nonce(7)
                .with_to(Address::repeat_byte(0x11))
                .with_gas_limit(21_000)
                .with_max_fee_per_gas(10)
                .with_max_priority_fee_per_gas(1),
            sender: Some(Address::repeat_byte(0x22)),
            key_id: Some(5),
            nonce_key: Some(U256::ZERO),
            init_primary_pubkey: None,
            init_cosigner_pubkey: None,
        };

        let json = serde_json::to_string(&request).unwrap();
        let decoded: QuantumTransactionRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, request);
    }

    #[test]
    fn decode_empty_list_rejects_non_empty_reserved_placeholder() {
        // The canonical encoder writes reserved fee-payer placeholder fields
        // as empty lists. `decode_empty_list` must reject any other list so
        // decode→re-encode cannot silently change the envelope hash.
        let empty = [alloy_rlp::EMPTY_LIST_CODE];
        assert!(decode_empty_list(&mut empty.as_ref()).is_ok());

        // `list(string(0x80))`: outer list header `0xc1` followed by empty string `0x80`.
        let non_empty = [0xc1u8, 0x80u8];
        let err = decode_empty_list(&mut non_empty.as_ref())
            .expect_err("non-empty list must be rejected");
        assert!(format!("{err}").contains("reserved"), "unexpected error: {err}");

        // A string (non-list) must also be rejected.
        let string_bytes = [0x80u8];
        let err =
            decode_empty_list(&mut string_bytes.as_ref()).expect_err("string must be rejected");
        assert!(matches!(err, alloy_rlp::Error::UnexpectedString));
    }

    #[test]
    fn deserializes_pq_receipt_type() {
        let receipt: QuantumTxReceipt = serde_json::from_str(
            r#"{
                "type":"Pq",
                "status":"0x1",
                "cumulativeGasUsed":"0x5208",
                "logs":[],
                "logsBloom":"0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
                "transactionHash":"0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "transactionIndex":"0x0",
                "blockHash":"0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "blockNumber":"0x1",
                "gasUsed":"0x5208",
                "effectiveGasPrice":"0x1",
                "from":"0x1111111111111111111111111111111111111111",
                "to":"0x2222222222222222222222222222222222222222",
                "contractAddress":null
            }"#,
        )
        .unwrap();

        assert!(receipt.status());
        assert_eq!(receipt.transaction_hash, TxHash::repeat_byte(0xaa));
    }
}
