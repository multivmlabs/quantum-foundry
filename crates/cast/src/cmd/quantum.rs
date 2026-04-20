use std::{path::PathBuf, time::Duration};

use alloy_ens::NameOrAddress;
use alloy_primitives::{Address, Bytes, U256, hex};
use alloy_provider::Provider;
use clap::{Parser, Subcommand};
use eyre::{Result, eyre};
use foundry_cli::{opts::TransactionOpts, utils::LoadConfig};
use foundry_common::{
    DetachedCosigner, QUANTUM_KEYVAULT_ADDRESS, QUANTUM_LIFECYCLE_GAS_FLOOR, QuantumAddKeyInputs,
    QuantumUpdateKeyAuthInputs, derive_primary_pubkey, encode_add_key_calldata,
    encode_bootstrap_calldata, encode_remove_key_calldata, encode_update_key_auth_calldata,
    parse_seed_file, provider::ProviderBuilder, sign_quantum_transaction_request_with_cosigner,
};
use foundry_primitives::QuantumNetwork;

use crate::tx::{CastTxBuilder, CastTxSender, SendTxOpts};

/// `cast quantum <subcommand>` — KeyVault lifecycle UX for Quantum.
///
/// These commands build native `0x7A` envelopes through the shared Quantum
/// signing pipeline used by `cast send --quantum`. Lifecycle writes are
/// intentionally separate from ordinary `cast send` so the CLI makes the
/// distinction between the **auth lane** (the `key_id` that signs) and the
/// **target key** (the key entry being mutated) impossible to confuse.
///
/// Lifecycle writes use the fixed `QUANTUM_LIFECYCLE_GAS_FLOOR` by default
/// because the validator-published transient state cannot be simulated by
/// `eth_estimateGas`.
#[derive(Debug, Parser)]
pub struct QuantumArgs {
    #[command(subcommand)]
    pub command: QuantumSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum QuantumSubcommand {
    /// Bootstrap a fresh Quantum account via `KeyVault.bootstrapKey()`.
    ///
    /// Bootstrap is primary-only in v1: exactly the ML-DSA key derived from the
    /// provided seed is registered on lane `key_id = 0`. A detached cosigner
    /// artifact is not accepted for bootstrap.
    Bootstrap(BootstrapArgs),

    /// Add a key to an existing Quantum account via `KeyVault.addKey(...)`.
    ///
    /// The auth lane used to sign (`--auth-key-id`) is explicit and separate
    /// from the target lane being added (`--target-key-id`).
    #[command(name = "add-key")]
    AddKey(AddKeyArgs),

    /// Remove a key from an existing Quantum account via `KeyVault.removeKey(uint32)`.
    #[command(name = "remove-key")]
    RemoveKey(RemoveKeyArgs),

    /// Update authorization on an existing key via `KeyVault.updateKeyAuth(...)`.
    #[command(name = "update-key-auth")]
    UpdateKeyAuth(UpdateKeyAuthArgs),
}

/// Common inputs shared by every lifecycle write.
#[derive(Debug, Parser, Clone)]
pub struct LifecycleCommonOpts {
    /// Quantum sender / account-lane address being mutated.
    #[arg(long = "sender", value_name = "ADDRESS")]
    pub sender: Address,

    /// Auth-lane `key_id` used to sign the lifecycle transaction.
    ///
    /// Distinct from the target key being added, removed, or updated.
    /// Defaults to `0`.
    #[arg(long = "auth-key-id", value_name = "KEY_ID", default_value_t = 0)]
    pub auth_key_id: u32,

    /// Path to the canonical v1 ML-DSA signer seed file.
    #[arg(long = "primary-seed-file", value_name = "PATH")]
    pub primary_seed_file: PathBuf,

    /// Optional v1 detached cosigner artifact JSON.
    ///
    /// Supported schemes are `p256` and `ecdsa`. The artifact's `signing_hash`
    /// must match the fork-computed Quantum signing hash byte-for-byte.
    #[arg(long = "cosigner-artifact", value_name = "PATH")]
    pub cosigner_artifact: Option<PathBuf>,

