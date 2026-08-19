# Silo Single-Node Deployment — Practical Guide

This document covers deploying a minimal single-validator silo node for runtime upgrade testing. It is a simplified alternative to the full multi-node silo described in `mainnet-state-fork.md` — useful when you need to test migrations quickly on one server without setting up 5 nodes.

The guide is based on the actual deployment done on 2026-07-01, including every challenge encountered and how each was resolved.

---

## Overview

```
Export mainnet state → Filter → Patch genesis → Run Alice node → Test upgrade
```

A single Docker container running `polkadex-node` with `--alice` and `--force-authoring` acts as the sole BABE and GRANDPA authority. The genesis is a fork of real mainnet state, so migrations run against production data.

---

## Prerequisites

- Docker and Docker Compose v2
- A server with at least 8 GB RAM and 100 GB disk
- A synced Polkadex archive or RPC node to export state from (or access to one via RPC)
- The `fork-off-substrate` tool set up with Node.js
- The new runtime WASM (`.wasm`) ready for upload
- `python3` available on the server for genesis patching

---

## Step 1 — Export Mainnet State

On a node that is fully synced to mainnet, export the state at a recent finalized block:

```bash
polkadex-node export-state \
  --chain /chain/chainspec.json \
  --base-path /data \
  > /root/exported-state.json
```

This produces a large JSON file containing every storage key-value pair from the chain at that block.

---

## Step 2 — Filter the Exported State

The full exported state is too large to use directly. Filter out pallets that are either too large to process or not needed for migration testing:

```python
python3 << 'EOF'
import json, os

print("Loading exported state...")
with open('/root/exported-state.json') as f:
    data = json.load(f)
top = data['genesis']['raw']['top']

EXCLUDE_PREFIXES = [
    '0x0b6bff37d53f98a6',   # OCEX pallet storage (too large)
    '0x3a636f6465',          # :code (runtime WASM — fork-off-substrate replaces this)
    '0x15',                  # Large pallet that exceeded 500MB during RPC export
]

pairs = []
excluded = 0
for k, v in top.items():
    if any(k.startswith(p) for p in EXCLUDE_PREFIXES):
        excluded += 1
        continue
    pairs.append([k, v])

print(f"Kept {len(pairs)} entries, excluded {excluded} entries")

with open('/root/fork-off-substrate/data/storage.json', 'w') as f:
    json.dump(pairs, f)

size = os.path.getsize('/root/fork-off-substrate/data/storage.json')
print(f"storage.json size: {size / 1024 / 1024:.1f} MB")
EOF
```

**Note on excluded pallets:** Migrations that touch excluded pallets will run against empty storage on the silo. This is acceptable for catching most migration bugs, but be aware the coverage is not complete. Identify all excluded prefixes by computing `twox128(pallet_name)` before deciding what to exclude.

---

## Step 3 — Run fork-off-substrate

```bash
cd /root/fork-off-substrate
node index.js
```

This generates a chainspec at `data/customSpec.json`. Build the raw version:

```bash
polkadex-node build-spec \
  --chain data/customSpec.json \
  --raw \
  > /root/polkadex-mainnet-setup/silo-raw.json
```

---

## Step 4 — Patch the Genesis

This is the most involved step. `fork-off-substrate` does not correctly replace all consensus state in Polkadex's genesis. Every item below **must** be patched manually or the node will not produce blocks.

Run all patches in a single script to keep them together:

