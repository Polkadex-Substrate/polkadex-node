// This file is part of Polkadex.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

//! # pallet-eth-bridge
//!
//! Handles the **Ethereum → Polkadex** leg of the WETH bridge.
//!
//! ## Bridge flow (Ethereum → Polkadex)
//!
//! 1. User calls `depositEth{value: N}(polkadexAccountId)` on `PolkadexBridge.sol`
//!    (Sepolia). The contract wraps ETH → WETH, locks it, and emits a `Deposit` event.
//! 2. The authorized relayer calls `submit_eth_header` with the finalized block header
//!    containing `receiptsRoot`.
//! 3. Anyone constructs an MPT receipt proof off-chain and calls `submit_deposit_proof`.
//!    The pallet verifies the proof, parses the `Deposit` event, and credits bridged
//!    WETH to the Polkadex recipient via the `BridgeAssets` callback.
//!
//! ## Trust model (v1 — trusted relayer)
//!
//! Block headers are submitted by a single authorized relayer. Governance can rotate
//! the relayer at any time. A future upgrade replaces the relayer with an Ethereum
//! beacon chain light client or a ZK proof of Ethereum finality.
//!
//! ## Polkadex → Ethereum
//!
//! Handled by `PolkadexBridge.sol` on the Ethereum side. Users call `withdraw()` there
//! with a BEEFY Merkle proof. This pallet is not involved in that direction.

#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;

mod mpt;
pub mod types;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

use types::{DepositProof, EthBlockHeader, TokenConfig, WithdrawalMessage};

/// Callback trait implemented by the runtime to mint and burn bridged tokens.
///
/// The pallet looks up the `polkadex_asset_id` from `TokenRegistry` before calling
/// these methods, so the runtime implementation only needs to delegate straight to
/// `pallet-assets` — no secondary mapping required.
///
/// For production runtimes use the built-in [`PalletAssetsBridge`] adapter.
/// For unit tests use the in-memory [`crate::mock::MockBridgeAssets`].
pub trait BridgeAssets<AccountId> {
    /// Mint `amount` (in Polkadex 12-decimal units) of the asset identified by
    /// `polkadex_asset_id` to `recipient`.
    /// Called on deposit: Ethereum → Polkadex.
    fn mint(
        polkadex_asset_id: u128,
        recipient: &AccountId,
        amount: u128,
    ) -> frame_support::pallet_prelude::DispatchResult;

    /// Burn `amount` (in Polkadex 12-decimal units) of the asset identified by
    /// `polkadex_asset_id` from `from`.
    /// Called on withdrawal: Polkadex → Ethereum.
    fn burn(
        polkadex_asset_id: u128,
        from: &AccountId,
        amount: u128,
    ) -> frame_support::pallet_prelude::DispatchResult;
}

/// No-op implementation used in tests that do not exercise asset operations.
impl<AccountId> BridgeAssets<AccountId> for () {
    fn mint(_: u128, _: &AccountId, _: u128) -> frame_support::pallet_prelude::DispatchResult { Ok(()) }
    fn burn(_: u128, _: &AccountId, _: u128) -> frame_support::pallet_prelude::DispatchResult { Ok(()) }
}

/// Production-ready `BridgeAssets` implementation backed by `pallet-assets`.
///
/// Wire it in the runtime config:
/// ```ignore
/// impl pallet_eth_bridge::Config for Runtime {
///     type BridgeAssets = pallet_eth_bridge::PalletAssetsBridge<pallet_assets::Pallet<Runtime>>;
///     ...
/// }
/// ```
pub struct PalletAssetsBridge<Assets>(sp_std::marker::PhantomData<Assets>);

impl<AccountId, Assets> BridgeAssets<AccountId> for PalletAssetsBridge<Assets>
where
    AccountId: Eq,
    Assets: frame_support::traits::fungibles::Mutate<AccountId, AssetId = u128, Balance = u128>,
{
    fn mint(polkadex_asset_id: u128, recipient: &AccountId, amount: u128)
        -> frame_support::pallet_prelude::DispatchResult
    {
        Assets::mint_into(polkadex_asset_id, recipient, amount).map(|_| ())
    }

    fn burn(polkadex_asset_id: u128, from: &AccountId, amount: u128)
        -> frame_support::pallet_prelude::DispatchResult
    {
        use frame_support::traits::tokens::{Fortitude, Precision, Preservation};
        Assets::burn_from(
            polkadex_asset_id,
            from,
            amount,
            Preservation::Expendable,
            Precision::Exact,
            Fortitude::Polite,
        ).map(|_| ())
    }
}

