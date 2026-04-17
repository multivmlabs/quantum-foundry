use std::num::NonZeroU64;

use alloy_consensus::{
    BlobTransactionSidecar, BlobTransactionSidecarEip7594, BlobTransactionSidecarVariant,
};
use alloy_eips::{Encodable2718, eip7702::SignedAuthorization};
use alloy_network::{AnyNetwork, Ethereum, Network, NetworkTransactionBuilder, TransactionBuilder};
use alloy_primitives::{Address, B256, Bytes, Signature, TxKind, U256, address, keccak256};
use alloy_provider::Provider;
use alloy_rlp::{BufMut, Encodable, Header as RlpHeader};
use alloy_signer::Signer;
use eyre::{Result, bail, ensure};
use foundry_primitives::{QuantumNetwork, QuantumTransactionRequest};
use op_alloy_network::Optimism;
use op_alloy_rpc_types::OpTransactionRequest;
use tempo_alloy::{TempoNetwork, provider::TempoProviderExt};
use tempo_primitives::{
    TempoSignature,
    transaction::{Call, KeychainSignature, PrimitiveSignature, SignedKeyAuthorization},
};

/// Composite transaction builder trait for Foundry transactions.
///
/// This extends the base `TransactionBuilder` trait with the same methods as
/// [`alloy_network::TransactionBuilder4844`] for handling blob transaction sidecars, and
/// [`alloy_network::TransactionBuilder7702`] for handling EIP-7702 authorization lists.
///
/// By default, all methods have no-op implementations, so this can be implemented for any Network.
///
/// If the Network supports Eip4844 blob transactions implement these methods:
/// - [`FoundryTransactionBuilder::max_fee_per_blob_gas`]
/// - [`FoundryTransactionBuilder::set_max_fee_per_blob_gas`]
/// - [`FoundryTransactionBuilder::blob_versioned_hashes`]
/// - [`FoundryTransactionBuilder::set_blob_versioned_hashes`]
/// - [`FoundryTransactionBuilder::blob_sidecar`]
/// - [`FoundryTransactionBuilder::set_blob_sidecar`]
///
/// If the Network supports EIP-7702 authorization lists, implement these methods:
/// - [`FoundryTransactionBuilder::authorization_list`]
/// - [`FoundryTransactionBuilder::set_authorization_list`]
///
/// If the Network supports Tempo transactions, implement these methods:
/// - [`FoundryTransactionBuilder::set_fee_token`]
/// - [`FoundryTransactionBuilder::set_nonce_key`]
/// - [`FoundryTransactionBuilder::set_key_id`]
/// - [`FoundryTransactionBuilder::set_valid_before`]
/// - [`FoundryTransactionBuilder::set_valid_after`]
/// - [`FoundryTransactionBuilder::set_fee_payer_signature`]
pub trait FoundryTransactionBuilder<N: Network>: NetworkTransactionBuilder<N> {
    /// Reset gas limit
    fn reset_gas_limit(&mut self);

    /// Get the max fee per blob gas for the transaction.
    fn max_fee_per_blob_gas(&self) -> Option<u128> {
        None
    }

    /// Set the max fee per blob gas for the transaction.
    fn set_max_fee_per_blob_gas(&mut self, _max_fee_per_blob_gas: u128) {}

    /// Builder-pattern method for setting max fee per blob gas.
    fn with_max_fee_per_blob_gas(mut self, max_fee_per_blob_gas: u128) -> Self {
        self.set_max_fee_per_blob_gas(max_fee_per_blob_gas);
        self
    }

    /// Gets the EIP-4844 blob versioned hashes of the transaction.
    ///
    /// These may be set independently of the sidecar, e.g. when the sidecar
    /// has been pruned but the hashes are still needed for `eth_call`.
    fn blob_versioned_hashes(&self) -> Option<&[B256]> {
        None
    }

    /// Sets the EIP-4844 blob versioned hashes of the transaction.
    fn set_blob_versioned_hashes(&mut self, _hashes: Vec<B256>) {}

    /// Builder-pattern method for setting the EIP-4844 blob versioned hashes.
    fn with_blob_versioned_hashes(mut self, hashes: Vec<B256>) -> Self {
        self.set_blob_versioned_hashes(hashes);
        self
    }

    /// Gets the blob sidecar (either EIP-4844 or EIP-7594 variant) of the transaction.
    fn blob_sidecar(&self) -> Option<&BlobTransactionSidecarVariant> {
        None
    }

    /// Sets the blob sidecar (either EIP-4844 or EIP-7594 variant) of the transaction.
    ///
    /// Note: This will also set the versioned blob hashes accordingly:
    /// [BlobTransactionSidecarVariant::versioned_hashes]
    fn set_blob_sidecar(&mut self, _sidecar: BlobTransactionSidecarVariant) {}

    /// Builder-pattern method for setting the blob sidecar of the transaction.
    fn with_blob_sidecar(mut self, sidecar: BlobTransactionSidecarVariant) -> Self {
        self.set_blob_sidecar(sidecar);
        self
    }