```python
python3 << 'EOF'
import json, xxhash, struct

with open('/root/polkadex-mainnet-setup/silo-raw.json') as f:
    spec = json.load(f)
top = spec['genesis']['raw']['top']

# Alice sr25519 public key (BABE authority ID / AccountId)
alice_sr25519 = 'd43593c715fdd31c61141abd04a99fd6822c8558854ccde39a5684e7a56da27d'

# Alice ed25519 public key (GRANDPA authority ID)
alice_ed25519 = '88dc3417d5058ec4b4503e0c12ea1a0a89be200fe98922423d4334014fa6b0ee'

# Alice ecdsa public key (BEEFY authority ID, 33 bytes compressed)
alice_ecdsa = '020a1091341fe5664bfa1782d5e04779689068c916b04cb365ec3153755684d9a1'

# SCALE: Vec<(AuthorityId, u64)> with one entry, weight 1
alice_babe_vec = '0x04' + alice_sr25519 + '0100000000000000'

# --- Babe::Authorities ---
top['0x1cb6f36e027abb2091cfb5110ab5087f5e0621c4869aa60c02be9adcc98a0d1d'] = alice_babe_vec

# --- Babe::NextAuthorities ---
top['0x1cb6f36e027abb2091cfb5110ab5087faacf00b9b41fda7a9268821c2a2b3e4c'] = alice_babe_vec

# --- Babe::GenesisSlot = 0 (CRITICAL — see Challenges section) ---
top['0x1cb6f36e027abb2091cfb5110ab5087f678711d15ebbceba5cd0cea158e6675a'] = '0x0000000000000000'

# --- Babe::CurrentSlot = 0 ---
top['0x1cb6f36e027abb2091cfb5110ab5087f06155b3cd9a8c9e5e9a23fd5dc13a5ed'] = '0x0000000000000000'

# --- Babe::EpochIndex = 0 ---
top['0x1cb6f36e027abb2091cfb5110ab5087f38316cbf8fa0da822a20ac1c55bf1be3'] = '0x0000000000000000'

# --- Babe::EpochStart = (0, 1200) in block numbers ---
top['0x1cb6f36e027abb2091cfb5110ab5087fe90e2fbf2d792cb324bffa9427fe1f0e'] = '0x00000000b0040000'

# --- Session::Validators = [Alice] ---
top['0xcec5070d609dd3497f72bde07fc96ba088dcde934c658227ee1dfafcd6e16903'] = \
    '0x04' + alice_sr25519

# --- Sudo::Key = Alice ---
top['0x5c0d1176a568c1f92944340dbfed9e9c530ebca703c85910e7164cb7d1c9e47b'] = \
    alice_sr25519

# --- :grandpa_authorities = VersionedAuthorityList::V1([(Alice_ed25519, 1)]) ---
# 0x01 = V1 variant, 0x04 = compact(1 entry)
top['0x3a6772616e6470615f617574686f726974696573'] = \
    '0x01' + '04' + alice_ed25519 + '0100000000000000'

# --- Session::NextKeys(Alice) and Session::QueuedKeys (see Challenges section) ---
#
# Polkadex SessionKeys struct order (from impl_opaque_keys! in runtime):
#   grandpa(ed25519,32) + babe(sr25519,32) + im_online(sr25519,32) +
#   authority_discovery(sr25519,32) + orderbook/OCEX(sr25519,32) + beefy(ecdsa,33)
#   = 193 bytes total  (mixnet is 0 bytes in this runtime version)
#
alice_session_keys = (
    alice_ed25519 +   # grandpa
    alice_sr25519 +   # babe
    alice_sr25519 +   # im_online
    alice_sr25519 +   # authority_discovery
    alice_sr25519 +   # orderbook (OCEX)
    alice_ecdsa       # beefy (33 bytes)
)

# Session::NextKeys(Alice) — Twox64Concat map:
#   twox128("Session") + twox128("NextKeys") + twox64(alice) + alice
alice_bytes = bytes.fromhex(alice_sr25519)
h = xxhash.xxh64(alice_bytes, seed=0).intdigest()
twox64_alice = struct.pack('<Q', h).hex()
nextkeys_key = ('0x'
    + 'cec5070d609dd3497f72bde07fc96ba0'   # twox128("Session")
    + '4c014e6bf8b8c2c011e7290b85696bb3'   # twox128("NextKeys")
    + twox64_alice
    + alice_sr25519)
top[nextkeys_key] = '0x' + alice_session_keys

# Session::QueuedKeys — SCALE Vec<(AccountId, SessionKeys)>:
#   compact(1) + alice_AccountId(32) + alice_session_keys(193)
top['0xcec5070d609dd3497f72bde07fc96ba0e0cdd062e6eaf24295ad4ccfc41d4609'] = \
    '0x04' + alice_sr25519 + alice_session_keys

with open('/root/polkadex-mainnet-setup/silo-raw.json', 'w') as f:
    json.dump(spec, f)
print("Patches applied and saved.")
EOF
```

> **Requires `python3-xxhash`:** install it first with `apt-get install -y python3-xxhash`.

### Storage key reference