#[frame_support::pallet]
pub mod pallet {
    use super::{mpt, BridgeAssets, DepositProof, EthBlockHeader, TokenConfig, WithdrawalMessage};
    use frame_support::pallet_prelude::*;
    use frame_system::pallet_prelude::*;
    use parity_scale_codec::Decode;

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    // ── Config ─────────────────────────────────────────────────────────────

    #[pallet::config]
    pub trait Config: frame_system::Config {
        /// Callback that mints bridged tokens on Polkadex.
        /// The runtime implements this using `pallet_assets`.
        type BridgeAssets: BridgeAssets<Self::AccountId>;

        /// Weight information for pallet extrinsics.
        type WeightInfo: WeightInfo;
    }

    // ── Storage ────────────────────────────────────────────────────────────

    /// Finalized Ethereum block headers keyed by block number.
    /// Submitted by the authorized relayer.
    #[pallet::storage]
    pub type EthHeaders<T: Config> =
        StorageMap<_, Blake2_128Concat, u64, EthBlockHeader, OptionQuery>;

    /// Deposit nonces that have already been processed (replay protection).
    #[pallet::storage]
    pub type ProcessedDeposits<T: Config> =
        StorageMap<_, Blake2_128Concat, u64, bool, ValueQuery>;

    /// The `PolkadexBridge` contract address on Ethereum.
    /// Only logs from this address are accepted as valid `Deposit` events.
    #[pallet::storage]
    pub type BridgeContractAddress<T: Config> = StorageValue<_, [u8; 20], OptionQuery>;

    /// Account authorised to submit Ethereum block headers.
    #[pallet::storage]
    pub type AuthorizedRelayer<T: Config> = StorageValue<_, T::AccountId, OptionQuery>;

    /// Maps Ethereum ERC-20 address → bridge token configuration.
    /// Populated by governance via `register_token`.
    #[pallet::storage]
    pub type TokenRegistry<T: Config> =
        StorageMap<_, Blake2_128Concat, [u8; 20], TokenConfig, OptionQuery>;

    /// Monotonically increasing nonce for outgoing withdrawals.
    /// Each `initiate_withdrawal` call increments this.
    #[pallet::storage]
    pub type OutgoingNonce<T: Config> = StorageValue<_, u64, ValueQuery>;

    /// Outgoing withdrawal messages keyed by nonce.
    /// The BEEFY relayer reads these to build the Merkle batch submitted to Ethereum.
    #[pallet::storage]
    pub type PendingWithdrawals<T: Config> =
        StorageMap<_, Blake2_128Concat, u64, WithdrawalMessage, OptionQuery>;

    // ── Events ─────────────────────────────────────────────────────────────

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        /// The authorised relayer was updated.
        RelayerSet { relayer: T::AccountId },

        /// The bridge contract address was updated.
        BridgeContractSet { address: [u8; 20] },

        /// A new Ethereum block header was accepted from the relayer.
        EthHeaderSubmitted { block_number: u64, block_hash: [u8; 32] },

        /// A deposit proof was verified and WETH was credited to the recipient.
        DepositProcessed {
            nonce: u64,
            token: [u8; 20],
            recipient: T::AccountId,
            amount: u128,
        },

        /// A token pair was registered by governance.
        TokenRegistered {
            eth_token: [u8; 20],
            polkadex_asset_id: u128,
            eth_asset_id: u32,
        },

