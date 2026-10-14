#!/usr/bin/env python3
# This file is part of Polkadex.
# Copyright (C) 2026 Polkadex contributors.
# SPDX-License-Identifier: GPL-3.0-or-later
"""Tests for the ledger extractors on synthetic tries. Run: python3 -m unittest test_extract"""
import contextlib, csv, io, os, random, struct, sys, tempfile, unittest
from decimal import Decimal
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import extract_ocex_trie as x
import extract_lowmem as lowmem
from rocksdict import Rdict, Options

MAX96 = 2**96 - 1  # 79228162514264337593543950335, 29 digits
USDT = 3496813586714279103986568049643838918


# ---------------------------------------------------------------- encoders, the inverse of the decoders
def enc_compact(n):
    if n < 1 << 6: return bytes([n << 2])
    if n < 1 << 14: return struct.pack("<H", (n << 2) | 1)
    if n < 1 << 30: return struct.pack("<I", (n << 2) | 2)
    b = n.to_bytes((n.bit_length() + 7) // 8, "little")
    return bytes([((len(b) - 4) << 2) | 3]) + b

def enc_decimal(mant, scale, neg=False):
    flags = (scale << 16) | (0x8000_0000 if neg else 0)
    return struct.pack("<IIII", flags, mant >> 64, mant & 0xFFFF_FFFF, (mant >> 32) & 0xFFFF_FFFF)

def enc_balances(items):  # [(asset, mant, scale, neg)], asset "PDEX" or a u128 id
    out = enc_compact(len(items))
    for asset, mant, scale, neg in items:
        out += b"\x01" if asset == "PDEX" else b"\x00" + asset.to_bytes(16, "little")
        out += enc_decimal(mant, scale, neg)
    return out

def enc_header(prefix, bits, n):
    maxv = (1 << bits) - 1
    if n < maxv: return bytes([prefix | n])
    out, rem = [prefix | maxv], n - maxv
    while rem >= 255: out.append(255); rem -= 255
    return bytes(out + [rem])

def enc_partial(nibbles):
    out = bytearray([nibbles[0]] if len(nibbles) % 2 else [])
    rest = nibbles[len(nibbles) % 2:]
    out += bytes((rest[i] << 4) | rest[i + 1] for i in range(0, len(rest), 2))
    return bytes(out)

def nibbles_of(key):
    return [n for b in key for n in (b >> 4, b & 0x0F)]

def make_trie(entries):
    """entries: {32-byte key: value}, first nibbles distinct. A root branch over one leaf per
    entry. Returns (root hash, {hash: (node bytes, refcount)})."""
    nodes, children = {}, [None] * 16
    for key, value in entries.items():
        nib = nibbles_of(key)
        leaf = enc_header(0b01 << 6, 6, len(nib) - 1) + enc_partial(nib[1:]) + enc_compact(len(value)) + value
        h = x.blake2_256(leaf); nodes[h] = (leaf, 1)
        assert children[nib[0]] is None, "test keys need distinct first nibbles"
        children[nib[0]] = h
    bitmap = sum(1 << i for i, c in enumerate(children) if c is not None)
    root = enc_header(0b10 << 6, 6, 0) + struct.pack("<H", bitmap) + b"".join(enc_compact(32) + c for c in children if c)
    rh = x.blake2_256(root); nodes[rh] = (root, 1)
    return rh, nodes

def acct(first_byte):
    return bytes([first_byte]) + bytes(range(1, 32))


def run_main(argv, named, nodes):
    """x.main() against in-memory trie nodes. Returns (exit message or None, stdout)."""
    out = io.StringIO()
    with mock.patch.object(x, "open_db", lambda _p: (None, [])), \
         mock.patch.object(x, "load_ocex_keys", lambda _db, _cfs, verbose=True: (named, nodes)), \
         mock.patch.object(sys, "argv", ["extract_ocex_trie.py", "--db", "unused"] + argv), \
         contextlib.redirect_stdout(out):
        try:
            x.main()
        except SystemExit as e:
            return str(e.code), out.getvalue()
    return None, out.getvalue()

def read_rows(path):
    with open(path, newline="") as f:
        return {(r["account_hex"], r["asset"] or r["asset_id"]): r["balance"] for r in csv.DictReader(f)}


class Decimals(unittest.TestCase):
    def test_29_digits_are_kept(self):
        d, _ = x.decode_decimal(enc_decimal(MAX96, 28), 0)
        self.assertEqual(x.format_decimal(d), "7.9228162514264337593543950335")
        d, _ = x.decode_decimal(enc_decimal(MAX96, 28, neg=True), 0)
        self.assertEqual(x.format_decimal(d), "-7.9228162514264337593543950335")
        # what the previous decoding produced
        self.assertEqual(str(Decimal(MAX96).scaleb(-28)), "7.922816251426433759354395034")

    def test_scale_above_28_is_rejected(self):
        with self.assertRaises(ValueError):
            x.decode_decimal(enc_decimal(1, 29), 0)

    def test_same_output_as_before_up_to_28_digits(self):
        # The published CSV was written with Decimal(m).scaleb(-s) and normalize(), exact up
        # to 28 digits. The new code must produce the same strings for all of those.
        def before(mant, scale, neg):
            d = Decimal(mant).scaleb(-scale)
            return f"{(-d if neg else d).normalize():f}"
        rng = random.Random(7)
        cases = [(0, s, n) for s in (0, 8, 18, 28) for n in (False, True)]
        cases += [(rng.randrange(10 ** rng.randint(1, 28)), rng.randint(0, 28), rng.random() < 0.5) for _ in range(20000)]
        cases += [(m * 10 ** k, s, False) for m in (1, 12, 105) for k in (0, 3, 9) for s in (0, 2, 18)]
        for mant, scale, neg in cases:
            d, _ = x.decode_decimal(enc_decimal(mant, scale, neg), 0)
            self.assertEqual(x.format_decimal(d), before(mant, scale, neg), (mant, scale, neg))


class Export(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.out = os.path.join(self.tmp.name, "balances.csv")
    def tearDown(self):
        self.tmp.cleanup()

    def test_29_digit_balances_through_csv_and_totals(self):
        a, b, c = acct(0x10), acct(0x20), acct(0x30)
        root, nodes = make_trie({
            a: enc_balances([(USDT, MAX96, 28, False), ("PDEX", 150, 2, False)]),
            b: enc_balances([(USDT, MAX96, 28, True)]),
            c: enc_balances([(USDT, MAX96, 0, False)]),
        })
        err, log = run_main(["--root", "0x" + root.hex(), "--out", self.out], {}, nodes)
        self.assertIsNone(err, log)
        rows = read_rows(self.out)
        self.assertEqual(rows[("0x" + a.hex(), "USDT")], "7.9228162514264337593543950335")
        self.assertEqual(rows[("0x" + a.hex(), "PDEX")], "1.5")
        self.assertEqual(rows[("0x" + b.hex(), "USDT")], "-7.9228162514264337593543950335")
        self.assertEqual(rows[("0x" + c.hex(), "USDT")], "79228162514264337593543950335")
        # sum of the three USDT balances, exact; 28-digit arithmetic printed ...950,340.000000
        self.assertIn("79,228,162,514,264,337,593,543,950,335.000000", log)
        self.assertFalse(os.path.exists(self.out + ".partial"))

    def test_undecodable_account_stops_the_export(self):
        good, bad = acct(0x10), acct(0x20)
        root, nodes = make_trie({
            good: enc_balances([("PDEX", 5, 0, False)]),
            bad: enc_compact(1) + b"\x02",  # AssetId tag 2 does not exist
        })
        err, log = run_main(["--root", "0x" + root.hex(), "--out", self.out], {}, nodes)
        self.assertIn("1 account(s) could not be decoded", err)
        self.assertIn("0x" + bad.hex(), log)
        self.assertIn("bad AssetId tag 2", log)
        self.assertFalse(os.path.exists(self.out))
        self.assertFalse(os.path.exists(self.out + ".partial"))

    def test_bad_scale_stops_the_export(self):
        root, nodes = make_trie({acct(0x10): enc_compact(1) + b"\x01" + enc_decimal(1, 29)})
        err, log = run_main(["--root", "0x" + root.hex(), "--out", self.out], {}, nodes)
        self.assertIn("could not be decoded", err)
        self.assertIn("decimal scale 29", log)
        self.assertFalse(os.path.exists(self.out))


class LowMemIndex(unittest.TestCase):
    """extract_lowmem.py against a real RocksDB built in a temporary directory."""
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.db_path = os.path.join(self.tmp.name, "db")
        self.index = os.path.join(self.tmp.name, "index")
        self.out = os.path.join(self.tmp.name, "balances.csv")
        self.a = acct(0x10)
        self.root, nodes = make_trie({self.a: enc_balances([("PDEX", 42, 0, False)])})
        db = Rdict(self.db_path, Options(raw_mode=True))
        for h, (node, rc) in nodes.items():
            # A zero byte before the hash makes every node key sort before "trie_root".
            db[x.FULL_PREFIX + b"\x00" + h] = enc_compact(len(node)) + node + struct.pack("<i", rc)
        db[x.FULL_PREFIX + b"trie_root"] = self.root
        db[x.FULL_PREFIX + b"worker_status"] = b"\x00"
        db.close()

    def tearDown(self):
        self.tmp.cleanup()

    def run_lowmem(self, *argv):
        out = io.StringIO()
        with mock.patch.object(x, "open_db", x.open_db), mock.patch.object(x, "load_ocex_keys", x.load_ocex_keys), \
             mock.patch.object(sys, "argv", ["extract_lowmem.py", "--db", self.db_path, "--index", self.index] + list(argv)), \
             contextlib.redirect_stdout(out):
            try:
                lowmem.main()
            except SystemExit as e:
                return str(e.code), out.getvalue()
        return None, out.getvalue()

    def test_reused_index_keeps_named_keys(self):
        err, log = self.run_lowmem("--scan")
        self.assertIsNone(err, log)
        self.assertIn("trie_root = 0x" + self.root.hex(), log)
        # Second run reuses the index, with no --root: the stored trie_root must be found.
        err, log = self.run_lowmem("--out", self.out)
        self.assertIsNone(err, log)
        self.assertIn("reusing index", log)
        self.assertIn("stored trie_root pointer: 0x" + self.root.hex(), log)
        self.assertEqual(read_rows(self.out)[("0x" + self.a.hex(), "PDEX")], "42")

    def test_read_only_open_leaves_the_fingerprint_unchanged(self):
        before = lowmem.db_fingerprint(self.db_path)
        db, _ = x.open_db(self.db_path); db.close()
        self.assertEqual(lowmem.db_fingerprint(self.db_path), before)

    def test_index_is_refused_after_the_database_changes(self):
        self.assertIsNone(self.run_lowmem("--scan")[0])
        db = Rdict(self.db_path, Options(raw_mode=True)); db[b"other"] = b"1"; db.close()
        err, _ = self.run_lowmem("--scan")
        self.assertIn("is a different one or has changed since", err)

    def test_index_from_the_earlier_version_is_refused(self):
        os.makedirs(self.index)
        with open(os.path.join(self.index, "COMPLETE"), "w") as f: f.write("5")
        err, _ = self.run_lowmem("--scan")
        self.assertIn("earlier version of this script", err)

    def test_incomplete_index_is_refused(self):
        os.makedirs(self.index)
        with open(os.path.join(self.index, "000001.log"), "w"): pass
        err, _ = self.run_lowmem("--scan")
        self.assertIn("holds no complete index", err)


if __name__ == "__main__":
    unittest.main()