| Storage item | Key |
|---|---|
| `Babe::Authorities` | `0x1cb6f36e027abb2091cfb5110ab5087f5e0621c4869aa60c02be9adcc98a0d1d` |
| `Babe::NextAuthorities` | `0x1cb6f36e027abb2091cfb5110ab5087faacf00b9b41fda7a9268821c2a2b3e4c` |
| `Babe::GenesisSlot` | `0x1cb6f36e027abb2091cfb5110ab5087f678711d15ebbceba5cd0cea158e6675a` |
| `Babe::CurrentSlot` | `0x1cb6f36e027abb2091cfb5110ab5087f06155b3cd9a8c9e5e9a23fd5dc13a5ed` |
| `Babe::EpochIndex` | `0x1cb6f36e027abb2091cfb5110ab5087f38316cbf8fa0da822a20ac1c55bf1be3` |
| `Babe::EpochStart` | `0x1cb6f36e027abb2091cfb5110ab5087fe90e2fbf2d792cb324bffa9427fe1f0e` |
| `Session::Validators` | `0xcec5070d609dd3497f72bde07fc96ba088dcde934c658227ee1dfafcd6e16903` |
| `Session::QueuedKeys` | `0xcec5070d609dd3497f72bde07fc96ba0e0cdd062e6eaf24295ad4ccfc41d4609` |
| `Session::NextKeys(Alice)` | computed — see script above |
| `Sudo::Key` | `0x5c0d1176a568c1f92944340dbfed9e9c530ebca703c85910e7164cb7d1c9e47b` |
| `:grandpa_authorities` | `0x3a6772616e6470615f617574686f726974696573` |

Keys can be recomputed with:
```js
const { xxhashAsHex } = require('@polkadot/util-crypto');
const key = xxhashAsHex('PalletName', 128) + xxhashAsHex('StorageName', 128).slice(2);
```

---

## Step 5 — Docker Compose Setup

`silo/docker-compose.yml`:

```yaml
services:
  silo:
    image: polkadex-node:mainnet
    container_name: polkadex-silo
    restart: unless-stopped
    ports:
      - "9944:9944"
      - "9615:9615"
    volumes:
      - ../nodes/silo:/data
      - ../silo-raw.json:/chain/silo-raw.json:ro
    command: >
      --chain /chain/silo-raw.json
      --base-path /data
      --validator
      --alice
      --force-authoring
      --no-mdns
      --unsafe-force-node-key-generation
      --rpc-cors all
      --rpc-external
      --unsafe-rpc-external
      --rpc-methods unsafe
      --rpc-port 9944
      --port 30333
      --db-cache 512
      --prometheus-external
      --prometheus-port 9615
      --name polkadex-silo
```

---

## Step 6 — Start and Verify

```bash
# Start fresh (always wipe the DB when changing genesis)
docker compose -f silo/docker-compose.yml down
rm -rf nodes/silo && mkdir -p nodes/silo
docker compose -f silo/docker-compose.yml up -d

# Watch logs
docker logs polkadex-silo -f
```

**Healthy startup looks like:**

```
👶 Starting BABE Authorship worker
👴 Loading GRANDPA authority set from genesis on what appears to be first startup.
👶 New epoch 0 launching at block 0x... (block slot NNNNNN >= start slot NNNNNN).
👶 Next epoch starts at slot NNNNNN
🏆 Imported #1 (...)
🏆 Imported #2 (...)
💤 Idle (0 peers), best: #5, finalized #3
```

**Verify via RPC:**

```bash
# Health check
curl -s -H "Content-Type: application/json" \
  -d '{"id":1,"jsonrpc":"2.0","method":"system_health","params":[]}' \
  http://localhost:9944

# Best and finalized block numbers
curl -s -H "Content-Type: application/json" \
  -d '{"id":1,"jsonrpc":"2.0","method":"chain_getHeader","params":[]}' \
  http://localhost:9944
```

Expected: 0 peers, not syncing, best block increasing, finalized block ~2 behind best.

---

## Step 7 — Connect Polkadot.js Apps to the Silo

### The WSS requirement

`polkadot.js.org/apps` is served over HTTPS. Browsers block mixed content, so a plain `ws://` connection from that page will be silently rejected — the explorer will appear to hang or show a connection error. You need one of the following:

**Option A — Nginx WSS reverse proxy (recommended)**