    /// Gets the EIP-4844 blob sidecar if the current sidecar is of that variant.
    fn blob_sidecar_4844(&self) -> Option<&BlobTransactionSidecar> {
        self.blob_sidecar().and_then(|s| s.as_eip4844())
    }

    /// Sets the EIP-4844 blob sidecar of the transaction.
    fn set_blob_sidecar_4844(&mut self, sidecar: BlobTransactionSidecar) {
        self.set_blob_sidecar(BlobTransactionSidecarVariant::Eip4844(sidecar));
    }

    /// Builder-pattern method for setting the EIP-4844 blob sidecar of the transaction.
    fn with_blob_sidecar_4844(mut self, sidecar: BlobTransactionSidecar) -> Self {
        self.set_blob_sidecar_4844(sidecar);
        self
    }

    /// Gets the EIP-7594 blob sidecar if the current sidecar is of that variant.
    fn blob_sidecar_7594(&self) -> Option<&BlobTransactionSidecarEip7594> {
        self.blob_sidecar().and_then(|s| s.as_eip7594())
    }

    /// Sets the EIP-7594 blob sidecar of the transaction.
    fn set_blob_sidecar_7594(&mut self, sidecar: BlobTransactionSidecarEip7594) {
        self.set_blob_sidecar(BlobTransactionSidecarVariant::Eip7594(sidecar));
    }

    /// Builder-pattern method for setting the EIP-7594 blob sidecar of the transaction.
    fn with_blob_sidecar_7594(mut self, sidecar: BlobTransactionSidecarEip7594) -> Self {
        self.set_blob_sidecar_7594(sidecar);
        self
    }

    /// Get the EIP-7702 authorization list for the transaction.
    fn authorization_list(&self) -> Option<&Vec<SignedAuthorization>> {
        None
    }

    /// Sets the EIP-7702 authorization list.
    fn set_authorization_list(&mut self, _authorization_list: Vec<SignedAuthorization>) {}

    /// Builder-pattern method for setting the authorization list.
    fn with_authorization_list(mut self, authorization_list: Vec<SignedAuthorization>) -> Self {
        self.set_authorization_list(authorization_list);
        self
    }

    /// Get the fee token for a Tempo transaction.
    fn fee_token(&self) -> Option<Address> {
        None
    }

    /// Set the fee token for a Tempo transaction.
    fn set_fee_token(&mut self, _fee_token: Address) {}

    /// Builder-pattern method for setting the Tempo fee token.
    fn with_fee_token(mut self, fee_token: Address) -> Self {
        self.set_fee_token(fee_token);
        self
    }

    /// Get the 2D nonce key for a Tempo transaction.
    fn nonce_key(&self) -> Option<U256> {
        None
    }

    /// Set the 2D nonce key for the Tempo transaction.
    fn set_nonce_key(&mut self, _nonce_key: U256) {}

    /// Builder-pattern method for setting a 2D nonce key for a Tempo transaction.
    fn with_nonce_key(mut self, nonce_key: U256) -> Self {
        self.set_nonce_key(nonce_key);
        self
    }

    /// Get the access key ID for a Tempo transaction.
    fn key_id(&self) -> Option<Address> {
        None
    }

    /// Set the access key ID for a Tempo transaction.
    ///
    /// Used during gas estimation to override the key_id that would normally be
    /// recovered from the signature.
    fn set_key_id(&mut self, _key_id: Address) {}

    /// Builder-pattern method for setting the Tempo access key ID.
    fn with_key_id(mut self, key_id: Address) -> Self {
        self.set_key_id(key_id);
        self
    }

    /// Get the valid_before timestamp for a Tempo expiring nonce transaction.
    fn valid_before(&self) -> Option<NonZeroU64> {
        None
    }

    /// Set the valid_before timestamp for a Tempo expiring nonce transaction.
    fn set_valid_before(&mut self, _valid_before: NonZeroU64) {}

    /// Builder-pattern method for setting the valid_before timestamp.
    fn with_valid_before(mut self, valid_before: NonZeroU64) -> Self {
        self.set_valid_before(valid_before);
        self
    }

    /// Get the valid_after timestamp for a Tempo expiring nonce transaction.
    fn valid_after(&self) -> Option<NonZeroU64> {
        None
    }

    /// Set the valid_after timestamp for a Tempo expiring nonce transaction.
    fn set_valid_after(&mut self, _valid_after: NonZeroU64) {}

    /// Builder-pattern method for setting the valid_after timestamp.
    fn with_valid_after(mut self, valid_after: NonZeroU64) -> Self {
        self.set_valid_after(valid_after);
        self
    }

    /// Get the fee payer (sponsor) signature for a Tempo sponsored transaction.
    fn fee_payer_signature(&self) -> Option<Signature> {
        None
    }

    /// Set the fee payer (sponsor) signature for a Tempo sponsored transaction.
    fn set_fee_payer_signature(&mut self, _signature: Signature) {}

