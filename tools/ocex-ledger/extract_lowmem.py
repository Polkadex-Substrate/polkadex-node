#!/usr/bin/env python3
# This file is part of Polkadex.
# Copyright (C) 2026 Polkadex contributors.
# SPDX-License-Identifier: GPL-3.0-or-later
"""Low-memory driver for extract_ocex_trie.py: builds an on-disk hash -> db-key index
(one pass, no values kept), then walks the trie with point lookups. Same CLI, plus --index."""
import os, sys, argparse, hashlib, json, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import extract_ocex_trie as x
from rocksdict import Rdict, Options, AccessType

class LazyNodes:
    def __init__(self, db, cfs, idx, count):
        self.db, self.idx, self.count = db, idx, count
        self.cols = {cf: (db.get_column_family(cf) if cf != "default" else db) for cf in cfs}
    def _locate(self, h):
        loc = self.idx.get(h)
        if loc is None: return None
        cf, key = loc.split(b"\x00", 1)
        return self.cols[cf.decode()], key
    def get(self, h):
        loc = self._locate(h)
        if loc is None: return None
        col, key = loc
        v = col.get(key)
        if v is None: return None
        try: return x.decode_db_value(v)
        except Exception: return (v, None)
    def __contains__(self, h): return self.idx.get(h) is not None
    def __len__(self): return self.count

def db_fingerprint(db_path):
    """Changes whenever the database does: the IDENTITY and CURRENT files, and the name and
    size of every table, write-ahead log and manifest file. The info LOG files are left out
    because opening the database writes to them."""
    h = hashlib.sha256()
    for name in sorted(os.listdir(db_path)):
        p = os.path.join(db_path, name)
        if name in ("IDENTITY", "CURRENT"):
            with open(p, "rb") as f: h.update(name.encode() + b"\0" + f.read() + b"\0")
        elif name.endswith((".sst", ".log", ".blob")) or name.startswith("MANIFEST-"):
            h.update(f"{name}\0{os.path.getsize(p)}\0".encode())
    return h.hexdigest()

def build_index(db, cfs, index_path, db_path, fingerprint):
    marker = os.path.join(index_path, "COMPLETE")
    if os.path.exists(marker):
        with open(marker) as f:
            try: meta = json.load(f)
            except ValueError: meta = None
        if not isinstance(meta, dict):
            sys.exit(f"the index at {index_path} was made by an earlier version of this script and does not "
                     f"record its database; remove it or pass another --index")
        if meta["fingerprint"] != fingerprint:
            sys.exit(f"the index at {index_path} was built from {meta['db']}, and the database at {db_path} is a "
                     f"different one or has changed since; remove the index or pass another --index. A running "
                     f"node changes its database, so stop it or work on a copy")
        # Named keys come from the index, not from a second scan: node keys can sort before
        # them in the database, so a scan that stops early misses them.
        named = {bytes.fromhex(k): bytes.fromhex(v) for k, v in meta["named"].items()}
        print(f"reusing index at {index_path} ({meta['nodes']:,} nodes, {len(named):,} named keys)")
        return named, Rdict(index_path, access_type=AccessType.read_only()), meta["nodes"]
    if os.path.isdir(index_path) and os.listdir(index_path):
        sys.exit(f"{index_path} exists but holds no complete index, probably from an interrupted run; "
                 f"remove it or pass another --index")
    opts = Options(); opts.create_if_missing(True); opts.set_write_buffer_size(256 << 20); opts.set_max_write_buffer_number(4)
    idx = Rdict(index_path, options=opts)
    named, n, t0 = {}, 0, time.time()
    for cf in cfs:
        col = db.get_column_family(cf) if cf != "default" else db
        it = col.iter(); it.seek(x.FULL_PREFIX); c = 0
        while it.valid():
            k = it.key()
            if not k.startswith(x.FULL_PREFIX): break
            rest = k[len(x.FULL_PREFIX):]
            if rest in x.NAMED or len(rest) < 32:
                named[rest] = it.value()
            else:
                h = rest[-32:]
                if idx.get(h) is None:
                    idx[h] = cf.encode() + b"\x00" + k
                    n += 1
            c += 1
            if c % 2_000_000 == 0:
                print(f"  column {cf}: {c:,} keys scanned, {n:,} unique nodes indexed, {time.time()-t0:.0f}s", flush=True)
            it.next()
        if c: print(f"  column {cf}: {c:,} offchain-ocex keys", flush=True)
    idx.flush(); idx.close()
    meta = dict(db=os.path.abspath(db_path), fingerprint=fingerprint, nodes=n,
                named={k.hex(): v.hex() for k, v in named.items()})
    with open(marker, "w") as f: json.dump(meta, f)
    return named, Rdict(index_path, access_type=AccessType.read_only()), n

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--index", default="ocex-index", help="directory for the on-disk hash index")
    known, rest = ap.parse_known_args()
    db_path = rest[rest.index("--db") + 1]
    fingerprint = db_fingerprint(db_path)
    db, cfs = x.open_db(db_path)
    print(f"opened {db_path} read-only; column families: {cfs}", flush=True)
    named, idx, n = build_index(db, cfs, known.index, db_path, fingerprint)
    lazy = LazyNodes(db, cfs, idx, n)
    x.load_ocex_keys = lambda _db, _cfs, verbose=True: (named, lazy)
    x.open_db = lambda _p: (db, cfs)
    sys.argv = [sys.argv[0]] + rest
    x.main()

if __name__ == "__main__":
    main()