        /// A user initiated a withdrawal from Polkadex to Ethereum.
        /// The withdrawal is now queued for the next BEEFY batch.
        WithdrawalInitiated {
            nonce: u64,
            eth_token: [u8; 20],
            sender: T::AccountId,
            eth_recipient: [u8; 20],
            amount: u128,
        },
    }

    // ── Errors ─────────────────────────────────────────────────────────────

    #[pallet::error]
    pub enum Error<T> {
        /// No relayer has been configured yet.
        NoRelayerSet,
        /// Caller is not the authorized relayer.
        NotAuthorizedRelayer,
        /// No bridge contract address has been set.
        BridgeContractNotSet,
        /// The referenced block header has not been submitted yet.
        HeaderNotFound,
        /// This deposit nonce has already been processed.
        DepositAlreadyProcessed,
        /// The MPT proof does not match the block's receipts root.
        InvalidMptProof,
        /// Failed to RLP-decode the provided receipt.
        InvalidReceipt,
        /// The specified log index is outside the receipt's logs array.
        LogIndexOutOfRange,
        /// The log at `log_index` is not a valid `Deposit` event from the bridge contract.
        InvalidDepositEvent,
        /// The nonce in the `Deposit` event does not match `proof.deposit_nonce`.
        NonceMismatch,
        /// The `polkadexRecipient` bytes cannot be decoded as a valid AccountId.
        InvalidRecipient,
        /// The deposit amount overflows u128 (practically impossible for WETH).
        AmountOverflow,
        /// No token configuration found for the given Ethereum token address.
        /// Call `register_token` first.
        TokenNotRegistered,
        /// The user does not hold enough of the bridged token to initiate this withdrawal.
        InsufficientBalance,
        /// Withdrawal amount must be greater than zero.
        ZeroWithdrawalAmount,
    }

    // ── Weights ────────────────────────────────────────────────────────────

    pub trait WeightInfo {
        fn set_authorized_relayer() -> Weight;
        fn set_bridge_contract() -> Weight;
        fn submit_eth_header() -> Weight;
        fn submit_deposit_proof() -> Weight;
        fn register_token() -> Weight;
        fn initiate_withdrawal() -> Weight;
    }

    /// Placeholder weights — replace with benchmarked values before mainnet.
    pub struct TestWeightInfo;
    impl WeightInfo for TestWeightInfo {
        fn set_authorized_relayer() -> Weight { Weight::from_parts(10_000, 0) }
        fn set_bridge_contract()    -> Weight { Weight::from_parts(10_000, 0) }
        fn submit_eth_header()      -> Weight { Weight::from_parts(50_000, 0) }
        fn submit_deposit_proof()   -> Weight { Weight::from_parts(500_000, 0) }
        fn register_token()         -> Weight { Weight::from_parts(10_000, 0) }
        fn initiate_withdrawal()    -> Weight { Weight::from_parts(100_000, 0) }
    }

    // ── Calls ──────────────────────────────────────────────────────────────

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// Set the account authorised to submit Ethereum block headers.
        ///
        /// Only callable by Root (sudo / governance).
        #[pallet::call_index(0)]
        #[pallet::weight(T::WeightInfo::set_authorized_relayer())]
        pub fn set_authorized_relayer(
            origin: OriginFor<T>,
            relayer: T::AccountId,
        ) -> DispatchResult {
            ensure_root(origin)?;
            <AuthorizedRelayer<T>>::put(&relayer);
            Self::deposit_event(Event::RelayerSet { relayer });
            Ok(())
        }

        /// Set the `PolkadexBridge` contract address on Ethereum.
        ///
        /// Only callable by Root (sudo / governance).
        #[pallet::call_index(1)]
        #[pallet::weight(T::WeightInfo::set_bridge_contract())]
        pub fn set_bridge_contract(
            origin: OriginFor<T>,
            address: [u8; 20],
        ) -> DispatchResult {
            ensure_root(origin)?;
            <BridgeContractAddress<T>>::put(address);
            Self::deposit_event(Event::BridgeContractSet { address });
            Ok(())
        }

        /// Submit a finalised Ethereum block header.
        ///
        /// Only the authorized relayer can call this. The `receipts_root` field
        /// is used to verify all subsequent deposit proofs for this block.
        #[pallet::call_index(2)]
        #[pallet::weight(T::WeightInfo::submit_eth_header())]
        pub fn submit_eth_header(
            origin: OriginFor<T>,
            header: EthBlockHeader,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            let relayer = <AuthorizedRelayer<T>>::get().ok_or(Error::<T>::NoRelayerSet)?;
            ensure!(who == relayer, Error::<T>::NotAuthorizedRelayer);

            let block_number = header.block_number;
            let block_hash = header.block_hash;
            <EthHeaders<T>>::insert(block_number, header);
            Self::deposit_event(Event::EthHeaderSubmitted { block_number, block_hash });
            Ok(())
        }

        /// Prove an Ethereum `Deposit` event and mint bridged WETH to the recipient.
        ///
        /// Callable by anyone. The MPT proof is the authorization — no signature needed.
        ///
        /// Steps performed on-chain:
        /// 1. Check the nonce has not been processed (replay protection).
        /// 2. Look up the stored Ethereum block header.
        /// 3. Verify the receipt is included in the block via MPT proof.
        /// 4. Parse the receipt to find the `Deposit` log at `proof.log_index`.
        /// 5. Validate the log comes from the configured bridge contract.
        /// 6. Decode the `Deposit` event and verify the nonce.
        /// 7. Decode the Polkadex recipient AccountId from the 32-byte topic.
        /// 8. Mark the nonce as processed and call `BridgeAssets::mint`.
        #[pallet::call_index(3)]
        #[pallet::weight(T::WeightInfo::submit_deposit_proof())]
        pub fn submit_deposit_proof(
            origin: OriginFor<T>,
            proof: DepositProof,
        ) -> DispatchResult {
            ensure_signed(origin)?;

            // 1. Replay protection
            ensure!(
                !<ProcessedDeposits<T>>::get(proof.deposit_nonce),
                Error::<T>::DepositAlreadyProcessed
            );

            // 2. Block header
            let header = <EthHeaders<T>>::get(proof.block_number)
                .ok_or(Error::<T>::HeaderNotFound)?;

            // 3. Bridge contract address
            let bridge_contract =
                <BridgeContractAddress<T>>::get().ok_or(Error::<T>::BridgeContractNotSet)?;

            // 4. MPT proof
            let receipt_valid = mpt::verify_receipt_proof(
                header.receipts_root,
                proof.tx_index,
                &proof.receipt_rlp,
                &proof.mpt_proof,
            );
            ensure!(receipt_valid, Error::<T>::InvalidMptProof);

            // 5. Parse logs
            let logs = mpt::parse_receipt_logs(&proof.receipt_rlp)
                .map_err(|_| Error::<T>::InvalidReceipt)?;

            let log_idx = proof.log_index as usize;
            ensure!(log_idx < logs.len(), Error::<T>::LogIndexOutOfRange);

            // 6. Decode Deposit event
            let event = mpt::parse_deposit_event(&logs[log_idx], bridge_contract)
                .map_err(|e| match e {
                    mpt::ParseError::AmountOverflow => Error::<T>::AmountOverflow,
                    _ => Error::<T>::InvalidDepositEvent,
                })?;

            // 7. Nonce cross-check
            ensure!(event.nonce == proof.deposit_nonce, Error::<T>::NonceMismatch);

            // 8. Look up token config — rejects deposits for unregistered tokens
            let config = <TokenRegistry<T>>::get(event.token)
                .ok_or(Error::<T>::TokenNotRegistered)?;

            // 9. Decode Polkadex AccountId from the 32-byte recipient field
            let recipient = T::AccountId::decode(&mut &event.polkadex_recipient[..])
                .map_err(|_| Error::<T>::InvalidRecipient)?;

            // 10. Convert amount from Ethereum decimals → Polkadex 12-decimal units.
            //     Example (WETH, 18 dec): 1e18 → 1e12
            let native_amount = config.eth_to_native(event.amount);

            // 11. Mark processed before minting (prevent reentrancy)
            <ProcessedDeposits<T>>::insert(proof.deposit_nonce, true);

            // 12. Mint bridged token via pallet-assets (through BridgeAssets impl)
            T::BridgeAssets::mint(config.polkadex_asset_id, &recipient, native_amount)?;

            Self::deposit_event(Event::DepositProcessed {
                nonce: proof.deposit_nonce,
                token: event.token,
                recipient,
                amount: native_amount,
            });

            Ok(())
        }

        /// Register a supported token pair (governance only).
        ///
        /// For WETH on Sepolia:
        ///   eth_token         = 0xfFf9976782d46CC05630D1f6eBAb18b2324d6B14
        ///   polkadex_asset_id = <u128 AssetId created in pallet-assets>
        ///   eth_asset_id      = 1  (must match TokenRegistry.sol on Ethereum)
        ///   decimals          = 18 (WETH uses 18 decimals; USDC uses 6)
        #[pallet::call_index(4)]
        #[pallet::weight(T::WeightInfo::register_token())]
        pub fn register_token(
            origin: OriginFor<T>,
            eth_token: [u8; 20],
            polkadex_asset_id: u128,
            eth_asset_id: u32,
            decimals: u8,
        ) -> DispatchResult {
            ensure_root(origin)?;
            let config = TokenConfig { polkadex_asset_id, eth_asset_id, decimals };
            <TokenRegistry<T>>::insert(eth_token, config);
            Self::deposit_event(Event::TokenRegistered { eth_token, polkadex_asset_id, eth_asset_id });
            Ok(())
        }

        /// Initiate a withdrawal from Polkadex back to Ethereum.
        ///
        /// Burns `amount` of the bridged token from the caller's Polkadex account and
        /// queues a `WithdrawalMessage` that the BEEFY relayer will include in the next
        /// signed Merkle batch. Once the batch root is committed to `BeefyLightClient.sol`,
        /// the user calls `PolkadexBridge.sol::withdraw()` on Ethereum with the Merkle
        /// proof to receive their WETH.
        ///
        /// # Arguments
        /// * `eth_token`     — Ethereum ERC-20 address (e.g. WETH on Sepolia).
        /// * `amount`        — Amount in the token's Ethereum decimals (18 for WETH).
        /// * `eth_recipient` — Ethereum address that will receive the tokens.
        #[pallet::call_index(5)]
        #[pallet::weight(T::WeightInfo::initiate_withdrawal())]
        pub fn initiate_withdrawal(
            origin: OriginFor<T>,
            eth_token: [u8; 20],
            amount: u128,
            eth_recipient: [u8; 20],
        ) -> DispatchResult {
            let caller = ensure_signed(origin)?;
            ensure!(amount > 0, Error::<T>::ZeroWithdrawalAmount);

            // 1. Verify the token is registered
            let config = <TokenRegistry<T>>::get(eth_token)
                .ok_or(Error::<T>::TokenNotRegistered)?;

            // 2. Burn `amount` (in Polkadex 12-decimal units) from caller's pallet-assets balance.
            //    BridgeAssets::burn will call pallet-assets::burn_from under the hood.
            T::BridgeAssets::burn(config.polkadex_asset_id, &caller, amount)
                .map_err(|_| Error::<T>::InsufficientBalance)?;

            // 3. Get and advance the outgoing nonce
            let nonce = <OutgoingNonce<T>>::get();
            <OutgoingNonce<T>>::put(nonce.saturating_add(1));

            // 4. Encode caller AccountId as 32 bytes for the Merkle leaf.
            //    For Polkadex (AccountId32) this is always exactly 32 bytes of the public key.
            let encoded = parity_scale_codec::Encode::encode(&caller);
            let mut polkadex_sender = [0u8; 32];
            let copy_len = encoded.len().min(32);
            polkadex_sender[..copy_len].copy_from_slice(&encoded[..copy_len]);

            // 5. Convert burned amount (12-decimal Polkadex units) → Ethereum decimals.
            //    The Ethereum contract will release exactly `eth_amount` of the ERC-20.
            //    Example (WETH, 18 dec): 1_000_000_000_000 → 1_000_000_000_000_000_000
            let eth_amount = config.native_to_eth(amount);

            // 6. Store the withdrawal message for the BEEFY relayer to pick up.
            //    `eth_amount` is in Ethereum decimals — this is what the bridge contract releases.
            let message = WithdrawalMessage {
                nonce,
                eth_asset_id: config.eth_asset_id,
                amount: eth_amount,
                eth_recipient,
                polkadex_sender,
            };
            <PendingWithdrawals<T>>::insert(nonce, &message);

            Self::deposit_event(Event::WithdrawalInitiated {
                nonce,
                eth_token,
                sender: caller,
                eth_recipient,
                amount: eth_amount,
            });

            Ok(())
        }
    }

    // ── Public helpers ─────────────────────────────────────────────────────

    impl<T: Config> Pallet<T> {
        /// Returns `true` if the given deposit nonce has been processed.
        pub fn is_deposit_processed(nonce: u64) -> bool {
            <ProcessedDeposits<T>>::get(nonce)
        }

        /// Returns the stored header for a given Ethereum block number, if any.
        pub fn eth_header(block_number: u64) -> Option<EthBlockHeader> {
            <EthHeaders<T>>::get(block_number)
        }

        /// Returns the pending withdrawal message for the given nonce, if any.
        /// Used by the BEEFY relayer to build the outgoing Merkle batch.
        pub fn pending_withdrawal(nonce: u64) -> Option<WithdrawalMessage> {
            <PendingWithdrawals<T>>::get(nonce)
        }

        /// Returns the next outgoing nonce (i.e. how many withdrawals have been initiated).
        pub fn outgoing_nonce() -> u64 {
            <OutgoingNonce<T>>::get()
        }
    }
}
