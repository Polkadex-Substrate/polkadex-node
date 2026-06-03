import * as dotenv from 'dotenv'
dotenv.config()

function require(key: string): string {
  const val = process.env[key]
  if (!val) throw new Error(`Missing required env var: ${key}`)
  return val
}

function optional(key: string, defaultValue: string): string {
  return process.env[key] ?? defaultValue
}

export const config = {
  ethereum: {
    wsUrl:              require('ETH_WS_URL'),
    httpUrl:            require('ETH_HTTP_URL'),
    relayerPrivateKey:  require('ETH_RELAYER_PRIVATE_KEY'),
    beefyLightClient:   require('BEEFY_LIGHT_CLIENT_ADDRESS'),
    polkadexBridge:     require('POLKADEX_BRIDGE_ADDRESS'),
    confirmationBlocks: Number(optional('ETH_CONFIRMATION_BLOCKS', '12')),
    validatorSetId:     BigInt(optional('BEEFY_VALIDATOR_SET_ID', '1')),
  },
  polkadex: {
    wsUrl:         require('POLKADEX_WS_URL'),
    relayerSeed:   require('POLKADEX_RELAYER_SEED'),
  },
  relayer: {
    withdrawalPollMs:  Number(optional('WITHDRAWAL_POLL_INTERVAL_MS', '12000')),
    withdrawalBatchSize: Number(optional('WITHDRAWAL_BATCH_SIZE', '1')),
    logLevel:          optional('LOG_LEVEL', 'info'),
  },
}

export type Config = typeof config