    /// Shared `cast send` flow options (rpc, wallet, confirmations, timeouts).
    #[command(flatten)]
    pub send_tx: SendTxOpts,

    /// Shared transaction options (gas limit, fees, nonce, value, access list).
    ///
    /// Value is ignored for KeyVault lifecycle writes.
    #[command(flatten)]
    pub tx: TransactionOpts,
}

#[derive(Debug, Parser)]
pub struct BootstrapArgs {
    #[command(flatten)]
    pub common: LifecycleCommonOpts,
}

#[derive(Debug, Parser)]
pub struct AddKeyArgs {
    /// Target key ID slot being added.
    #[arg(long = "target-key-id", value_name = "KEY_ID")]
    pub target_key_id: u32,

    /// New key public-key bytes (hex).
    #[arg(long = "pubkey", value_name = "HEX_BYTES")]
    pub pubkey: Bytes,

    /// New key scheme identifier (1=ML-DSA-44, 2=P256, 3=ECDSA).
    #[arg(long = "scheme", value_name = "U8")]
    pub scheme: u8,

    /// Auth-proof bytes accompanying the new key (hex).
    #[arg(long = "auth-proof", value_name = "HEX_BYTES", default_value = "0x")]
    pub auth_proof: Bytes,

    /// Required cosigner scheme for the new key (0=none, 2=P256, 3=ECDSA).
    #[arg(long = "cosigner-scheme", value_name = "U8", default_value_t = 0)]
    pub cosigner_scheme: u8,

    /// Scoped-permissions flag byte.
    #[arg(long = "scoped-permissions", value_name = "U8", default_value_t = 0)]
    pub scoped_permissions: u8,

    /// Scope-data bytes (hex, ABI-opaque; empty by default).
    #[arg(long = "scope-data", value_name = "HEX_BYTES", default_value = "0x")]
    pub scope_data: Bytes,

    #[command(flatten)]
    pub common: LifecycleCommonOpts,
}

#[derive(Debug, Parser)]
pub struct RemoveKeyArgs {
    /// Target key ID slot being removed.
    #[arg(long = "target-key-id", value_name = "KEY_ID")]
    pub target_key_id: u32,

    #[command(flatten)]
    pub common: LifecycleCommonOpts,
}

#[derive(Debug, Parser)]
pub struct UpdateKeyAuthArgs {
    /// Target key ID slot whose authorization is being updated.
    #[arg(long = "target-key-id", value_name = "KEY_ID")]
    pub target_key_id: u32,

    /// Auth-proof bytes (hex).
    #[arg(long = "auth-proof", value_name = "HEX_BYTES", default_value = "0x")]
    pub auth_proof: Bytes,

    /// Key scheme identifier (1=ML-DSA-44, 2=P256, 3=ECDSA).
    #[arg(long = "scheme", value_name = "U8")]
    pub scheme: u8,

    /// Scope-data bytes (hex, ABI-opaque; empty by default).
    #[arg(long = "scope-data", value_name = "HEX_BYTES", default_value = "0x")]
    pub scope_data: Bytes,

    /// Scoped-permissions flag byte.
    #[arg(long = "scoped-permissions", value_name = "U8", default_value_t = 0)]
    pub scoped_permissions: u8,

