use std::{path::PathBuf, str::FromStr, time::Duration};

use alloy_consensus::{SignableTransaction, Signed};
use alloy_ens::NameOrAddress;
use alloy_network::{Ethereum, EthereumWallet, Network};
use alloy_primitives::{Address, U256, hex};
use alloy_provider::{Provider, ProviderBuilder as AlloyProviderBuilder};
use alloy_signer::{Signature, Signer};
use clap::Parser;
use eyre::{Result, eyre};
use foundry_cli::{opts::TransactionOpts, utils::LoadConfig};
use foundry_common::{
    DetachedCosigner, FoundryTransactionBuilder, QUANTUM_ADD_KEY_SELECTOR,
    QUANTUM_BOOTSTRAP_SELECTOR, QUANTUM_KEYVAULT_ADDRESS, QUANTUM_LIFECYCLE_GAS_FLOOR,
    QUANTUM_REMOVE_KEY_SELECTOR, QUANTUM_SEND_LIFECYCLE_REJECTION_MESSAGE,
    QUANTUM_UPDATE_KEY_AUTH_SELECTOR, derive_primary_pubkey,
    fmt::{UIfmt, UIfmtReceiptExt},
    parse_seed_file,
    provider::ProviderBuilder,
    sign_quantum_transaction_request_with_cosigner,
};
use foundry_primitives::QuantumNetwork;
use foundry_wallets::{TempoAccessKeyConfig, WalletSigner};
use tempo_alloy::TempoNetwork;

use crate::{
    cmd::tip20::iso4217_warning_message,
    tx::{self, CastTxBuilder, CastTxSender, SendTxOpts},
};
use tempo_contracts::precompiles::{TIP20_FACTORY_ADDRESS, is_iso4217_currency};

/// CLI arguments for `cast send`.
#[derive(Debug, Parser)]
pub struct SendTxArgs {
    /// The destination of the transaction.
    ///
    /// If not provided, you must use cast send --create.
    #[arg(value_parser = NameOrAddress::from_str)]
    to: Option<NameOrAddress>,

    /// The signature of the function to call.
    sig: Option<String>,

    /// The arguments of the function to call.
    #[arg(allow_negative_numbers = true)]
    args: Vec<String>,

    /// Raw hex-encoded data for the transaction. Used instead of \[SIG\] and \[ARGS\].
    #[arg(
        long,
        conflicts_with_all = &["sig", "args"]
    )]
    data: Option<String>,

    #[command(flatten)]
    send_tx: SendTxOpts,

    #[command(subcommand)]
    command: Option<SendTxSubcommands>,

    /// Send via `eth_sendTransaction` using the `--from` argument or $ETH_FROM as sender
    #[arg(long, requires = "from")]
    unlocked: bool,

    /// Skip confirmation prompts (e.g. non-ISO 4217 currency warnings).
    #[arg(long)]
    force: bool,

    #[command(flatten)]
    tx: TransactionOpts,

    /// The path of blob data to be sent.
    #[arg(
        long,
        value_name = "BLOB_DATA_PATH",
        conflicts_with = "legacy",
        requires = "blob",
        help_heading = "Transaction options"
    )]
    path: Option<PathBuf>,
}

#[derive(Debug, Parser)]
pub enum SendTxSubcommands {
    /// Use to deploy raw contract bytecode.
    #[command(name = "--create")]
    Create {
        /// The bytecode of the contract to deploy.
        code: String,

        /// The signature of the function to call.
        sig: Option<String>,

        /// The arguments of the function to call.
        #[arg(allow_negative_numbers = true)]
        args: Vec<String>,
    },
}

impl SendTxArgs {
    pub async fn run(self) -> Result<()> {
        if self.tx.quantum.is_quantum() {
            return self.run_quantum().await;
        }

        // Resolve the signer early so we know if it's a Tempo access key.
        let (signer, tempo_access_key) = self.send_tx.eth.wallet.maybe_signer().await?;

        if tempo_access_key.is_some() || self.tx.tempo.is_tempo() {
            self.run_generic::<TempoNetwork>(signer, tempo_access_key).await
        } else {
            self.run_generic::<Ethereum>(signer, None).await
        }
    }