Install nginx on the silo server and terminate TLS in front of port 9944. With a domain name, use Let's Encrypt. With an IP-only server, use a self-signed certificate (you will need to accept it in the browser first).

```nginx
# /etc/nginx/sites-available/silo-rpc
server {
    listen 443 ssl;
    server_name <your-domain-or-ip>;

    ssl_certificate     /etc/ssl/certs/silo.crt;
    ssl_certificate_key /etc/ssl/private/silo.key;

    location / {
        proxy_pass http://127.0.0.1:9944;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header Host $host;
        proxy_read_timeout 3600s;
    }
}
```

Generate a self-signed cert if you do not have a domain:

```bash
openssl req -x509 -newkey rsa:4096 -keyout /etc/ssl/private/silo.key \
  -out /etc/ssl/certs/silo.crt -days 365 -nodes \
  -subj "/CN=<server-ip>"
```

Visit `https://<server-ip>` in your browser first and accept the certificate warning. Then connect Polkadot.js Apps to `wss://<server-ip>`.

**Option B — Run Polkadot.js Apps locally (simplest)**

Clone and run the apps UI locally over plain HTTP, which has no mixed-content restriction:

```bash
git clone https://github.com/polkadot-js/apps.git
cd apps
yarn install
yarn start
```

Open `http://localhost:3000` and connect to `ws://<server-ip>:9944` directly.

---

### Submitting the runtime upgrade

Once connected:

1. Navigate to **Developer → Sudo**
2. Select `system` → `setCodeWithoutChecks`
3. Upload the new runtime `.wasm`
4. Submit the transaction (signed by Alice — available under dev accounts)
5. Watch the node logs for migration output in the next block

---

## Challenges and Root Causes

### 1. fork-off-substrate does not fix consensus state

`fork-off-substrate` copies raw storage from mainnet but does not replace the validator set, session keys, or sudo key. The silo starts with 200 mainnet BABE authorities and two mainnet GRANDPA keys — Alice is in neither set so she can never author or finalize blocks.

**Fix:** Manually patch every consensus-related storage key listed in Step 4.

---

### 2. BABE stuck at block #0 — `Expected epoch change to happen at [block], sNNNNNN`

This was the hardest problem. After patching BABE authorities to Alice only, blocks were proposed and pre-sealed successfully but import failed every slot with this error.

**Root cause** (found by reading `sc-consensus-babe` v0.55.0 and `pallet-babe` v45.0.0 source):