    /// Builder-pattern method for setting the fee payer signature.
    fn with_fee_payer_signature(mut self, signature: Signature) -> Self {
        self.set_fee_payer_signature(signature);
        self
    }

    /// Get the explicit Quantum sender for a Quantum transaction.
    fn quantum_sender(&self) -> Option<Address> {
        None
    }

    /// Set the explicit Quantum sender for a Quantum transaction.
    fn set_quantum_sender(&mut self, _sender: Address) {}

    /// Builder-pattern method for setting the explicit Quantum sender.
    fn with_quantum_sender(mut self, sender: Address) -> Self {
        self.set_quantum_sender(sender);
        self
    }

    /// Get the explicit Quantum key lane for a Quantum transaction.
    fn quantum_key_id(&self) -> Option<u32> {
        None
    }

    /// Set the explicit Quantum key lane for a Quantum transaction.
    fn set_quantum_key_id(&mut self, _key_id: u32) {}

    /// Builder-pattern method for setting the explicit Quantum key lane.
    fn with_quantum_key_id(mut self, key_id: u32) -> Self {
        self.set_quantum_key_id(key_id);
        self
    }

    /// Get the Quantum nonce lane for a Quantum transaction.
    fn quantum_nonce_key(&self) -> Option<U256> {
        None
    }

    /// Set the Quantum nonce lane for a Quantum transaction.
    fn set_quantum_nonce_key(&mut self, _nonce_key: U256) {}

    /// Builder-pattern method for setting the Quantum nonce lane.
    fn with_quantum_nonce_key(mut self, nonce_key: U256) -> Self {
        self.set_quantum_nonce_key(nonce_key);
        self
    }

    /// Get the Quantum bootstrap primary pubkey for a Quantum transaction.
    fn quantum_init_primary_pubkey(&self) -> Option<&Bytes> {
        None
    }

    /// Set the Quantum bootstrap primary pubkey for a Quantum transaction.
    fn set_quantum_init_primary_pubkey(&mut self, _pubkey: Bytes) {}

    /// Builder-pattern method for setting the Quantum bootstrap primary pubkey.
    fn with_quantum_init_primary_pubkey(mut self, pubkey: Bytes) -> Self {
        self.set_quantum_init_primary_pubkey(pubkey);
        self
    }

    /// Get the Quantum bootstrap cosigner pubkey for a Quantum transaction.
    fn quantum_init_cosigner_pubkey(&self) -> Option<&Bytes> {
        None
    }

    /// Set the Quantum bootstrap cosigner pubkey for a Quantum transaction.
    fn set_quantum_init_cosigner_pubkey(&mut self, _pubkey: Bytes) {}

    /// Builder-pattern method for setting the Quantum bootstrap cosigner pubkey.
    fn with_quantum_init_cosigner_pubkey(mut self, pubkey: Bytes) -> Self {
        self.set_quantum_init_cosigner_pubkey(pubkey);
        self
    }

    /// Computes the sponsor (fee payer) signature hash for this transaction.
    ///
    /// This builds an unsigned consensus-level transaction from the request and computes
    /// the hash that a sponsor needs to sign. Returns `None` for networks that don't
    /// support sponsored transactions.
    fn compute_sponsor_hash(&self, _from: Address) -> Option<B256> {
        None
    }

    /// Set the key authorization for a Tempo transaction.
    ///
    /// Embeds a [`SignedKeyAuthorization`] in the transaction body, provisioning the access key
    /// on-chain as part of this transaction.
    fn set_key_authorization(&mut self, _key_authorization: SignedKeyAuthorization) {}

    /// Converts a CREATE transaction into an AA-compatible call entry.
    ///
    /// Tempo AA transactions use a `calls` list instead of `to`+`input`. Must be
    /// called before gas estimation so the RPC sees the correct tx structure.
    /// No-op for non-Tempo networks.
    fn convert_create_to_call(&mut self) {}

    /// Clears the `to` and `value` fields for batch transactions that use `calls`.
    ///
    /// In Tempo AA batch transactions, targets are specified in the `calls` field, not in `to`.
    /// If `to` is set, `build_aa()` would add a spurious extra call. Must be called after
    /// `prepare()` sets `kind`/`to` but before gas estimation.
    /// No-op for non-Tempo networks.
    fn clear_batch_to(&mut self) {}

    /// Signs the transaction using an access key (keychain mode).
    ///
    /// If `key_authorization` is provided and the key is not yet provisioned on-chain,
    /// embeds the authorization in the transaction before signing.
    ///
    /// The default implementation returns an error. Only `TempoNetwork` supports this.
    fn sign_with_access_key(
        self,
        _provider: &impl Provider<N>,
        _signer: &(impl Signer + Sync),
        _wallet_address: Address,
        _key_address: Address,
        _key_authorization: Option<&SignedKeyAuthorization>,
    ) -> impl Future<Output = Result<Vec<u8>>> + Send {
        async { eyre::bail!("access key signing is not supported for this network") }
    }
}

