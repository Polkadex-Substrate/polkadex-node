# Mainnet State Fork — Upgrade Testing Guide

This guide covers how to create an isolated silo network from mainnet state, test a client + runtime upgrade end-to-end (including cross-chain), and then perform the real mainnet upgrade.

---

## Overview

```
Sync with mainnet → Export state → Fork into silo → Test upgrade → Tear down → Upgrade mainnet
```

---

## Prerequisites

- New node binary built and ready
- New runtime WASM compiled and ready
- All storage migrations documented
- New binary confirmed backwards compatible (can run old runtime)
- 5 servers provisioned:

| Server | Role |
|--------|------|
| silo-boot | Bootnode |
| silo-val-1 | Validator 1 |
| silo-val-2 | Validator 2 |
| silo-val-3 | Validator 3 |
| silo-rpc | RPC / Archive |

- Session keys generated for all 3 validators (keys you control)
- Sudo key generated

---

## Phase 1 — Sync with Mainnet

On all 5 servers, start the **old binary** with the mainnet chainspec and let them fully sync.

```bash
polkadex-node \
  --chain mainnet \
  --base-path /data \
  --database rocksdb \
  --sync warp \
  --bootnodes /dns/polkadex.boot.faradaynodes.com/tcp/30716/p2p/12D3KooWENh4QpXzfaD8nonhTaSHreuRpKRTRtDTztGeJ7XxXgyw
```

Wait until all nodes are fully synced (check with `system_syncState` RPC).

---

## Phase 2 — Export Mainnet State

Once synced, pick a recent finalized block and export the state from the RPC node:

```bash
polkadex-node export-state \
  --chain mainnet \
  --base-path /data \
  > mainnet-state.json
```

> Note the block number at which the state was exported — you will need it.

---

## Phase 3 — Fork the State

Modify `mainnet-state.json` to prepare it for the isolated silo:

1. **Replace the validator set** with your own session keys (BABE + GRANDPA + BEEFY)
2. **Add a sudo key** for fast-tracking democracy during testing
3. **Reduce democracy periods** (voting period, enactment delay) to speed up testing
4. **Set a unique `protocolId`** so silo nodes never accidentally connect to mainnet peers

Tools like [`fork-off-substrate`](https://github.com/maxsam4/fork-off-substrate) can automate steps 1–3.

---

## Phase 4 — Generate Silo Chainspec

Convert the modified state into a raw chainspec:

```bash
polkadex-node build-spec \
  --chain mainnet-state.json \
  --raw \
  > silo-chainspec.json
```

Distribute `silo-chainspec.json` to all 5 servers.

---

## Phase 5 — Start the Silo Network

**On silo-boot:**
```bash
polkadex-node \
  --chain /chain/silo-chainspec.json \
  --base-path /data \
  --node-key-file /node-key \
  --no-mdns \
  --port 30333
```

Note the bootnode peer ID from the logs.

**On silo-val-1, silo-val-2, silo-val-3:**
```bash
polkadex-node \
  --chain /chain/silo-chainspec.json \
  --base-path /data \
  --validator \
  --no-mdns \
  --bootnodes /ip4/<silo-boot-ip>/tcp/30333/p2p/<bootnode-peer-id>
```

**On silo-rpc:**
```bash
polkadex-node \
  --chain /chain/silo-chainspec.json \
  --base-path /data \
  --rpc-external \
  --rpc-port 9944 \
  --no-mdns \
  --bootnodes /ip4/<silo-boot-ip>/tcp/30333/p2p/<bootnode-peer-id>
```

Verify the silo is producing and finalizing blocks before proceeding.

---

## Phase 6 — Cross-Chain Setup (Anvil + Hyperbridge)

**Start Anvil** on silo-rpc (or a dedicated server):

```bash
anvil --port 8545 --chain-id 11155111
```

**Deploy fresh Hyperbridge contracts** on Anvil, initialized with the silo's BEEFY validator set.

**Configure Tesseract** to point at the silo:

```toml
[hyperbridge]
state_machine = "KUSAMA-4009"
rpc_ws = "ws://<silo-rpc-ip>:9944"

[relayer]
messaging = true

[evm-11155111]
rpc_urls = ["http://127.0.0.1:8545"]
ismp_host = "<fresh-hyperbridge-contract-address>"
```

Start Tesseract and verify the EVM channel is active in the logs.

**Baseline test:** Do a cross-chain transfer (Anvil → silo PDEX and back) and confirm it succeeds.

---

## Phase 7 — Client Binary Upgrade

Upgrade the node binary on each server one at a time. After each restart, verify the chain keeps finalizing before moving to the next.

```bash
# Stop the node
systemctl stop polkadex-node

# Replace the binary
cp polkadex-node-new /usr/local/bin/polkadex-node

# Start the node
systemctl start polkadex-node
```

Order: `silo-rpc` → `silo-val-1` → `silo-val-2` → `silo-val-3` → `silo-boot`

---

## Phase 8 — Runtime Upgrade via Democracy

1. Submit the new runtime WASM via the sudo key (fast-tracks democracy in the silo):
   - Polkadot.js → `sudo` → `system.setCode(wasm)`
2. Wait for enactment
3. Verify the chain continues finalizing after the runtime upgrade
4. Check storage migrations ran correctly — query expected storage keys on the RPC node

---

## Phase 9 — Post-Upgrade Cross-Chain Verification

Repeat the cross-chain transfer test after the upgrade:

1. Transfer from Anvil → silo PDEX
2. Transfer from silo PDEX → Anvil
3. Confirm both succeed ✓

If cross-chain works after the upgrade, the silo test is complete.

---

## Phase 10 — Tear Down Silo

Stop all silo nodes and delete chain data:

```bash
systemctl stop polkadex-node
rm -rf /data
```

---

## Phase 11 — Real Mainnet Upgrade

1. **Coordinate with all mainnet validators** — share the new binary and the planned enactment block
2. Each validator upgrades their binary **before** the enactment block
3. **Propose the runtime upgrade** via democracy on real mainnet:
   - Submit the WASM via `technicalCommittee.propose` or democracy
4. Monitor the voting period
5. Confirm enactment — verify chain keeps finalizing
6. Verify cross-chain (Tesseract) is still active after the upgrade

---

## Key Risks

| Risk | Mitigation |
|------|-----------|
| Validator misses binary upgrade before enactment | Set enactment block far enough ahead; direct contact with each validator |
| Storage migration fails | Caught in silo Phase 8 — fix before touching mainnet |
| Chain stalls after upgrade | Debug in silo first; never skip silo testing |
| Cross-chain breaks after upgrade | Silo Phase 9 catches this |
| Silo accidentally connects to mainnet | Unique `protocolId` + different genesis hash prevents this |
