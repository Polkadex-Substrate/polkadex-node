import { ethers } from 'ethers'
import { config } from '../config'

// Singleton providers — created once and reused
let _wsProvider:   ethers.WebSocketProvider | null = null
let _httpProvider: ethers.JsonRpcProvider   | null = null
let _wallet:       ethers.Wallet            | null = null

export function getWsProvider(): ethers.WebSocketProvider {
  if (!_wsProvider) {
    _wsProvider = new ethers.WebSocketProvider(config.ethereum.wsUrl)
  }
  return _wsProvider
}

export function getHttpProvider(): ethers.JsonRpcProvider {
  if (!_httpProvider) {
    _httpProvider = new ethers.JsonRpcProvider(config.ethereum.httpUrl)
  }
  return _httpProvider
}

// The relayer wallet signs BEEFY commitments submitted to BeefyLightClient.sol.
// Its address must be in the validator set registered on-chain.
export function getRelayerWallet(): ethers.Wallet {
  if (!_wallet) {
    _wallet = new ethers.Wallet(
      config.ethereum.relayerPrivateKey,
      getHttpProvider(),
    )
  }
  return _wallet
}

export async function closeConnections(): Promise<void> {
  if (_wsProvider) {
    await _wsProvider.destroy()
    _wsProvider = null
  }
}