pub const QUANTUM_TX_TYPE_ID: u8 = 0x7A;
pub const QUANTUM_KEYVAULT_ADDRESS: Address = address!("0000000000000000000000000000000000001000");
pub const QUANTUM_BOOTSTRAP_SELECTOR: [u8; 4] = [0x5e, 0x8e, 0x7a, 0x13];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuantumSingleCall {
    pub kind: TxKind,
    pub value: U256,
    pub input: Bytes,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuantumBootstrapFieldsV1 {
    pub init_primary_pubkey: Bytes,
    pub init_cosigner_pubkey: Option<Bytes>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuantumWriteRequestInputsV1 {
    pub sender: Address,
    pub key_id: u32,
    pub nonce_key: Option<U256>,
    pub bootstrap: Option<QuantumBootstrapFieldsV1>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuantumWriteRequestV1 {
    pub sender: Address,
    pub key_id: u32,
    pub nonce: u64,
    pub chain_id: u64,
    pub max_priority_fee_per_gas: u128,
    pub max_fee_per_gas: u128,
    pub gas_limit: u64,
    pub call: QuantumSingleCall,
    pub access_list: alloy_eips::eip2930::AccessList,
    pub bootstrap: Option<QuantumBootstrapFieldsV1>,
}

impl QuantumWriteRequestV1 {
    pub fn from_transaction_request<B: TransactionBuilder>(
        tx: &B,
        inputs: QuantumWriteRequestInputsV1,
    ) -> Result<Self> {
        if let Some(nonce_key) = inputs.nonce_key
            && nonce_key != U256::ZERO
        {
            bail!("Quantum v1 only supports nonce_key = 0")
        }

        let request = Self {
            sender: inputs.sender,
            key_id: inputs.key_id,
            nonce: TransactionBuilder::nonce(tx)
                .ok_or_else(|| eyre::eyre!("Quantum request missing nonce"))?,
            chain_id: TransactionBuilder::chain_id(tx)
                .ok_or_else(|| eyre::eyre!("Quantum request missing chain_id"))?,
            max_priority_fee_per_gas: TransactionBuilder::max_priority_fee_per_gas(tx)
                .ok_or_else(|| eyre::eyre!("Quantum request missing max_priority_fee_per_gas"))?,
            max_fee_per_gas: TransactionBuilder::max_fee_per_gas(tx)
                .ok_or_else(|| eyre::eyre!("Quantum request missing max_fee_per_gas"))?,
            gas_limit: TransactionBuilder::gas_limit(tx)
                .ok_or_else(|| eyre::eyre!("Quantum request missing gas_limit"))?,
            call: QuantumSingleCall {
                kind: TransactionBuilder::kind(tx)
                    .ok_or_else(|| eyre::eyre!("Quantum request missing call or CREATE kind"))?,
                value: TransactionBuilder::value(tx).unwrap_or_default(),
                input: TransactionBuilder::input(tx).cloned().unwrap_or_default(),
            },
            access_list: TransactionBuilder::access_list(tx).cloned().unwrap_or_default(),
            bootstrap: inputs.bootstrap,
        };

        request.validate_v1()?;
        Ok(request)
    }

    pub fn from_quantum_transaction_request(tx: &QuantumTransactionRequest) -> Result<Self> {
        let bootstrap = match (tx.init_primary_pubkey.clone(), tx.init_cosigner_pubkey.clone()) {
            (None, None) => None,
            (Some(init_primary_pubkey), init_cosigner_pubkey) => {
                Some(QuantumBootstrapFieldsV1 { init_primary_pubkey, init_cosigner_pubkey })
            }
            (None, Some(_)) => {
                bail!("Quantum bootstrap cosigner pubkey requires an accompanying primary pubkey")
            }
        };

        Self::from_transaction_request(
            tx,
            QuantumWriteRequestInputsV1 {
                sender: tx.sender.ok_or_else(|| eyre::eyre!("Quantum request missing sender"))?,
                key_id: tx.key_id.unwrap_or(0),
                nonce_key: Some(tx.nonce_key.unwrap_or(U256::ZERO)),
                bootstrap,
            },
        )
    }

    pub fn validate_v1(&self) -> Result<()> {
        ensure!(self.sender != Address::ZERO, "Quantum sender must be explicit and non-zero");
        ensure!(
            self.max_priority_fee_per_gas <= self.max_fee_per_gas,
            "max priority fee must not exceed max fee"
        );

        if let Some(bootstrap) = &self.bootstrap {
            ensure!(
                self.is_bootstrap_call(),
                "Quantum bootstrap fields are only valid for bootstrapKey() writes"
            );
            ensure!(
                bootstrap.init_cosigner_pubkey.is_none(),
                "Quantum bootstrap remains primary-only in v1"
            );
        }

        if self.is_bootstrap_call() {
            ensure!(
                self.key_id == 0,
                "Quantum bootstrap requests must use key_id = 0 (account-lane primary); got key_id = {}",
                self.key_id
            );
        }

        Ok(())
    }

    pub fn is_bootstrap_call(&self) -> bool {
        matches!(self.call.kind, TxKind::Call(to) if to == QUANTUM_KEYVAULT_ADDRESS)
            && self.call.input.as_ref().starts_with(&QUANTUM_BOOTSTRAP_SELECTOR)
    }

    pub fn encode_fields(&self, out: &mut dyn BufMut) {
        self.chain_id.encode(out);
        self.sender.encode(out);
        U256::ZERO.encode(out);
        self.nonce.encode(out);
        self.key_id.encode(out);
        self.max_priority_fee_per_gas.encode(out);
        self.max_fee_per_gas.encode(out);
        self.gas_limit.encode(out);
        encode_single_call_list(&self.call, out);
        self.access_list.encode(out);
        encode_option_as_list::<Address>(None, out);
        encode_option_as_list::<u32>(None, out);
        encode_option_as_list(
            self.bootstrap.as_ref().map(|bootstrap| &bootstrap.init_primary_pubkey),
            out,
        );
        encode_option_as_list(
            self.bootstrap.as_ref().and_then(|bootstrap| bootstrap.init_cosigner_pubkey.as_ref()),
            out,
        );
    }

    pub fn encoded_fields_length(&self) -> usize {
        self.chain_id.length()
            + self.sender.length()
            + U256::ZERO.length()
            + self.nonce.length()
            + self.key_id.length()
            + self.max_priority_fee_per_gas.length()
            + self.max_fee_per_gas.length()
            + self.gas_limit.length()
            + single_call_list_length(&self.call)
            + self.access_list.length()
            + option_as_list_length::<Address>(None)
            + option_as_list_length::<u32>(None)
            + option_as_list_length(
                self.bootstrap.as_ref().map(|bootstrap| &bootstrap.init_primary_pubkey),
            )
            + option_as_list_length(
                self.bootstrap
                    .as_ref()
                    .and_then(|bootstrap| bootstrap.init_cosigner_pubkey.as_ref()),
            )
    }

    pub fn encode_for_signing(&self, out: &mut dyn BufMut) {
        out.put_u8(QUANTUM_TX_TYPE_ID);
        let payload_len = self.encoded_fields_length();
        RlpHeader { list: true, payload_length: payload_len }.encode(out);
        self.encode_fields(out);
    }

    pub fn encode_body(&self, out: &mut dyn BufMut) {
        let payload_len = self.encoded_fields_length();
        RlpHeader { list: true, payload_length: payload_len }.encode(out);
        self.encode_fields(out);
    }

    pub fn signature_hash(&self) -> B256 {
        let mut buf = Vec::new();
        self.encode_for_signing(&mut buf);
        keccak256(buf)
    }
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

impl FoundryTransactionBuilder<Ethereum> for <Ethereum as Network>::TransactionRequest {
    fn reset_gas_limit(&mut self) {
        self.gas = None;
    }

    fn max_fee_per_blob_gas(&self) -> Option<u128> {
        self.max_fee_per_blob_gas
    }

    fn set_max_fee_per_blob_gas(&mut self, max_fee_per_blob_gas: u128) {
        self.max_fee_per_blob_gas = Some(max_fee_per_blob_gas);
    }

    fn blob_versioned_hashes(&self) -> Option<&[B256]> {
        self.blob_versioned_hashes.as_deref()
    }

    fn set_blob_versioned_hashes(&mut self, hashes: Vec<B256>) {
        self.blob_versioned_hashes = Some(hashes);
    }

    fn blob_sidecar(&self) -> Option<&BlobTransactionSidecarVariant> {
        self.sidecar.as_ref()
    }

    fn set_blob_sidecar(&mut self, sidecar: BlobTransactionSidecarVariant) {
        self.sidecar = Some(sidecar);
        self.populate_blob_hashes();
    }

    fn authorization_list(&self) -> Option<&Vec<SignedAuthorization>> {
        self.authorization_list.as_ref()
    }

    fn set_authorization_list(&mut self, authorization_list: Vec<SignedAuthorization>) {
        self.authorization_list = Some(authorization_list);
    }
}

impl FoundryTransactionBuilder<AnyNetwork> for <AnyNetwork as Network>::TransactionRequest {
    fn reset_gas_limit(&mut self) {
        self.gas = None;
    }

    fn max_fee_per_blob_gas(&self) -> Option<u128> {
        self.max_fee_per_blob_gas
    }

    fn set_max_fee_per_blob_gas(&mut self, max_fee_per_blob_gas: u128) {
        self.max_fee_per_blob_gas = Some(max_fee_per_blob_gas);
    }

    fn blob_versioned_hashes(&self) -> Option<&[B256]> {
        self.blob_versioned_hashes.as_deref()
    }

    fn set_blob_versioned_hashes(&mut self, hashes: Vec<B256>) {
        self.blob_versioned_hashes = Some(hashes);
    }

    fn blob_sidecar(&self) -> Option<&BlobTransactionSidecarVariant> {
        self.sidecar.as_ref()
    }

    fn set_blob_sidecar(&mut self, sidecar: BlobTransactionSidecarVariant) {
        self.sidecar = Some(sidecar);
        self.populate_blob_hashes();
    }

    fn authorization_list(&self) -> Option<&Vec<SignedAuthorization>> {
        self.authorization_list.as_ref()
    }

    fn set_authorization_list(&mut self, authorization_list: Vec<SignedAuthorization>) {
        self.authorization_list = Some(authorization_list);
    }
}

impl FoundryTransactionBuilder<Optimism> for OpTransactionRequest {
    fn reset_gas_limit(&mut self) {
        self.as_mut().gas = None;
    }

    fn authorization_list(&self) -> Option<&Vec<SignedAuthorization>> {
        self.as_ref().authorization_list.as_ref()
    }

    fn set_authorization_list(&mut self, authorization_list: Vec<SignedAuthorization>) {
        self.as_mut().authorization_list = Some(authorization_list);
    }
}

impl FoundryTransactionBuilder<TempoNetwork> for <TempoNetwork as Network>::TransactionRequest {
    fn reset_gas_limit(&mut self) {
        self.gas = None;
    }

    fn authorization_list(&self) -> Option<&Vec<SignedAuthorization>> {
        self.authorization_list.as_ref()
    }

    fn set_authorization_list(&mut self, authorization_list: Vec<SignedAuthorization>) {
        self.authorization_list = Some(authorization_list);
    }

    fn fee_token(&self) -> Option<Address> {
        self.fee_token
    }

    fn set_fee_token(&mut self, fee_token: Address) {
        self.fee_token = Some(fee_token);
    }

    fn nonce_key(&self) -> Option<U256> {
        self.nonce_key
    }

    fn set_nonce_key(&mut self, nonce_key: U256) {
        self.nonce_key = Some(nonce_key);
    }

    fn key_id(&self) -> Option<Address> {
        self.key_id
    }

    fn set_key_id(&mut self, key_id: Address) {
        self.key_id = Some(key_id);
    }

    fn valid_before(&self) -> Option<NonZeroU64> {
        self.valid_before
    }

    fn set_valid_before(&mut self, valid_before: NonZeroU64) {
        self.valid_before = Some(valid_before);
    }

    fn valid_after(&self) -> Option<NonZeroU64> {
        self.valid_after
    }

    fn set_valid_after(&mut self, valid_after: NonZeroU64) {
        self.valid_after = Some(valid_after);
    }

    fn fee_payer_signature(&self) -> Option<Signature> {
        self.fee_payer_signature
    }

    fn set_fee_payer_signature(&mut self, signature: Signature) {
        self.fee_payer_signature = Some(signature);
    }

    fn compute_sponsor_hash(&self, from: Address) -> Option<B256> {
        let tx = self.clone().build_aa().ok()?;
        Some(tx.fee_payer_signature_hash(from))
    }

    fn set_key_authorization(&mut self, key_authorization: SignedKeyAuthorization) {
        self.key_authorization = Some(key_authorization);
    }

    fn convert_create_to_call(&mut self) {
        if self.calls.is_empty() && self.inner.to.is_some_and(|to| to.is_create()) {
            let input = self.inner.input.input().cloned().unwrap_or_default();
            let value = self.inner.value.unwrap_or(U256::ZERO);
            self.calls.push(Call { to: TxKind::Create, value, input });
            self.inner.input = Default::default();
            self.inner.value = None;
            self.inner.to = None;
        }
    }

    fn clear_batch_to(&mut self) {
        if !self.calls.is_empty() {
            self.inner.to = None;
            self.inner.value = None;
        }
    }

    fn sign_with_access_key(
        mut self,
        provider: &impl Provider<TempoNetwork>,
        signer: &(impl Signer + Sync),
        wallet_address: Address,
        key_address: Address,
        key_authorization: Option<&SignedKeyAuthorization>,
    ) -> impl Future<Output = Result<Vec<u8>>> + Send {
        let auth = key_authorization.cloned();
        let provisioning_fut = provider.get_keychain_key(wallet_address, key_address);

        async move {
            if let Some(auth) = auth {
                let is_provisioned =
                    provisioning_fut.await.map(|info| info.keyId != Address::ZERO).unwrap_or(false);

                if !is_provisioned {
                    self.set_key_authorization(auth);
                }
            }

            let tempo_tx = self
                .build_aa()
                .map_err(|e| eyre::eyre!("failed to build Tempo AA transaction: {e}"))?;

            let sig_hash = tempo_tx.signature_hash();
            let signing_hash = KeychainSignature::signing_hash(sig_hash, wallet_address);
            let raw_sig = signer.sign_hash(&signing_hash).await?;

            let keychain_sig =
                KeychainSignature::new(wallet_address, PrimitiveSignature::Secp256k1(raw_sig));
            let aa_signed = tempo_tx.into_signed(TempoSignature::Keychain(keychain_sig));

            let mut buf = Vec::new();
            aa_signed.encode_2718(&mut buf);
            Ok(buf)
        }
    }
}

impl FoundryTransactionBuilder<QuantumNetwork> for QuantumTransactionRequest {
    fn reset_gas_limit(&mut self) {
        self.inner.gas = None;
    }

    fn quantum_sender(&self) -> Option<Address> {
        self.sender
    }

    fn set_quantum_sender(&mut self, sender: Address) {
        self.sender = Some(sender);
    }

    fn quantum_key_id(&self) -> Option<u32> {
        self.key_id
    }

    fn set_quantum_key_id(&mut self, key_id: u32) {
        self.key_id = Some(key_id);
    }

    fn quantum_nonce_key(&self) -> Option<U256> {
        self.nonce_key
    }

    fn set_quantum_nonce_key(&mut self, nonce_key: U256) {
        self.nonce_key = Some(nonce_key);
    }

    fn quantum_init_primary_pubkey(&self) -> Option<&Bytes> {
        self.init_primary_pubkey.as_ref()
    }

    fn set_quantum_init_primary_pubkey(&mut self, pubkey: Bytes) {
        self.init_primary_pubkey = Some(pubkey);
    }

    fn quantum_init_cosigner_pubkey(&self) -> Option<&Bytes> {
        self.init_cosigner_pubkey.as_ref()
    }

    fn set_quantum_init_cosigner_pubkey(&mut self, pubkey: Bytes) {
        self.init_cosigner_pubkey = Some(pubkey);
    }
}

#[cfg(test)]
mod tests {
    use alloy_rpc_types::TransactionRequest;

    use super::*;

    fn base_eth_request() -> TransactionRequest {
        TransactionRequest::default()
            .with_to(Address::ZERO)
            .with_nonce(42)
            .with_gas_limit(21_000)
            .with_max_fee_per_gas(50_000_000_000u128)
            .with_max_priority_fee_per_gas(1_000_000_000u128)
            .with_value(U256::from(1_000_000_000_000_000_000u128))
            .with_input(Bytes::from_static(b"hello"))
            .with_chain_id(1337)
    }

    #[test]
    fn quantum_request_signature_hash_matches_source_of_truth() {
        let request = QuantumWriteRequestV1 {
            sender: Address::ZERO,
            key_id: 0,
            nonce: 42,
            chain_id: 1337,
            max_priority_fee_per_gas: 1_000_000_000,
            max_fee_per_gas: 50_000_000_000,
            gas_limit: 21_000,
            call: QuantumSingleCall {
                kind: TxKind::Call(Address::ZERO),
                value: U256::from(1_000_000_000_000_000_000u128),
                input: Bytes::from_static(b"hello"),
            },
            access_list: Default::default(),
            bootstrap: None,
        };

        let mut buf = Vec::new();
        request.encode_body(&mut buf);
        assert_eq!(
            format!("{:#x}", keccak256(&buf)),
            "0xd039fbca7d51e653c90e8b84adb8fa6e30929dffa7eb41dbd6ec40594ce3ad4e"
        );
        assert_eq!(
            format!("{:#x}", request.signature_hash()),
            "0x909fe4db64c4605eb394b9de4d064bce0ab6d718b32050f00d80d7f525753b7d"
        );
    }

    #[test]
    fn quantum_request_requires_explicit_sender() {
        let err = QuantumWriteRequestV1::from_transaction_request(
            &base_eth_request(),
            QuantumWriteRequestInputsV1 {
                sender: Address::ZERO,
                key_id: 0,
                nonce_key: None,
                bootstrap: None,
            },
        )
        .unwrap_err();

        assert!(err.to_string().contains("explicit and non-zero"));
    }

    #[test]
    fn quantum_request_rejects_nonzero_nonce_key() {
        let err = QuantumWriteRequestV1::from_transaction_request(
            &base_eth_request(),
            QuantumWriteRequestInputsV1 {
                sender: Address::repeat_byte(0x11),
                key_id: 7,
                nonce_key: Some(U256::from(1u64)),
                bootstrap: None,
            },
        )
        .unwrap_err();

        assert_eq!(err.to_string(), "Quantum v1 only supports nonce_key = 0");
    }

    #[test]
    fn quantum_request_rejects_bootstrap_fields_outside_bootstrap() {
        let err = QuantumWriteRequestV1::from_transaction_request(
            &base_eth_request(),
            QuantumWriteRequestInputsV1 {
                sender: Address::repeat_byte(0x11),
                key_id: 0,
                nonce_key: None,
                bootstrap: Some(QuantumBootstrapFieldsV1 {
                    init_primary_pubkey: Bytes::from(vec![0x01, 0x02]),
                    init_cosigner_pubkey: None,
                }),
            },
        )
        .unwrap_err();

        assert_eq!(
            err.to_string(),
            "Quantum bootstrap fields are only valid for bootstrapKey() writes"
        );
    }

    #[test]
    fn quantum_request_rejects_bootstrap_cosigner_fields_in_v1() {
        let bootstrap_tx = TransactionRequest::default()
            .with_to(QUANTUM_KEYVAULT_ADDRESS)
            .with_nonce(0)
            .with_gas_limit(21_000)
            .with_max_fee_per_gas(1_000_000_000u128)
            .with_max_priority_fee_per_gas(1_000_000u128)
            .with_input(Bytes::from(QUANTUM_BOOTSTRAP_SELECTOR.to_vec()))
            .with_chain_id(1337);

        let err = QuantumWriteRequestV1::from_transaction_request(
            &bootstrap_tx,
            QuantumWriteRequestInputsV1 {
                sender: Address::repeat_byte(0x11),
                key_id: 0,
                nonce_key: None,
                bootstrap: Some(QuantumBootstrapFieldsV1 {
                    init_primary_pubkey: Bytes::from(vec![0x01, 0x02]),
                    init_cosigner_pubkey: Some(Bytes::from(vec![0x03, 0x04])),
                }),
            },
        )
        .unwrap_err();

        assert_eq!(err.to_string(), "Quantum bootstrap remains primary-only in v1");
    }

    #[test]
    fn quantum_request_rejects_bootstrap_with_nonzero_key_id() {
        let bootstrap_tx = TransactionRequest::default()
            .with_to(QUANTUM_KEYVAULT_ADDRESS)
            .with_nonce(0)
            .with_gas_limit(21_000)
            .with_max_fee_per_gas(1_000_000_000u128)
            .with_max_priority_fee_per_gas(1_000_000u128)
            .with_input(Bytes::from(QUANTUM_BOOTSTRAP_SELECTOR.to_vec()))
            .with_chain_id(1337);

        let err = QuantumWriteRequestV1::from_transaction_request(
            &bootstrap_tx,
            QuantumWriteRequestInputsV1 {
                sender: Address::repeat_byte(0x11),
                key_id: 7,
                nonce_key: None,
                bootstrap: None,
            },
        )
        .unwrap_err();

        assert_eq!(
            err.to_string(),
            "Quantum bootstrap requests must use key_id = 0 (account-lane primary); got key_id = 7"
        );
    }

    #[test]
    fn quantum_request_preserves_create_as_single_call() {
        let mut create_tx = TransactionRequest::default()
            .with_nonce(3)
            .with_gas_limit(120_000)
            .with_max_fee_per_gas(1_000_000_000u128)
            .with_max_priority_fee_per_gas(1_000_000u128)
            .with_input(Bytes::from(vec![0x60, 0x00, 0x60, 0x00]))
            .with_chain_id(1337);
        create_tx.to = Some(TxKind::Create);

        let request = QuantumWriteRequestV1::from_transaction_request(
            &create_tx,
            QuantumWriteRequestInputsV1 {
                sender: Address::repeat_byte(0x22),
                key_id: 9,
                nonce_key: None,
                bootstrap: None,
            },
        )
        .unwrap();

        assert!(matches!(request.call.kind, TxKind::Create));
        assert_eq!(request.call.input, Bytes::from(vec![0x60, 0x00, 0x60, 0x00]));
    }

    #[test]
    fn quantum_request_from_quantum_transaction_request_preserves_quantum_fields() {
        let request = QuantumTransactionRequest {
            inner: TransactionRequest::default()
                .with_chain_id(1337)
                .with_nonce(9)
                .with_to(Address::repeat_byte(0x44))
                .with_gas_limit(21_000)
                .with_max_fee_per_gas(10)
                .with_max_priority_fee_per_gas(1)
                .with_value(U256::from(5u64))
                .with_input(Bytes::from(vec![0xde, 0xad])),
            sender: Some(Address::repeat_byte(0x22)),
            key_id: Some(7),
            nonce_key: Some(U256::ZERO),
            init_primary_pubkey: None,
            init_cosigner_pubkey: None,
        };

        let write_request =
            QuantumWriteRequestV1::from_quantum_transaction_request(&request).unwrap();

        assert_eq!(write_request.sender, Address::repeat_byte(0x22));
        assert_eq!(write_request.key_id, 7);
        assert!(
            matches!(write_request.call.kind, TxKind::Call(to) if to == Address::repeat_byte(0x44))
        );
        assert_eq!(write_request.call.input, Bytes::from(vec![0xde, 0xad]));
    }

    #[test]
    fn quantum_request_from_quantum_transaction_request_rejects_nonzero_nonce_key() {
        let request = QuantumTransactionRequest {
            inner: TransactionRequest::default()
                .with_chain_id(1337)
                .with_nonce(9)
                .with_to(Address::repeat_byte(0x44))
                .with_gas_limit(21_000)
                .with_max_fee_per_gas(10)
                .with_max_priority_fee_per_gas(1),
            sender: Some(Address::repeat_byte(0x22)),
            key_id: Some(7),
            nonce_key: Some(U256::from(1u64)),
            init_primary_pubkey: None,
            init_cosigner_pubkey: None,
        };

        let err = QuantumWriteRequestV1::from_quantum_transaction_request(&request).unwrap_err();
        assert_eq!(err.to_string(), "Quantum v1 only supports nonce_key = 0");
    }
}
