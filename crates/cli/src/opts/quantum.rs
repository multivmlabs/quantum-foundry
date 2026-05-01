use std::path::PathBuf;

use alloy_network::Network;
use alloy_primitives::{Address, Bytes, U256};
use clap::Parser;
use foundry_common::FoundryTransactionBuilder;

/// CLI options for the v1 Quantum adapter path.
#[derive(Clone, Debug, Default, Parser)]
#[command(next_help_heading = "Quantum")]
pub struct QuantumOpts {
    /// Enable the explicit Quantum adapter path.
    ///
    /// This keeps Quantum selection explicit in v1 instead of inferring it from chain ID.
    #[arg(id = "quantum_enabled", long = "quantum")]
    pub enabled: bool,

    /// Explicit Quantum sender/account-lane address.
    ///
    /// Quantum writes never auto-derive the sender from the signing key.
    #[arg(id = "quantum_sender", long = "quantum.sender", value_name = "ADDRESS")]
    pub sender: Option<Address>,

    /// Quantum account-lane key ID.
    ///
    /// Defaults to `0` for ordinary v1 write flows.
    #[arg(id = "quantum_key_id", long = "quantum.key-id", value_name = "KEY_ID")]
    pub key_id: Option<u32>,

    /// Path to the canonical v1 ML-DSA signer seed file.
    ///
    /// The file must contain a single 32-byte seed as hex, with or without a `0x` prefix.
    #[arg(
        id = "quantum_primary_seed_file",
        long = "quantum.primary-seed-file",
        value_name = "PATH"
    )]
    pub primary_seed_file: Option<PathBuf>,

    /// Explicit bootstrap primary pubkey bytes for `bootstrapKey()`.
    ///
    /// When omitted, write flows that already have the ML-DSA seed file available may derive the
    /// primary pubkey automatically before signing.
    #[arg(
        id = "quantum_init_primary_pubkey",
        long = "quantum.init-primary-pubkey",
        value_name = "HEX_BYTES"
    )]
    pub init_primary_pubkey: Option<Bytes>,

    /// Explicit bootstrap cosigner pubkey bytes.
    ///
    /// Quantum v1 remains primary-only, so requests carrying this field fail closed during
    /// request validation.
    #[arg(
        id = "quantum_init_cosigner_pubkey",
        long = "quantum.init-cosigner-pubkey",
        value_name = "HEX_BYTES"
    )]
    pub init_cosigner_pubkey: Option<Bytes>,
}

impl QuantumOpts {
    /// Returns `true` if any Quantum-specific option is set.
    pub fn is_quantum(&self) -> bool {
        self.enabled
            || self.sender.is_some()
            || self.key_id.is_some()
            || self.primary_seed_file.is_some()
            || self.init_primary_pubkey.is_some()
            || self.init_cosigner_pubkey.is_some()
    }

    /// Returns the resolved key ID for the Phase 0 seam.
    pub fn resolved_key_id(&self) -> u32 {
        self.key_id.unwrap_or(0)
    }

    /// Applies Quantum-specific options to a transaction request.
    ///
    /// All setters are no-ops for non-Quantum networks, so this is safe to call unconditionally.
    pub fn apply<N: Network>(&self, tx: &mut N::TransactionRequest)
    where
        N::TransactionRequest: FoundryTransactionBuilder<N>,
    {
        if !self.is_quantum() {
            return;
        }

        if let Some(sender) = self.sender {
            tx.set_quantum_sender(sender);
        }

        tx.set_quantum_key_id(self.resolved_key_id());
        tx.set_quantum_nonce_key(U256::ZERO);

        if let Some(pubkey) = self.init_primary_pubkey.clone() {
            tx.set_quantum_init_primary_pubkey(pubkey);
        }

        if let Some(pubkey) = self.init_cosigner_pubkey.clone() {
            tx.set_quantum_init_cosigner_pubkey(pubkey);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_quantum_opts() {
        let opts = QuantumOpts::try_parse_from([
            "",
            "--quantum",
            "--quantum.sender",
            "0x000000000000000000000000000000000000dEaD",
            "--quantum.key-id",
            "7",
            "--quantum.primary-seed-file",
            "./seed.hex",
            "--quantum.init-primary-pubkey",
            "0x010203",
            "--quantum.init-cosigner-pubkey",
            "0x0405",
        ])
        .unwrap();

        assert!(opts.is_quantum());
        assert_eq!(opts.resolved_key_id(), 7);
        assert_eq!(opts.primary_seed_file.as_deref(), Some(std::path::Path::new("./seed.hex")));
        assert_eq!(opts.init_primary_pubkey, Some(Bytes::from(vec![0x01, 0x02, 0x03])));
        assert_eq!(opts.init_cosigner_pubkey, Some(Bytes::from(vec![0x04, 0x05])));
    }
}
