/**
 * Polkadex ↔ Ethereum WETH Bridge Relayer
 *
 * Starts two independent workers:
 *   1. DepositWorker  — watches Ethereum for Deposit events and submits proofs to Polkadex
 *   2. WithdrawalWorker — polls Polkadex for pending withdrawals and submits BEEFY batches to Ethereum
 *
 * Usage:
 *   cp .env.example .env          # fill in your keys and addresses
 *   npm install
 *   npm run dev                   # TypeScript dev mode
 *   npm run build && npm start    # production
 */

import { getApi } from './polkadex/client'
import { getWsProvider } from './ethereum/client'
import { startDepositWorker } from './workers/depositWorker'
import { startWithdrawalWorker } from './workers/withdrawalWorker'
import { closeConnections } from './ethereum/client'
import { disconnect as disconnectPolkadex } from './polkadex/client'

async function main(): Promise<void> {
  console.log('╔══════════════════════════════════════════════╗')
  console.log('║  Polkadex ↔ Ethereum WETH Bridge Relayer    ║')
  console.log('╚══════════════════════════════════════════════╝\n')

  // Initialise both chain connections upfront so startup failures are caught early
  await getApi()
  getWsProvider() // WebSocket provider connects lazily; trigger it now

  // Start both workers
  startDepositWorker()
  startWithdrawalWorker()

  console.log('\n[Main] Both workers running. Press Ctrl+C to stop.\n')

  // Graceful shutdown
  process.on('SIGINT',  () => shutdown('SIGINT'))
  process.on('SIGTERM', () => shutdown('SIGTERM'))
}

async function shutdown(signal: string): Promise<void> {
  console.log(`\n[Main] Received ${signal}, shutting down…`)
  await closeConnections()
  await disconnectPolkadex()
  process.exit(0)
}

main().catch(err => {
  console.error('[Main] Fatal startup error:', err)
  process.exit(1)
})
