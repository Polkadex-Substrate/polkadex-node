#!/usr/bin/env python3
# This file is part of Polkadex.
#
# Copyright (c) 2026 the polkadex-node contributors.
# SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
#
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
# GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License
# along with this program. If not, see <https://www.gnu.org/licenses/>.
"""
Extract the Polkadex orderbook balance ledger from a validator node's RocksDB.

The OCEX offchain worker kept a Merkle trie of per-user balances in the node's
offchain storage (client DB column OFFCHAIN, keys prefixed
b"storage" + b"offchain-ocex::").  Trie nodes are stored as
SCALE (Vec<u8>, i32 refcount) under keys ending in the node's 32-byte blake2 hash.
Nothing is ever physically deleted, so any historical root can be reopened.

Layout verified against Polkadex-Substrate/Polkadex @ mainnet-release (the runtime
live at the 14 Dec 2024 halt) and identical in the 2025 node-upgrade repo.

Usage:
  python3 extract_ocex_trie.py --db /path/to/chains/polkadex_main_network/db/full --scan
  python3 extract_ocex_trie.py --db ... --out balances.csv
  python3 extract_ocex_trie.py --db ... --root 0x<on-chain state_hash of snapshot 694063> --out balances.csv

Read-only: the DB is opened with RocksDB read_only access and never written.
"""
import argparse
import csv
import hashlib
import struct
import sys
from collections import defaultdict
from decimal import Decimal

try:
    from rocksdict import Rdict, Options, AccessType
except ImportError:
    sys.exit("missing dependency: python3 -m pip install --user rocksdict")

try:
    from substrateinterface.utils.ss58 import ss58_encode
except ImportError:  # optional
    ss58_encode = None

STORAGE_PREFIX = b"storage"            # sp_offchain::STORAGE_PREFIX (PERSISTENT kind)
OCEX_PREFIX = b"offchain-ocex::"
FULL_PREFIX = STORAGE_PREFIX + OCEX_PREFIX
NAMED = {b"trie_root", b"state_info", b"snapshot_id", b"worker_status",
         b"hashed_null_node", b"null_node_data"}
STATE_INFO_KEY = OCEX_PREFIX + b"state_info"   # also stored *inside* the trie

KNOWN_ASSETS = {
    3496813586714279103986568049643838918: "USDT",
    304494718746685751324769169435167367843: "USDC",
    95930534000017180603917534864279132680: "DOT",
    119367686984583275840673742485354142551: "DED",
    339306133874233608313826294843504252047: "PINK",
}
SS58_POLKADEX = 88


# ---------------------------------------------------------------- SCALE helpers
def compact(data, pos):
    b0 = data[pos]
    mode = b0 & 3
    if mode == 0:
        return b0 >> 2, pos + 1
    if mode == 1:
        return struct.unpack_from("<H", data, pos)[0] >> 2, pos + 2
    if mode == 2:
        return struct.unpack_from("<I", data, pos)[0] >> 2, pos + 4
    n = (b0 >> 2) + 4
    return int.from_bytes(data[pos + 1:pos + 1 + n], "little"), pos + 1 + n


def decode_db_value(raw):
    """SCALE (Vec<u8>, i32) as written by storage.rs db_insert."""
    n, p = compact(raw, 0)
    val = raw[p:p + n]
    rc = struct.unpack_from("<i", raw, p + n)[0]
    return val, rc


def decode_decimal(data, pos):
    flags, hi, lo, mid = struct.unpack_from("<IIII", data, pos)
    mant = (hi << 64) | (mid << 32) | lo
    scale = (flags >> 16) & 0xFF
    neg = bool(flags & 0x8000_0000)
    d = Decimal(mant).scaleb(-scale)
    return (-d if neg else d), pos + 16


def decode_asset_id(data, pos):
    tag = data[pos]
    if tag == 0:
        return int.from_bytes(data[pos + 1:pos + 17], "little"), pos + 17
    if tag == 1:
        return "PDEX", pos + 1
    raise ValueError(f"bad AssetId tag {tag}")


def decode_balances(data):
    n, p = compact(data, 0)
    out = {}
    for _ in range(n):
        a, p = decode_asset_id(data, p)
        d, p = decode_decimal(data, p)
        out[a] = d
    if p != len(data):
        raise ValueError("trailing bytes in balance map")
    return out


def decode_state_info(data):
    last_block, worker_nonce, stid, snapshot_id = struct.unpack_from("<IQQQ", data, 0)
    return dict(last_block=last_block, worker_nonce=worker_nonce, stid=stid, snapshot_id=snapshot_id)


# ---------------------------------------------------------------- trie node codec (sp-trie LayoutV1)
EMPTY, LEAF, BRANCH, BRANCH_VAL, LEAF_HASHED, BRANCH_HASHED = range(6)


