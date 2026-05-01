use alloy_chains::Chain;
use alloy_primitives::ChainId;

/// Network selection, defaulting to Ethereum
#[derive(Clone, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum NetworkVariant {
    /// Ethereum (default)
    #[default]
    Ethereum,
    /// Optimism / OP-stack
    Optimism,
    /// Tempo
    Tempo,
    /// Quantum
    Quantum,
}

impl From<ChainId> for NetworkVariant {
    fn from(chain_id: ChainId) -> Self {
        let chain = Chain::from_id(chain_id);
        if chain.is_tempo() {
            Self::Tempo
        } else if chain.is_optimism() {
            Self::Optimism
        } else {
            // Quantum stays explicitly selected in v1; do not auto-infer it from chain ID here.
            Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quantum_network_variant() {
        assert_eq!(
            <NetworkVariant as clap::ValueEnum>::from_str("quantum", true).unwrap(),
            NetworkVariant::Quantum
        );
    }
}
