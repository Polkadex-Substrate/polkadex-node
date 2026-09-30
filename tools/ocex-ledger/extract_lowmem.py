#!/usr/bin/env python3
# This file is part of Polkadex.
# Copyright (C) 2026 Polkadex contributors.
# SPDX-License-Identifier: GPL-3.0-or-later
"""Low-memory driver for extract_ocex_trie.py: builds an on-disk hash -> db-key index
(one pass, no values kept), then walks the trie with point lookups. Same CLI, plus --index."""
import os, sys, argparse, struct, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import extract_ocex_trie as x
from rocksdict import Rdict, Options

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

def build_index(db, cfs, index_path):
    marker = os.path.join(index_path, "COMPLETE")
    if os.path.exists(marker):
        n = int(open(marker).read().strip() or 0)
        print(f"reusing index at {index_path} ({n:,} nodes)")
        named = {}
        for cf in cfs:
            col = db.get_column_family(cf) if cf != "default" else db
            it = col.iter(); it.seek(x.FULL_PREFIX)
            while it.valid():
                k = it.key()
                if not k.startswith(x.FULL_PREFIX): break
                rest = k[len(x.FULL_PREFIX):]
                if rest in x.NAMED or len(rest) < 32: named[rest] = it.value()
                else: break  # named keys sort before the long node keys? not guaranteed; cheap full pass below
                it.next()
        return named, Rdict(index_path, access_type=__import__("rocksdict").AccessType.read_only()), n
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
    with open(marker, "w") as f: f.write(str(n))
    from rocksdict import AccessType
    return named, Rdict(index_path, access_type=AccessType.read_only()), n

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--index", default="ocex-index", help="directory for the on-disk hash index")
    known, rest = ap.parse_known_args()
    db_path = rest[rest.index("--db") + 1]
    db, cfs = x.open_db(db_path)
    print(f"opened {db_path} read-only; column families: {cfs}", flush=True)
    named, idx, n = build_index(db, cfs, known.index)
    lazy = LazyNodes(db, cfs, idx, n)
    x.load_ocex_keys = lambda _db, _cfs, verbose=True: (named, lazy)
    x.open_db = lambda _p: (db, cfs)
    sys.argv = [sys.argv[0]] + rest
    x.main()

if __name__ == "__main__":
    main()