    async fn run_quantum(self) -> Result<()> {
        let Self { to, mut sig, args, data, send_tx, command, unlocked, force: _, mut tx, path } =
            self;

        if unlocked {
            return Err(eyre!("the Quantum adapter path does not support --unlocked"));
        }
        if send_tx.browser.browser {
            return Err(eyre!("the Quantum adapter path does not support browser signing"));
        }
        if tx.tempo.is_tempo() {
            return Err(eyre!("Quantum and Tempo options cannot be combined"));
        }
        if command.is_some() {
            return Err(eyre!("the Quantum adapter path only supports cast send-style call flows"));
        }
        if path.is_some() {
            return Err(eyre!("the Quantum adapter path does not support blob data"));
        }
        if let Some(data) = data {
            sig = Some(data);
        }

        let sender = tx
            .quantum
            .sender
            .ok_or_else(|| eyre!("--quantum.sender is required for Quantum writes"))?;
        validate_quantum_sender(send_tx.eth.wallet.from, sender)?;
        let seed_path =
            tx.quantum.primary_seed_file.as_ref().ok_or_else(|| {
                eyre!("--quantum.primary-seed-file is required for Quantum writes")
            })?;
        let primary_seed = parse_seed_file(seed_path)?;

        // Quantum v1 does not carry EIP-7702 authorization lists in the signed
        // envelope (`QuantumTxEnvelope::authorization_list()` returns `None`).
        // Reject `--auth` explicitly so callers do not believe a 7702 auth is
        // being broadcast when it would be silently dropped.
        if !tx.auth.is_empty() {
            return Err(eyre!(
                "the Quantum adapter path does not support EIP-7702 `--auth`; the v1 envelope does not carry authorization lists"
            ));
        }

        let config = send_tx.eth.load_config()?;
        let provider = ProviderBuilder::<QuantumNetwork>::from_config(&config)?.build()?;

        if let Some(interval) = send_tx.poll_interval {
            provider.client().set_poll_interval(Duration::from_secs(interval));
        }

        // Resolve the destination up front so the lifecycle fence and bootstrap
        // gas-floor block observe the true destination address, not an
        // unresolved name/ENS target. A name that resolves to the KeyVault
        // would otherwise slip past the literal-address check below.
        let resolved_to = match to {
            Some(to) => Some(to.resolve(&provider).await?),
            None => None,
        };

        // Fail closed before any RPC simulation: ordinary `cast send` must not accept
        // unsupported KeyVault lifecycle selectors (addKey / removeKey / updateKeyAuth).
        // Only `bootstrapKey()` is supported from this path in v1.
        let destination_is_keyvault = resolved_to == Some(QUANTUM_KEYVAULT_ADDRESS);
        if destination_is_keyvault && quantum_input_is_unsupported_lifecycle(sig.as_deref()) {
            return Err(eyre!(QUANTUM_SEND_LIFECYCLE_REJECTION_MESSAGE));
        }

        let is_bootstrap = destination_is_keyvault && quantum_input_is_bootstrap(sig.as_deref());

        // v1 bootstrap is primary-only: `cast quantum bootstrap` rejects any
        // cosigner artifact, and `cast send --quantum` must enforce the same
        // invariant so both sanctioned entry points produce the same envelope
        // shape. Cosigner attachment is a signature-side artifact, so this
        // cannot be caught by `QuantumWriteRequestV1::validate_v1`.
        if is_bootstrap && tx.quantum.cosigner_artifact.is_some() {
            return Err(eyre!(
                "Quantum v1 bootstrap is primary-only; cosigner artifact is not supported"
            ));
        }

        let cosigner = tx
            .quantum
            .cosigner_artifact
            .as_deref()
            .map(DetachedCosigner::from_artifact_file)
            .transpose()?;

        if is_bootstrap {
            if tx.quantum.init_primary_pubkey.is_none() {
                tx.quantum.init_primary_pubkey = Some(derive_primary_pubkey(primary_seed));
            }
            // Bootstrap/lifecycle calls cannot be simulated via `eth_estimateGas` because
            // the validator-published bootstrap transient state is absent. Apply the fixed
            // lifecycle gas floor when the caller did not override it, mirroring
            // `quantum-send-tx`'s LIFECYCLE_GAS_FLOOR.
            if tx.gas_limit.is_none() {
                tx.gas_limit = Some(U256::from(QUANTUM_LIFECYCLE_GAS_FLOOR));
            }
        }

        let builder = CastTxBuilder::new(&provider, tx.clone(), &config)
            .await?
            .with_to(resolved_to.map(NameOrAddress::Address))
            .await?
            .with_code_sig_and_args(None, sig, args)
            .await?;

        let (tx_request, _) = builder.build(sender).await?;
        let payload =
            sign_quantum_transaction_request_with_cosigner(tx_request, primary_seed, cosigner)?;

        let timeout = send_tx.timeout.unwrap_or(config.transaction_timeout);
        let cast = CastTxSender::new(&provider);
        let pending = cast.send_raw(&payload.raw_transaction).await?;
        let tx_hash = *pending.inner().tx_hash();
        cast.print_tx_result(tx_hash, send_tx.cast_async, send_tx.confirmations, timeout).await
    }

