import { ApiPromise, WsProvider } from '@polkadot/api'
import { Keyring } from '@polkadot/keyring'
import type { KeyringPair } from '@polkadot/keyring/types'
import { config } from '../config'

// Custom types that mirror pallet-eth-bridge Rust structs.
// @polkadot/api uses these to encode/decode SCALE codec correctly.
const ETH_BRIDGE_TYPES = {
  EthBlockHeader: {
    block_number:  'u64',
    block_hash:    '[u8; 32]',
    receipts_root: '[u8; 32]',
    timestamp:     'u64',
  },
  DepositProof: {
    block_number:   'u64',
    tx_index:       'u64',
    receipt_rlp:    'Vec<u8>',
    mpt_proof:      'Vec<Vec<u8>>',
    log_index:      'u32',
    deposit_nonce:  'u64',
  },
  TokenConfig: {
    polkadex_asset_id: 'u128',
    eth_asset_id:      'u32',
    decimals:          'u8',
  },
  WithdrawalMessage: {
    nonce:           'u64',
    eth_asset_id:    'u32',
    amount:          'u128',
    eth_recipient:   '[u8; 20]',
    polkadex_sender: '[u8; 32]',
  },
}

let _api: ApiPromise | null = null

export async function getApi(): Promise<ApiPromise> {
  if (_api && _api.isConnected) return _api

  const provider = new WsProvider(config.polkadex.wsUrl)
  _api = await ApiPromise.create({
    provider,
    types: ETH_BRIDGE_TYPES,
  })

  console.log(`[Polkadex] Connected to ${config.polkadex.wsUrl}`)
  console.log(`[Polkadex] Chain: ${(await _api.rpc.system.chain()).toHuman()}`)

  return _api
}

// The relayer keypair used to sign and submit extrinsics.
// Must match the AuthorizedRelayer set in pallet-eth-bridge.
let _relayerPair: KeyringPair | null = null

export function getRelayerPair(): KeyringPair {
  if (_relayerPair) return _relayerPair

  const keyring = new Keyring({ type: 'sr25519' })
  _relayerPair = keyring.addFromUri(config.polkadex.relayerSeed)
  console.log(`[Polkadex] Relayer address: ${_relayerPair.address}`)
  return _relayerPair
}

export async function disconnect(): Promise<void> {
  if (_api) {
    await _api.disconnect()
    _api = null
  }
}
