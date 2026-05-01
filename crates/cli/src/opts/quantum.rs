use std::path::PathBuf;

use alloy_primitives::Address;
use clap::Parser;

/// CLI options for the Phase 0 Quantum seam spike.
#[derive(Clone, Debug, Default, Parser)]
#[command(next_help_heading = "Quantum")]
pub struct QuantumOpts {
    /// Enable the Phase 0 native Quantum raw-transaction path.
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
    /// Defaults to `0` for the Phase 0 seam spike.
    #[arg(id = "quantum_key_id", long = "quantum.key-id", value_name = "KEY_ID")]
    pub key_id: Option<u32>,

    /// Path to the canonical Phase 0 ML-DSA signer seed file.
    ///
    /// The file must contain a single 32-byte seed as hex, with or without a `0x` prefix.
    #[arg(
        id = "quantum_primary_seed_file",
        long = "quantum.primary-seed-file",
        value_name = "PATH"
    )]
    pub primary_seed_file: Option<PathBuf>,
}

impl QuantumOpts {
    /// Returns `true` if any Quantum-specific option is set.
    pub fn is_quantum(&self) -> bool {
        self.enabled
            || self.sender.is_some()
            || self.key_id.is_some()
            || self.primary_seed_file.is_some()
    }

    /// Returns the resolved key ID for the Phase 0 seam.
    pub fn resolved_key_id(&self) -> u32 {
        self.key_id.unwrap_or(0)
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
        ])
        .unwrap();

        assert!(opts.is_quantum());
        assert_eq!(opts.resolved_key_id(), 7);
        assert_eq!(
            opts.primary_seed_file.as_deref(),
            Some(std::path::Path::new("./seed.hex"))
        );
    }
}