- `sc-consensus-babe::find_pre_digest` returns a synthetic `slot: 0` for the genesis block (block #0), because genesis has no real BABE pre-digest.
- The block verifier computes `first_in_epoch = parent_slot < epoch_descriptor.start_slot()`. Since `parent_slot = 0` and any non-zero genesis slot would make `epoch_descriptor.start_slot() > 0`, this is always `true` for block #1.
- When `first_in_epoch = true`, the verifier requires a `NextEpochData` consensus digest in the block header.
- `pallet-babe::initialize()` only deposits that digest when it detects `GenesisSlot == 0` in storage — this is an intentional sentinel value. On block #1, when `GenesisSlot == 0`, pallet-babe calls `initialize_genesis_epoch(current_slot)` which sets `GenesisSlot` to the actual slot and deposits the required `NextEpochData` digest.
- Any non-zero value for `GenesisSlot` in genesis storage bypasses this path entirely, so block #1 is produced without the required digest, and import fails.

**Fix:** Set `Babe::GenesisSlot = 0` in `silo-raw.json`. Do not set it to any other value — pallet-babe will populate it correctly from the slot of block #1.

**Key insight:** `GenesisSlot = 0` is not just a valid initial value — it is a **sentinel** that activates epoch 0 initialization on the first block. It is used this way in all Substrate dev chains.

---

### 3. GRANDPA not finalizing — `finalized #0` stays forever

After BABE was fixed, blocks were produced but `finalized` stayed at #0 indefinitely. No GRANDPA voter messages in the logs.

**Root cause:** The `:grandpa_authorities` key in the genesis still had two mainnet ed25519 public keys. Alice's ed25519 key is neither of them. GRANDPA needs more than 2/3 of authority weight to finalize — with 0 out of 2 valid voters, it never reaches quorum.

Note: this key uses a different encoding from regular pallet storage. It is stored as `VersionedAuthorityList::V1(...)`:
- Byte `0x01` — V1 enum variant
- Compact-encoded length
- 32-byte ed25519 public key per authority
- 8-byte u64 weight per authority

**Fix:** Patch `:grandpa_authorities` to `0x010488dc3417...ee0100000000000000` (V1, 1 entry, Alice's ed25519, weight 1).

---

### 4. Silo stalls after ~8 hours — session rotation applies mainnet validators

After running for ~8 hours (two BABE epochs), block production stopped. The silo never recovered.

**Root cause:** In Polkadex, pallet-session rotates validators at each BABE epoch boundary (every 1,200 slots ≈ 4 hours). At each rotation it calls `on_new_session` on BABE, which updates `Babe::Authorities` and `Babe::NextAuthorities` for the NEXT epoch. The authorities are read from **`Session::NextKeys(validator)`** — a per-validator storage map holding that validator's registered session keys.

Because Alice was not a mainnet validator, `Session::NextKeys(Alice)` was missing entirely from the genesis. When pallet-session tried to build Alice's authority record for BABE, it got an empty/zero public key. The `NextEpochData` digest announced in each epoch therefore contained a zero BABE key, and Alice could not produce any blocks once that epoch started.

The genesis also had `Session::QueuedKeys` populated with all 200 mainnet validators. This is used as the `queued` parameter to the first `on_new_session` call, meaning BABE received mainnet BABE keys as the "next epoch" authorities — a second path to the same failure.

`Staking::ForceEra` was already `ForceNone` (0x02) in the mainnet state, which correctly prevents pallet-staking from electing new validators. But the session key lookup failure would have caused the stall regardless.

**Fix:** Patch two additional storage items in the genesis:

1. **`Session::NextKeys(Alice)`** — Add Alice's canonical session keys so pallet-session can populate BABE's authority set at every session boundary.
2. **`Session::QueuedKeys`** — Replace the 200-entry mainnet list with a single Alice entry so the first `on_new_session` call also gets the correct queued authorities.

Both patches are included in the script in Step 4. The `Session::NextKeys(Alice)` storage key is a `Twox64Concat` map entry and requires the `xxhash` Python library to compute.

**Session keys struct for Polkadex (193 bytes):**

```
grandpa (ed25519, 32 bytes)  +  babe (sr25519, 32 bytes)  +
im_online (sr25519, 32 bytes)  +  authority_discovery (sr25519, 32 bytes)  +
orderbook/OCEX (sr25519, 32 bytes)  +  beefy (ecdsa, 33 bytes)
```

Mixnet contributes 0 bytes in this runtime version (pallet is present but does not contribute a key slot to the session keys struct).

**With this fix and `ForceEra = ForceNone`, the silo can run indefinitely** — Alice remains the only authority through all epoch and session rotations because staking never elects new validators and session key lookups always return Alice's correct keys.

---

### 6. Always wipe the chain DB when changing genesis

If the silo is restarted with a modified `silo-raw.json` but the old `nodes/silo/` chain data still exists, the node will use the cached genesis from disk and ignore the updated chainspec entirely. The genesis hash will differ from what you expect.

**Fix:** Always delete at least the `db/` subdirectory (`rm -rf nodes/silo/chains/*/db`) before restarting after any chainspec change. The `keystore/` can be preserved.

---

## Isolation Guarantee

The silo cannot accidentally connect to or affect mainnet. Substrate enforces genesis hash matching at the P2P handshake layer — any peer with a different genesis hash is immediately banned. The forked genesis always produces a different hash from mainnet, so isolation is automatic and not dependent on configuration.

---

## Limitations

- **Excluded pallets are not migration-tested.** Storage migrations that touch OCEX or any other excluded pallet will run against empty storage, not real mainnet data. For full coverage, use the multi-node silo from `mainnet-state-fork.md`.
- **No cross-chain testing.** The single-node silo does not include Anvil + Tesseract. Cross-chain functionality must be tested separately.
- **GRANDPA finality lags ~2 blocks.** This is normal for a single-validator chain and does not affect the upgrade test.
- **OCEX worker and THEA errors in logs are expected.** These workers require session keys in the keystore that Alice doesn't have. They do not affect block production or migration execution.