    pub async fn run_generic<N: Network>(
        self,
        pre_resolved_signer: Option<WalletSigner>,
        access_key: Option<TempoAccessKeyConfig>,
    ) -> Result<()>
    where
        N::TxEnvelope: From<Signed<N::UnsignedTx>>,
        N::UnsignedTx: SignableTransaction<Signature>,
        N::TransactionRequest: FoundryTransactionBuilder<N>,
        N::ReceiptResponse: UIfmt + UIfmtReceiptExt,
    {
        let Self { to, mut sig, mut args, data, send_tx, mut tx, command, unlocked, force, path } =
            self;

        let print_sponsor_hash = tx.tempo.print_sponsor_hash;
        let sponsor_signature = tx.tempo.sponsor_signature;

        let blob_data = if let Some(path) = path { Some(std::fs::read(path)?) } else { None };

        if let Some(data) = data {
            sig = Some(data);
        }

        let code = if let Some(SendTxSubcommands::Create {
            code,
            sig: constructor_sig,
            args: constructor_args,
        }) = command
        {
            // ensure we don't violate settings for transactions that can't be CREATE: 7702 and 4844
            // which require mandatory target
            if to.is_none() && !tx.auth.is_empty() {
                return Err(eyre!(
                    "EIP-7702 transactions can't be CREATE transactions and require a destination address"
                ));
            }
            // ensure we don't violate settings for transactions that can't be CREATE: 7702 and 4844
            // which require mandatory target
            if to.is_none() && blob_data.is_some() {
                return Err(eyre!(
                    "EIP-4844 transactions can't be CREATE transactions and require a destination address"
                ));
            }

            sig = constructor_sig;
            args = constructor_args;
            Some(code)
        } else {
            None
        };

        // Validate ISO 4217 currency code for TIP20Factory createToken calls.
        if let Some(ref to_addr) = to {
            let is_factory = match to_addr {
                NameOrAddress::Address(addr) => *addr == TIP20_FACTORY_ADDRESS,
                NameOrAddress::Name(name) => {
                    Address::from_str(name).ok() == Some(TIP20_FACTORY_ADDRESS)
                }
            };

            if !force
                && is_factory
                && let Some(ref sig_str) = sig
                && sig_str.starts_with("createToken")
                && let Some(currency) = args.get(2)
                && !is_iso4217_currency(currency)
            {
                sh_warn!("{}", iso4217_warning_message(currency))?;
                let response: String = foundry_common::prompt!("\nContinue anyway? [y/N] ")?;
                if !matches!(response.trim(), "y" | "Y") {
                    sh_println!("Aborted.")?;
                    return Ok(());
                }
            }
        }

        let config = send_tx.eth.load_config()?;
        let provider = ProviderBuilder::<N>::from_config(&config)?.build()?;

        if let Some(interval) = send_tx.poll_interval {
            provider.client().set_poll_interval(Duration::from_secs(interval))
        }

        // Inject access key ID into TempoOpts so it's set before gas estimation.
        if let Some(ref ak) = access_key {
            tx.tempo.key_id = Some(ak.key_address);
        }

        let builder = CastTxBuilder::new(&provider, tx, &config)
            .await?
            .with_to(to)
            .await?
            .with_code_sig_and_args(code, sig, args)
            .await?
            .with_blob_data(blob_data)?;

        // If --tempo.print-sponsor-hash was passed, build the tx, print the hash, and exit.
        if print_sponsor_hash {
            // Use the pre-resolved signer to derive the actual sender address, since the
            // sponsor hash commits to the sender.
            let signer = pre_resolved_signer.as_ref().ok_or_else(|| {
                eyre!("--tempo.print-sponsor-hash requires a signer (e.g. --private-key)")
            })?;
            let from = signer.address();
            let (tx, _) = builder.build(from).await?;
            let hash = tx
                .compute_sponsor_hash(from)
                .ok_or_else(|| eyre!("This network does not support sponsored transactions"))?;
            sh_println!("{hash:?}")?;
            return Ok(());
        }

        let timeout = send_tx.timeout.unwrap_or(config.transaction_timeout);

        // Launch browser signer if `--browser` flag is set
        let browser = send_tx.browser.run::<N>().await?;

        // Case 1:
        // Default to sending via eth_sendTransaction if the --unlocked flag is passed.
        // This should be the only way this RPC method is used as it requires a local node
        // or remote RPC with unlocked accounts.
        if unlocked && browser.is_none() {
            // only check current chain id if it was specified in the config
            if let Some(config_chain) = config.chain {
                let current_chain_id = provider.get_chain_id().await?;
                let config_chain_id = config_chain.id();
                // switch chain if current chain id is not the same as the one specified in the
                // config
                if config_chain_id != current_chain_id {
                    sh_warn!("Switching to chain {}", config_chain)?;
                    provider
                        .raw_request::<_, ()>(
                            "wallet_switchEthereumChain".into(),
                            [serde_json::json!({
                                "chainId": format!("0x{:x}", config_chain_id),
                            })],
                        )
                        .await?;
                }
            }

            let (tx, _) = builder.build(config.sender).await?;

            cast_send(
                provider,
                tx,
                send_tx.cast_async,
                send_tx.sync,
                send_tx.confirmations,
                timeout,
            )
            .await
        // Case 2:
        // Browser wallet signs and sends the transaction in one step.
        } else if let Some(browser) = browser {
            let (tx_request, _) = builder.build(browser.address()).await?;
            let tx_hash = browser.send_transaction_via_browser(tx_request).await?;

            let cast = CastTxSender::new(&provider);
            cast.print_tx_result(tx_hash, send_tx.cast_async, send_tx.confirmations, timeout).await
        // Case 3:
        // Tempo access key (keychain) signing. Uses `sign_with_access_key` which
        // handles the provisioning check and embeds `key_authorization` when needed.
        } else if let Some(ak) = access_key {
            let signer = match pre_resolved_signer {
                Some(s) => s,
                None => send_tx.eth.wallet.signer().await?,
            };
            let from = ak.wallet_address;

            let (tx_request, _) = builder.build(from).await?;

            let raw_tx = tx_request
                .sign_with_access_key(
                    &provider,
                    &signer,
                    ak.wallet_address,
                    ak.key_address,
                    ak.key_authorization.as_ref(),
                )
                .await?;

            let tx_hash = *provider.send_raw_transaction(&raw_tx).await?.tx_hash();

            let cast = CastTxSender::new(&provider);
            cast.print_tx_result(tx_hash, send_tx.cast_async, send_tx.confirmations, timeout).await
        // Case 4:
        // An option to use a local signer was provided.
        // If we cannot successfully instantiate a local signer, then we will assume we don't have
        // enough information to sign and we must bail.
        } else {
            let signer = match pre_resolved_signer {
                Some(s) => s,
                None => send_tx.eth.wallet.signer().await?,
            };
            let from = signer.address();

            tx::validate_from_address(send_tx.eth.wallet.from, from)?;

            let (mut tx_request, _) = builder.build(&signer).await?;

            // Apply sponsor signature after gas estimation so the estimate is
            // consistent with what `--tempo.print-sponsor-hash` computes.
            if let Some(sig) = sponsor_signature {
                tx_request.set_fee_payer_signature(sig);
            }

            let wallet = EthereumWallet::from(signer);
            let provider = AlloyProviderBuilder::<_, _, N>::default()
                .wallet(wallet)
                .connect_provider(&provider);

            cast_send(
                provider,
                tx_request,
                send_tx.cast_async,
                send_tx.sync,
                send_tx.confirmations,
                timeout,
            )
            .await
        }
    }
}