def decode_header(data):
    b = data[0]
    if b == 0:
        return EMPTY, 0, 1
    if b >> 6 == 0b01:
        kind, bits = LEAF, 6
    elif b >> 6 == 0b10:
        kind, bits = BRANCH, 6
    elif b >> 6 == 0b11:
        kind, bits = BRANCH_VAL, 6
    elif b >> 5 == 0b001:
        kind, bits = LEAF_HASHED, 5
    elif b >> 4 == 0b0001:
        kind, bits = BRANCH_HASHED, 4
    else:
        raise ValueError(f"unknown node header {b:#x}")
    maxv = (1 << bits) - 1
    n = b & maxv
    pos = 1
    if n == maxv:
        n -= 1
        while True:
            x = data[pos]
            pos += 1
            if x < 255:
                n += x + 1
                break
            n += 255
    return kind, n, pos


def decode_partial(data, pos, nnibbles):
    nbytes = (nnibbles + 1) // 2
    raw = data[pos:pos + nbytes]
    nibbles = []
    i = 0
    if nnibbles % 2 == 1:
        nibbles.append(raw[0] & 0x0F)
        i = 1
    for b in raw[i:]:
        nibbles.append(b >> 4)
        nibbles.append(b & 0x0F)
    return nibbles, pos + nbytes


def decode_node(data):
    """Returns (partial_nibbles, value, children) where
    value = ('inline', bytes) | ('hash', bytes32) | None
    children = list of 16 entries: None | ('hash', bytes32) | ('inline', bytes)"""
    kind, n, pos = decode_header(data)
    if kind == EMPTY:
        return [], None, None
    partial, pos = decode_partial(data, pos, n)
    if kind == LEAF:
        ln, pos = compact(data, pos)
        return partial, ("inline", data[pos:pos + ln]), None
    if kind == LEAF_HASHED:
        return partial, ("hash", data[pos:pos + 32]), None
    bitmap = struct.unpack_from("<H", data, pos)[0]
    pos += 2
    value = None
    if kind == BRANCH_VAL:
        ln, pos = compact(data, pos)
        value = ("inline", data[pos:pos + ln])
        pos += ln
    elif kind == BRANCH_HASHED:
        value = ("hash", data[pos:pos + 32])
        pos += 32
    children = [None] * 16
    for i in range(16):
        if bitmap & (1 << i):
            ln, pos = compact(data, pos)
            child = data[pos:pos + ln]
            pos += ln
            children[i] = ("hash", child) if ln == 32 else ("inline", child)
    return partial, value, children


def blake2_256(b):
    return hashlib.blake2b(b, digest_size=32).digest()


# ---------------------------------------------------------------- DB access
def open_db(path):
    cfs = Rdict.list_cf(path)
    # rocksdict auto-detects options + column families in RAW mode when no
    # Options object is given; read_only never touches the DB files.
    db = Rdict(path, access_type=AccessType.read_only())
    return db, cfs


def load_ocex_keys(db, cfs, verbose=True):
    """Return (named: {name: raw_value}, nodes: {hash32: (bytes, rc)})."""
    named, nodes = {}, {}
    for cf in cfs:
        col = db.get_column_family(cf) if cf != "default" else db
        it = col.iter()
        it.seek(FULL_PREFIX)
        count = 0
        while it.valid():
            k = it.key()
            if not k.startswith(FULL_PREFIX):
                break
            rest = k[len(FULL_PREFIX):]
            v = it.value()
            if rest in NAMED or len(rest) < 32:
                named[rest] = v
            else:
                h = rest[-32:]
                try:
                    val, rc = decode_db_value(v)
                except Exception:
                    val, rc = v, None
                # same hash => same content; keep the copy with the highest refcount
                prev = nodes.get(h)
                if prev is None or (rc or 0) > (prev[1] or 0):
                    nodes[h] = (val, rc)
            count += 1
            it.next()
        if verbose and count:
            print(f"  column {cf}: {count:,} offchain-ocex keys")
    return named, nodes


# ---------------------------------------------------------------- traversal
def walk(nodes, root, verify=True):
    """Yield (key_bytes, value_bytes). Raises on missing node / hash mismatch."""
    stack = [(root, [])]  # (node ref, path nibbles)
    stats = defaultdict(int)
    while stack:
        ref, path = stack.pop()
        kind, payload = ref
        if kind == "hash":
            entry = nodes.get(payload)
            if entry is None:
                stats["missing_nodes"] += 1
                raise KeyError(f"trie node {payload.hex()} not found in DB")
            data = entry[0]
            if verify and blake2_256(data) != payload:
                raise ValueError(f"hash mismatch for node {payload.hex()}")
            stats["nodes"] += 1
        else:
            data = payload
            stats["inline_nodes"] += 1
        partial, value, children = decode_node(data)
        full = path + partial
        if value is not None:
            vkind, vpayload = value
            if vkind == "hash":
                entry = nodes.get(vpayload)
                if entry is None:
                    raise KeyError(f"value node {vpayload.hex()} not found in DB")
                vbytes = entry[0]
                if verify and blake2_256(vbytes) != vpayload:
                    raise ValueError(f"hash mismatch for value {vpayload.hex()}")
                stats["hashed_values"] += 1
            else:
                vbytes = vpayload
            if len(full) % 2:
                raise ValueError("odd nibble count at leaf")
            key = bytes((full[i] << 4) | full[i + 1] for i in range(0, len(full), 2))
            yield key, vbytes, stats
        if children:
            for i in range(15, -1, -1):
                if children[i] is not None:
                    stack.append((children[i], full + [i]))