    #[command(flatten)]
    pub common: LifecycleCommonOpts,
}

impl QuantumArgs {
    pub async fn run(self) -> Result<()> {
        match self.command {
            QuantumSubcommand::Bootstrap(args) => run_bootstrap(args).await,
            QuantumSubcommand::AddKey(args) => run_add_key(args).await,
            QuantumSubcommand::RemoveKey(args) => run_remove_key(args).await,
            QuantumSubcommand::UpdateKeyAuth(args) => run_update_key_auth(args).await,
        }
    }
}

async fn run_bootstrap(args: BootstrapArgs) -> Result<()> {
    let BootstrapArgs { mut common } = args;
    // v1 bootstrap is primary-only: reject cosigner supplied via either the
    // lifecycle-specific `--cosigner-artifact` or the shared
    // `--quantum.cosigner-artifact` flag.
    if common.cosigner_artifact.is_some() || common.tx.quantum.cosigner_artifact.is_some() {
        return Err(eyre!(
            "Quantum v1 bootstrap is primary-only; cosigner artifact is not supported"
        ));
    }
    if common.auth_key_id != 0 {
        return Err(eyre!(
            "bootstrap requires --auth-key-id 0; lane 0 is the only valid bootstrap lane in v1"
        ));
    }

    // Populate the bootstrap `init_primary_pubkey` field if the caller did not
    // provide one. Mirrors `cast send` bootstrap behavior. If the caller did
    // provide one, it must match the key derived from the signing seed — a
    // mismatch would initialize a key the caller cannot sign with.
    let primary_seed = parse_seed_file(&common.primary_seed_file)?;
    let derived = derive_primary_pubkey(primary_seed);
    match common.tx.quantum.init_primary_pubkey.as_ref() {
        None => common.tx.quantum.init_primary_pubkey = Some(derived),
        Some(provided) if provided != &derived => {
            return Err(eyre!(
                "--quantum.init-primary-pubkey does not match the public key derived from --primary-seed-file; omit the flag to auto-fill"
            ));
        }
        Some(_) => {}
    }

    submit_lifecycle(common, encode_bootstrap_calldata(), true).await
}

async fn run_add_key(args: AddKeyArgs) -> Result<()> {
    let AddKeyArgs {
        target_key_id,
        pubkey,
        scheme,
        auth_proof,
        cosigner_scheme,
        scoped_permissions,
        scope_data,
        common,
    } = args;
    let calldata = encode_add_key_calldata(&QuantumAddKeyInputs {
        target_key_id,
        pubkey,
        scheme,
        auth_proof,
        cosigner_scheme,
        scoped_permissions,
        scope_data,
    });
    submit_lifecycle(common, calldata, false).await
}

async fn run_remove_key(args: RemoveKeyArgs) -> Result<()> {
    let RemoveKeyArgs { target_key_id, common } = args;
    let calldata = encode_remove_key_calldata(target_key_id);
    submit_lifecycle(common, calldata, false).await
}

async fn run_update_key_auth(args: UpdateKeyAuthArgs) -> Result<()> {
    let UpdateKeyAuthArgs {
        target_key_id,
        auth_proof,
        scheme,
        scope_data,
        scoped_permissions,
        common,
    } = args;
    let calldata = encode_update_key_auth_calldata(&QuantumUpdateKeyAuthInputs {
        target_key_id,
        auth_proof,
        scheme,
        scope_data,
        scoped_permissions,
    });
    submit_lifecycle(common, calldata, false).await
}

/// Shared lifecycle submission path: reuse `CastTxBuilder` to fill fees and
/// nonce, then sign via the fork's ML-DSA signer and broadcast via
/// `eth_sendRawTransaction`. Bypasses the ordinary-send lifecycle guard
/// intentionally — the caller has already built the canonical KeyVault
/// calldata via the shared lifecycle core.
async fn submit_lifecycle(
    mut common: LifecycleCommonOpts,
    calldata: Bytes,
    is_bootstrap: bool,
) -> Result<()> {
    // Fail closed on a mismatched `--from`. `cast send --quantum` and
    // `forge create --quantum` both reject a `--from` that disagrees with the
    // quantum sender; `cast quantum` must enforce the same invariant so an
    // operator cannot think they are acting as one account while the command
    // actually signs for another.
    if let Some(from) = common.send_tx.eth.wallet.from
        && from != common.sender
    {
        return Err(eyre!(
            "--from must match --sender when using the Quantum lifecycle path; got {} and {}",
            from,
            common.sender,
        ));
    }

    // Set the quantum sender on the shared TransactionOpts so the wallet glue
    // finds it. The sender is the account being mutated, on whose behalf the
    // ML-DSA signer produces the primary signature.
    match common.tx.quantum.sender {
        None => common.tx.quantum.sender = Some(common.sender),
        Some(quantum_sender) if quantum_sender != common.sender => {
            return Err(eyre!(
                "--sender and --quantum.sender must match; got {} and {}",
                common.sender,
                quantum_sender,
            ));
        }
        Some(_) => {}
    }
    match common.tx.quantum.key_id {
        None => common.tx.quantum.key_id = Some(common.auth_key_id),
        Some(quantum_key_id) if quantum_key_id != common.auth_key_id => {
            return Err(eyre!(
                "--auth-key-id and --quantum.key-id must match; got {} and {}",
                common.auth_key_id,
                quantum_key_id,
            ));
        }
        Some(_) => {}
    }
    if common.tx.quantum.primary_seed_file.is_none() {
        common.tx.quantum.primary_seed_file = Some(common.primary_seed_file.clone());
    }
    if common.tx.quantum.cosigner_artifact.is_none()
        && let Some(ref p) = common.cosigner_artifact
    {
        common.tx.quantum.cosigner_artifact = Some(p.clone());
    }

    // The `cast quantum` help contract says value is ignored for KeyVault
    // lifecycle writes. Reject non-zero `--value` explicitly rather than
    // silently zero it: forwarding ETH to `bootstrapKey()`/`addKey()`/etc. is
    // almost always an operator mistake.
    if common.tx.value.is_some_and(|v| !v.is_zero()) {
        return Err(eyre!("KeyVault lifecycle writes do not accept `--value`; remove the flag"));
    }

    // Quantum v1 does not carry EIP-7702 authorization lists in the signed
    // 0x7a envelope. Reject `--auth` explicitly so callers do not believe a
    // 7702 auth is being broadcast when it would be silently dropped.
    if !common.tx.auth.is_empty() {
        return Err(eyre!(
            "the Quantum adapter path does not support EIP-7702 `--auth`; the v1 envelope does not carry authorization lists"
        ));
    }

    let primary_seed = parse_seed_file(&common.primary_seed_file)?;
    // Read the merged cosigner path so `--quantum.cosigner-artifact` and
    // `--cosigner-artifact` are both honored consistently.
    let cosigner = common
        .tx
        .quantum
        .cosigner_artifact
        .as_deref()
        .map(DetachedCosigner::from_artifact_file)
        .transpose()?;

    // KeyVault lifecycle writes cannot be simulated via `eth_estimateGas` because
    // the validator-published bootstrap transient state is absent. Apply the fixed
    // lifecycle gas floor when the caller did not override it.
    if common.tx.gas_limit.is_none() {
        common.tx.gas_limit = Some(U256::from(QUANTUM_LIFECYCLE_GAS_FLOOR));
    }

    let config = common.send_tx.eth.load_config()?;
    let provider = ProviderBuilder::<QuantumNetwork>::from_config(&config)?.build()?;

    if let Some(interval) = common.send_tx.poll_interval {
        provider.client().set_poll_interval(Duration::from_secs(interval));
    }

    // Destination is always the canonical KeyVault precompile address.
    let to = Some(NameOrAddress::Address(QUANTUM_KEYVAULT_ADDRESS));
    // Hex-encode calldata; `CastTxBuilder::with_code_sig_and_args` decodes raw hex
    // when the `sig` string starts with `0x` and no parentheses.
    let data_hex = format!("0x{}", hex::encode(&calldata));

    let builder = CastTxBuilder::new(&provider, common.tx.clone(), &config)
        .await?
        .with_to(to)
        .await?
        .with_code_sig_and_args(None, Some(data_hex), Vec::new())
        .await?;

    let (tx_request, _) = builder.build(common.sender).await?;
    let payload =
        sign_quantum_transaction_request_with_cosigner(tx_request, primary_seed, cosigner)?;

    // `is_bootstrap` is currently informational; the on-chain selector determines
    // the validator-side path. Retained so the caller's intent is explicit and
    // can be validated in the future.
    let _ = is_bootstrap;

    let timeout = common.send_tx.timeout.unwrap_or(config.transaction_timeout);
    let cast = CastTxSender::new(&provider);
    let pending = cast.send_raw(&payload.raw_transaction).await?;
    let tx_hash = *pending.inner().tx_hash();
    cast.print_tx_result(tx_hash, common.send_tx.cast_async, common.send_tx.confirmations, timeout)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn quantum_command_clap_shape_is_valid() {
        QuantumArgs::command().debug_assert();
    }

    #[test]
    fn bootstrap_args_parse() {
        let args = QuantumArgs::parse_from([
            "cast-quantum",
            "bootstrap",
            "--sender",
            "0x47872C3e8676384B80648D95bEaC2c0C348eF272",
            "--primary-seed-file",
            "/tmp/seed.hex",
            "--rpc-url",
            "http://localhost:18545",
        ]);
        assert!(matches!(args.command, QuantumSubcommand::Bootstrap(_)));
    }

    #[test]
    fn add_key_args_parse_with_scoped_permissions() {
        let args = QuantumArgs::parse_from([
            "cast-quantum",
            "add-key",
            "--sender",
            "0x47872C3e8676384B80648D95bEaC2c0C348eF272",
            "--primary-seed-file",
            "/tmp/seed.hex",
            "--target-key-id",
            "1",
            "--pubkey",
            "0xaabbcc",
            "--scheme",
            "2",
            "--scoped-permissions",
            "1",
            "--scope-data",
            "0x1234",
        ]);
        let QuantumSubcommand::AddKey(a) = args.command else { panic!("expected add-key") };
        assert_eq!(a.target_key_id, 1);
        assert_eq!(a.scheme, 2);
        assert_eq!(a.scoped_permissions, 1);
        assert_eq!(a.scope_data.as_ref(), &[0x12, 0x34]);
    }

    #[test]
    fn remove_key_args_parse() {
        let args = QuantumArgs::parse_from([
            "cast-quantum",
            "remove-key",
            "--sender",
            "0x47872C3e8676384B80648D95bEaC2c0C348eF272",
            "--primary-seed-file",
            "/tmp/seed.hex",
            "--target-key-id",
            "2",
        ]);
        let QuantumSubcommand::RemoveKey(r) = args.command else { panic!("expected remove-key") };
        assert_eq!(r.target_key_id, 2);
    }

    #[test]
    fn update_key_auth_args_parse() {
        let args = QuantumArgs::parse_from([
            "cast-quantum",
            "update-key-auth",
            "--sender",
            "0x47872C3e8676384B80648D95bEaC2c0C348eF272",
            "--primary-seed-file",
            "/tmp/seed.hex",
            "--target-key-id",
            "3",
            "--scheme",
            "1",
            "--auth-proof",
            "0x1122",
            "--scope-data",
            "0x",
            "--scoped-permissions",
            "2",
        ]);
        let QuantumSubcommand::UpdateKeyAuth(u) = args.command else {
            panic!("expected update-key-auth")
        };
        assert_eq!(u.target_key_id, 3);
        assert_eq!(u.scheme, 1);
        assert_eq!(u.auth_proof.as_ref(), &[0x11, 0x22]);
        assert_eq!(u.scoped_permissions, 2);
    }

    #[test]
    fn auth_key_id_defaults_to_zero_but_is_distinct_from_target_key_id() {
        let args = QuantumArgs::parse_from([
            "cast-quantum",
            "add-key",
            "--sender",
            "0x47872C3e8676384B80648D95bEaC2c0C348eF272",
            "--primary-seed-file",
            "/tmp/seed.hex",
            "--target-key-id",
            "5",
            "--pubkey",
            "0xaa",
            "--scheme",
            "2",
            "--auth-key-id",
            "0",
        ]);
        let QuantumSubcommand::AddKey(a) = args.command else { panic!("expected add-key") };
        assert_eq!(a.common.auth_key_id, 0);
        assert_eq!(a.target_key_id, 5);
    }
}