fn validate_quantum_sender(cli_from: Option<Address>, quantum_sender: Address) -> Result<()> {
    if let Some(from) = cli_from
        && from != quantum_sender
    {
        eyre::bail!("--from must match --quantum.sender when using the Quantum adapter path")
    }

    Ok(())
}

fn strip_hex_prefix(value: &str) -> &str {
    value.strip_prefix("0x").or_else(|| value.strip_prefix("0X")).unwrap_or(value)
}

fn quantum_input_is_bootstrap(input: Option<&str>) -> bool {
    let Some(input) = input else { return false };
    if input.starts_with("bootstrapKey(") {
        return true;
    }
    let hex_body = strip_hex_prefix(input.trim()).to_ascii_lowercase();
    hex_body.starts_with(&hex::encode(QUANTUM_BOOTSTRAP_SELECTOR))
}

fn quantum_input_is_unsupported_lifecycle(input: Option<&str>) -> bool {
    let Some(input) = input else { return false };
    let trimmed = input.trim();
    if trimmed.starts_with("addKey(")
        || trimmed.starts_with("removeKey(")
        || trimmed.starts_with("updateKeyAuth(")
    {
        return true;
    }
    let hex_body = strip_hex_prefix(trimmed).to_ascii_lowercase();
    let add = hex::encode(QUANTUM_ADD_KEY_SELECTOR);
    let remove = hex::encode(QUANTUM_REMOVE_KEY_SELECTOR);
    let update = hex::encode(QUANTUM_UPDATE_KEY_AUTH_SELECTOR);
    hex_body.starts_with(&add) || hex_body.starts_with(&remove) || hex_body.starts_with(&update)
}

pub(crate) async fn cast_send<N: Network, P: Provider<N>>(
    provider: P,
    tx: N::TransactionRequest,
    cast_async: bool,
    sync: bool,
    confs: u64,
    timeout: u64,
) -> Result<()>
where
    N::TransactionRequest: FoundryTransactionBuilder<N>,
    N::ReceiptResponse: UIfmt + UIfmtReceiptExt,
{
    let cast = CastTxSender::new(provider);

    if sync {
        // Send transaction and wait for receipt synchronously
        let receipt = cast.send_sync(tx).await?;
        sh_println!("{receipt}")?;
    } else {
        let pending_tx = cast.send(tx).await?;
        let tx_hash = *pending_tx.inner().tx_hash();
        cast.print_tx_result(tx_hash, cast_async, confs, timeout).await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::SendTxArgs;

    #[test]
    fn send_command_clap_shape_is_valid() {
        SendTxArgs::command().debug_assert();
    }
}