def to_ss58(pubkey):
    if ss58_encode is None:
        return ""
    return ss58_encode(pubkey.hex(), SS58_POLKADEX)


# ---------------------------------------------------------------- main
def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--db", required=True, help="path to .../chains/polkadex_main_network/db/full")
    ap.add_argument("--scan", action="store_true", help="only report what the DB contains")
    ap.add_argument("--root", help="0x-hex trie root to open (default: stored trie_root pointer)")
    ap.add_argument("--out", default="ocex_balances.csv")
    ap.add_argument("--no-verify", action="store_true", help="skip per-node hash verification")
    args = ap.parse_args()

    db, cfs = open_db(args.db)
    print(f"opened {args.db} read-only; column families: {cfs}")
    named, nodes = load_ocex_keys(db, cfs)
    groups = defaultdict(int)
    for k in named:
        stem = k.split(b"\x00", 1)[0]
        groups[stem.decode("ascii", "replace")] += 1
    print(f"named keys by stem: {dict(groups)}")
    for k in (b"trie_root", b"state_info", b"worker_status", b"snapshot_id"):
        if k in named:
            print(f"  {k.decode()} = 0x{named[k].hex()}")
    print(f"trie/value nodes loaded: {len(nodes):,}")

    stored_root = named.get(b"trie_root")
    if stored_root is not None:
        print(f"stored trie_root pointer: 0x{stored_root.hex()}")
    if b"snapshot_id" in named:
        print(f"stored snapshot_id key: 0x{named[b'snapshot_id'].hex()}")

    # Triage: is the halt-state root (on-chain Snapshots(694063).state_hash) in this DB?
    HALT_ROOT = bytes.fromhex("0899feaee1f47b34036109f56c76b5fbd22ec9e339ecc57fa0bcc58f6a2f05c0")
    print(f"halt root 0x{HALT_ROOT.hex()[:16]}... present in this DB: {HALT_ROOT in nodes}")

    if args.scan:
        return

    if args.root:
        root = bytes.fromhex(args.root.removeprefix("0x"))
    elif stored_root and stored_root != bytes(32):
        root = stored_root
    else:
        sys.exit("trie_root pointer is empty/zero; pass --root <on-chain state_hash of snapshot 694063>")
    print(f"opening trie at root 0x{root.hex()}")

    accounts, other, state_info = {}, {}, None
    stats = None
    for key, val, stats in walk(nodes, ("hash", root), verify=not args.no_verify):
        if key == STATE_INFO_KEY:
            state_info = decode_state_info(val)
        elif len(key) == 32:
            try:
                accounts[key] = decode_balances(val)
            except Exception as e:
                other[key] = (val, f"balance decode failed: {e}")
        else:
            other[key] = (val, "non-account key")

    if stats is None:
        sys.exit("trie at this root contains no entries; check --root against the on-chain state hash")
    print(f"\ntraversal ok: {dict(stats)}")
    print(f"state_info inside trie: {state_info}")
    print(f"accounts with balances: {len(accounts):,}; other keys: {len(other):,}")

    totals = defaultdict(Decimal)
    holders = defaultdict(int)
    with open(args.out, "w", newline="") as f:
        w = csv.writer(f)
        w.writerow(["account_ss58_polkadex", "account_hex", "asset", "asset_id", "balance"])
        for acct in sorted(accounts):
            for asset, bal in sorted(accounts[acct].items(), key=lambda x: str(x[0])):
                name = "PDEX" if asset == "PDEX" else KNOWN_ASSETS.get(asset, "")
                w.writerow([to_ss58(acct), "0x" + acct.hex(), name, asset, f"{bal.normalize():f}"])
                if bal != 0:
                    totals[name or asset] += bal
                    holders[name or asset] += 1
    print(f"\nwrote {args.out}")
    print(f"\n{'asset':>12} {'holders':>8} {'total':>24}")
    for k in sorted(totals, key=lambda x: str(x)):
        print(f"{str(k)[:12]:>12} {holders[k]:>8} {totals[k]:>24,.6f}")
    print("\nCompare these totals with the orderbook custody balances recorded on chain at the halt block.")
    if other:
        print(f"\nnon-account keys (first 5): " + ", ".join(k.hex()[:24] for k in list(other)[:5]))


if __name__ == "__main__":
    main()
