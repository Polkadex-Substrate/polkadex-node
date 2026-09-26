# Orderbook ledger extraction

Recovers the per-user orderbook balance ledger from a validator node's local database.

## Background

The orderbook engine halted on 14 December 2024 at block 8,440,893. The last snapshot before the halt is number 694,063, and the chain records its state hash on chain:

```
Snapshots(694063).state_hash = 0x0899feaee1f47b34036109f56c76b5fbd22ec9e339ecc57fa0bcc58f6a2f05c0
```

That hash is the root of a Merkle trie of per-user balances. The trie itself was never stored on chain. Every validator in the orderbook authority set at the time ran the orderbook worker inside its node, and that worker kept its own copy of the trie in the node's offchain storage, inside the client database. It was never synced between nodes, so each validator held it independently. Nothing in that storage is ever physically deleted, so a node that was in sync at the halt and has not been re-synced since still holds the full ledger.

This script opens such a database read-only, finds the trie, verifies every node against its hash, and writes the balances to a CSV. A CSV produced from root `0x0899fe...` is provably the ledger the chain committed to at the halt.

## Who has it

A node may hold the ledger if all of these are true:

1. It was an active validator on 14 December 2024, with an orderbook session key set.
2. It has never been purged, re-synced from scratch, or restored from a snapshot since.
3. It was in sync with the engine at the halt. Nodes whose worker had stalled earlier hold an older ledger, which the scan reports.

## Usage

Requires Python 3.9 or newer, and free memory roughly equal to the size of the offchain column, which for a node that ran to the halt may be 8 to 16 GB. The scan loads every ledger node before walking.

```
pip install -r requirements.txt

# 1. Report what the database contains. Read-only, takes about a minute.
python3 extract_ocex_trie.py --db /path/to/chains/polkadex_main_network/db/full --scan

# 2. If the scan reports the halt root present, extract the ledger at that root.
python3 extract_ocex_trie.py --db /path/to/chains/polkadex_main_network/db/full \
    --root 0x0899feaee1f47b34036109f56c76b5fbd22ec9e339ecc57fa0bcc58f6a2f05c0 \
    --out balances.csv
```

The database path is the one the node prints at startup in the line `Database: RocksDb at <path>`. For the default setup it is `~/.local/share/polkadex-node/chains/polkadex_main_network/db/full`; for the Docker image it is `chains/polkadex_main_network/db/full` under the volume mounted at `/data`.

The script opens RocksDB in read-only mode and writes nothing. It never reads the `keystore` or `network` directories. If the read-only open fails because the node holds the database lock, stop the node for the few minutes the scan takes and start it again afterwards.

## What to send

The console output of the scan, and `balances.csv` if produced. Do not send the database, the keystore, or the network key.

## Verification

Each trie node is verified against its blake2 hash during traversal, and the root is compared to the on-chain state hash. A CSV that traverses cleanly from `0x0899fe...` cannot have been altered without the mismatch being reported. Independent copies from different validators should be byte-identical.
