#!/usr/bin/env python3
"""Tests for the settle deadline in migrate_currency.py: a wave's inclusion and finality share one
SETTLE_TIMEOUT on a monotonic clock. A simulated node and clock stand in for the chain; nothing here
touches the network.

Run next to migrate_currency.py:  python3 -m unittest -v test_migrate_currency
"""
import contextlib
import io
import json
import os
import sys
import tempfile
import types
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

# migrate_currency imports substrate-interface at the top. These tests never build a SubstrateInterface,
# so a stub stands in for the package and it need not be installed.
if "substrateinterface" not in sys.modules:
    _si = types.ModuleType("substrateinterface")
    _si.Keypair = _si.KeypairType = _si.SubstrateInterface = None
    _exc = types.ModuleType("substrateinterface.exceptions")

    class SubstrateRequestException(Exception):
        pass

    _exc.SubstrateRequestException = SubstrateRequestException
    _si.exceptions = _exc
    sys.modules["substrateinterface"] = _si
    sys.modules["substrateinterface.exceptions"] = _exc

import migrate_currency as mc  # noqa: E402

UNIT = 10 ** 12
KP = types.SimpleNamespace(ss58_address="signer")


def bhash(n):
    return "0x%064x" % n


def every_6s(t):
    """A block every 6 s from #100, finality two blocks behind."""
    return 100 + int(t // 6), 98 + int(t // 6)


class SimTime:
    """A simulated monotonic clock that only sleep() moves. A wait that outlives `limit` raises, so a
    wait with no deadline fails the test instead of hanging it."""

    def __init__(self, limit=4 * mc.SETTLE_TIMEOUT):
        self.now, self.limit = 0.0, limit

    def clock(self):
        return self.now

    def sleep(self, seconds):
        self.now += seconds
        if self.now > self.limit:
            raise RuntimeError(f"still waiting after {self.now:,.0f} simulated seconds")


class ScaleBytes:
    def __init__(self, raw):
        self.data = raw

    def __str__(self):
        return "0x" + self.data.hex()


class FakeApi:
    """The part of SubstrateInterface that Sweep.send() uses."""

    def compose_call(self, module, function, params):
        return {"module": module, "function": function, "params": params}

    def create_signed_extrinsic(self, call, keypair, nonce, era):
        raw = json.dumps({"call": call, "signer": keypair.ss58_address, "nonce": nonce, "era": era},
                         sort_keys=True).encode()
        return types.SimpleNamespace(data=ScaleBytes(raw))


class FakeNode:
    """Stands in for Chain: a node that answers every request. heights(t) gives (best, finalized) at
    simulated time t; each new block takes up to per_block calls from the pool, in order, and applies
    them. Every stash starts on the staking lock (bond 100, free 100, ED 1, so migrate_currency trims 1)
    with the Controller payee."""

    def __init__(self, sim, heights, controllers=None, per_block=50):
        self.sim, self.heights, self.per_block = sim, heights, per_block
        self.controllers = controllers or {}  # stash -> controller, for the payee pass
        self.ed, self.decimals, self.symbol = UNIT, 12, "PDEX"
        self.s = FakeApi()
        self.best_n, self.fin_n = heights(sim.now)
        self.xts = {}        # block number -> [extrinsic hash]
        self.events = {}     # (block number, extrinsic index) -> events
        self.pool_ = {}      # extrinsic hash -> decoded extrinsic, in submission order
        self.drop = set()    # stashes whose next call the pool loses without including it
        self.migrated = {}   # stash -> block that migrated it
        self.payee_set = {}  # stash -> block that set Account(controller)
        self.nonce = 0       # the signer's nonce on chain
        self.submitted = []  # every extrinsic hash submitted

    def advance(self):
        best, fin = self.heights(self.sim.now)
        while self.best_n < best:
            self.best_n += 1
            self.xts[self.best_n] = []
            for h in list(self.pool_)[:self.per_block]:
                self.apply(self.best_n, h, self.pool_.pop(h))
        self.fin_n = max(self.fin_n, min(fin, self.best_n))

    def apply(self, n, h, xt):
        idx = len(self.xts[n])
        self.xts[n].append(h)
        self.nonce += 1
        fn, params = xt["call"]["function"], xt["call"]["params"]
        if fn == "migrate_currency":
            st = params["stash"]
            done, err = self.migrated, "Staking.AlreadyMigrated"
            evs = [("Staking", "CurrencyMigrated", {"stash": st, "force_withdraw": UNIT})]
        else:
            st = next(s for s, c in self.controllers.items() if c == params["controller"])
            done, err = self.payee_set, "Staking.NotController"
            evs = []
        ok = st not in done
        if ok:
            done[st] = n
            evs.append(("System", "ExtrinsicSuccess", {}))
        else:
            evs = [("System", "ExtrinsicFailed", {"dispatch_error": err})]
        evs.append(("TransactionPayment", "TransactionFeePaid", {"actual_fee": 0 if ok else 5}))
        self.events[(n, idx)] = [{"module_id": m, "event_id": e, "extrinsic_idx": idx, "attributes": a}
                                 for m, e, a in evs]

    # What Sweep calls on Chain.
    def best(self):
        self.advance()
        return self.best_n

    def finalized(self):
        self.advance()
        return self.fin_n, bhash(self.fin_n)

    def hash_at(self, n):
        return bhash(n)

    def block(self, h):
        n = int(h, 16)
        return {"number": n, "parent": bhash(n - 1), "xts": list(self.xts.get(n, []))}

    def xt_events(self, h, idx):
        return self.events.get((int(h, 16), idx), [])

    def pool(self):
        self.advance()
        return set(self.pool_)

    def rpc(self, method, params):
        if method == "system_accountNextIndex":
            return self.nonce + len(self.pool_)
        if method == "author_submitExtrinsic":
            raw = bytes.fromhex(params[0][2:])
            h = mc.xt_hash(raw)
            xt = json.loads(raw)
            self.submitted.append(h)
            st = xt["call"]["params"].get("stash")
            if st in self.drop:
                self.drop.discard(st)
            else:
                self.pool_[h] = xt
            return h
        raise AssertionError(f"unexpected rpc {method}")

    def amount(self, planck):
        return f"{planck / UNIT:,.4f}"

    def module_error(self, de):
        return de

    def snapshot(self, stashes, block_hash):
        n, out = int(block_hash, 16), {}
        for st in stashes:
            done = self.migrated.get(st, n + 1) <= n
            bond = 99 * UNIT if done else 100 * UNIT
            out[st] = {"free": UNIT if done else 100 * UNIT, "reserved": 99 * UNIT if done else 0,
                       "frozen": 0 if done else 100 * UNIT, "providers": 1, "consumers": 1 if done else 2,
                       "staking_lock": 0 if done else 100 * UNIT, "staking_hold": 99 * UNIT if done else 0,
                       "ledger": {"total": bond, "active": bond, "unlocking": 0, "chunks": 0}}
        return out

    def multi(self, entries, block_hash):
        n, out = int(block_hash, 16), {}
        for tag, _module, item, params in entries:
            st = params[0]
            if item == "Payee":
                done = self.payee_set.get(st, n + 1) <= n
                out[tag] = {"Account": self.controllers[st]} if done else "Controller"
            elif item == "Bonded":
                out[tag] = self.controllers[st]
            else:
                raise AssertionError(f"unexpected storage item {item}")
        return out


class SettleDeadlineTest(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.tmp = tmp.name
        self.out_path = os.path.join(self.tmp, "sweep.jsonl")

    def sweep(self, node, sim, kind="currency"):
        s = mc.Sweep(node, KP, self.out_path, validators=set(), min_bond=100 * UNIT,
                     clock=sim.clock, sleep=sim.sleep)
        self.addCleanup(s.out.close)
        if kind == "payee":
            s.kind, s.controllers = "payee", dict(node.controllers)
        return s

    def rows(self):
        with open(self.out_path) as f:
            return [json.loads(line) for line in f]

    def quiet(self, fn):
        with contextlib.redirect_stdout(io.StringIO()):
            return fn()

    def expect_stop(self, fn):
        """fn must stop the script: sys.exit(<message>), which exits with status 1."""
        out = io.StringIO()
        with contextlib.redirect_stdout(out), self.assertRaises(SystemExit) as cm:
            fn()
        self.assertIsInstance(cm.exception.code, str)
        self.assertIn("pending", cm.exception.code)
        return cm.exception.code, out.getvalue()

    def assert_stopped_at(self, sim, start, poll):
        self.assertGreaterEqual(sim.now, start + mc.SETTLE_TIMEOUT)
        self.assertLess(sim.now, start + mc.SETTLE_TIMEOUT + poll)

    def test_review_case_responsive_node_best_block_fixed_at_100(self):
        """The review's reproduction: best block fixed at 100 while the node answers, a pending call born
        at 100, from_n=101. Before the fix it was still waiting after 1,201 simulated seconds."""
        self.assertEqual(mc.SETTLE_TIMEOUT, 900)  # the 15 minutes in the README
        for entry in ("wait_inclusion", "settle"):
            with self.subTest(entry=entry):
                self.out_path = os.path.join(self.tmp, entry + ".jsonl")
                sim = SimTime()
                node = FakeNode(sim, lambda t: (100, 100))
                sweep = self.sweep(node, sim)
                tx = {"stash": "S1", "hash": "0x" + "ab" * 32, "nonce": 7, "birth": 100, "attempt": 1,
                      "kind": "currency", "wave": 3}
                sweep.open[tx["hash"]] = tx  # as send() leaves it
                sweep.fin_scanned = 99       # as run() leaves it
                msg, out = self.expect_stop(lambda: getattr(sweep, entry)([tx], 101))
                self.assert_stopped_at(sim, 0, 1)
                self.assertIn("inclusion stalled: best #100, 1 of 1 calls not in a block", msg)
                self.assertIn("1 calls pending at best #100", msg)
                self.assertIn(f"pending: currency wave 3 stash S1 nonce 7 hash {tx['hash']} best #100", out)
                [row] = self.rows()
                self.assertEqual({k: row[k] for k in ("pass", "stash", "status", "nonce", "extrinsic_hash",
                                                      "wave", "best_block", "attempt")},
                                 {"pass": "currency", "stash": "S1", "status": "pending", "nonce": 7,
                                  "extrinsic_hash": tx["hash"], "wave": 3, "best_block": 100, "attempt": 1})

    def test_normal_wave_is_included_finalized_and_recorded(self):
        sim = SimTime()
        node = FakeNode(sim, every_6s)
        sweep = self.sweep(node, sim)
        self.quiet(lambda: sweep.run(["S1", "S2"], 50))
        rows = self.rows()
        self.assertEqual([(r["stash"], r["status"], r["block"], r["nonce"]) for r in rows],
                         [("S1", "ok", 101, 0), ("S2", "ok", 101, 1)])
        self.assertTrue(all(r["force_withdraw"] == UNIT and r["fee"] == 0 and r["free_plus_reserved_unchanged"]
                            for r in rows))
        self.assertEqual(sweep.open, {})
        self.assertLess(sim.now, 60)

    def test_chain_that_resumes_before_the_deadline_continues_normally(self):
        def heights(t):  # best block stuck at #100 for 10 minutes, then a block every 6 s again
            if t < 600:
                return 100, 98
            k = int((t - 600) // 6)
            return 101 + k, 99 + k

        sim = SimTime()
        node = FakeNode(sim, heights)
        sweep = self.sweep(node, sim)
        self.quiet(lambda: sweep.run(["S1", "S2"], 1))  # two waves of one: the second goes after the stall
        self.assertEqual([(r["stash"], r["status"], r["block"]) for r in self.rows()],
                         [("S1", "ok", 101), ("S2", "ok", 104)])
        self.assertGreater(sim.now, 600)
        self.assertLess(sim.now, mc.SETTLE_TIMEOUT)

    def test_inclusion_and_finality_share_one_deadline(self):
        """Included after 10 minutes, then finality stalls: the stop comes 15 minutes after the wave was
        sent, not 15 minutes after its inclusion."""
        sim = SimTime()
        node = FakeNode(sim, lambda t: (100, 98) if t < 600 else (101, 98))
        sweep = self.sweep(node, sim)
        msg, _ = self.expect_stop(lambda: sweep.run(["S1"], 50))
        self.assert_stopped_at(sim, 0, 2)
        self.assertIn("finality stalled: finalized #98, waiting for #101", msg)
        [row] = self.rows()
        self.assertEqual((row["stash"], row["status"], row["nonce"], row["wave"], row["best_block"]),
                         ("S1", "pending", 0, 1, 101))

    def test_stop_records_finalized_calls_and_a_rerun_skips_what_is_done(self):
        """One call lands and is finalized, then the best block stalls with the other call in the pool.
        The stop records the first as ok and the second as pending; once blocks advance the pending call
        lands, and a re-run on the same JSONL skips both stashes and sends nothing."""
        sim = SimTime()
        node = FakeNode(sim, lambda t: (100, 98) if t < 6 else (101, 101), per_block=1)
        sweep = self.sweep(node, sim)
        msg, _ = self.expect_stop(lambda: sweep.run(["S1", "S2"], 50))
        self.assert_stopped_at(sim, 0, 1)
        self.assertIn("1 of 2 calls not in a block", msg)
        self.assertEqual([(r["stash"], r["status"], r.get("block"), r.get("best_block")) for r in self.rows()],
                         [("S1", "ok", 101, None), ("S2", "pending", None, 101)])

        t0 = sim.now
        node.heights = lambda t: (102 + int((t - t0) // 6), 100 + int((t - t0) // 6))
        sent = len(node.submitted)
        rerun = self.sweep(node, sim)
        self.quiet(lambda: rerun.run(["S1", "S2"], 50))
        self.assertEqual(len(node.submitted), sent)
        self.assertEqual(node.migrated, {"S1": 101, "S2": 102})
        rows = self.rows()
        self.assertEqual([(r["stash"], r["status"]) for r in rows[2:]], [("S1", "skipped"), ("S2", "skipped")])
        self.assertEqual({st: r["status"] for st, r in mc.final_rows(rows).items()},
                         {"S1": "ok", "S2": "skipped"})

    def test_resubmission_finality_wait_has_the_same_limit(self):
        """The pool loses the call; before resubmitting it the script waits for finality, which stalls.
        That wait stops at the same limit and records the lost call as pending."""
        sim = SimTime()
        node = FakeNode(sim, lambda t: (100 + int(t // 6), min(98 + int(t // 6), 101)))
        node.drop.add("S1")
        sweep = self.sweep(node, sim)
        msg, _ = self.expect_stop(lambda: sweep.run(["S1"], 50))
        self.assert_stopped_at(sim, 18, 2)  # the pool check finds the call gone at 18 s
        self.assertIn("finality stalled: finalized #101, waiting for #103", msg)
        [row] = self.rows()
        self.assertEqual((row["stash"], row["status"], row["nonce"], row["attempt"], row["wave"]),
                         ("S1", "pending", 0, 1, 1))

    def test_payee_pass_normal(self):
        sim = SimTime()
        node = FakeNode(sim, every_6s, controllers={"P1": "C1", "P2": "P2"})
        sweep = self.sweep(node, sim, kind="payee")
        self.quiet(lambda: sweep.run(["P1", "P2"], 50))
        self.assertEqual([(r["pass"], r["stash"], r["status"], r["block"], r["destination_unchanged"])
                          for r in self.rows()],
                         [("payee", "P1", "ok", 101, True), ("payee", "P2", "ok", 101, True)])
        self.assertLess(sim.now, 60)

    def test_payee_pass_stops_at_deadline_when_best_block_stalls(self):
        sim = SimTime()
        node = FakeNode(sim, lambda t: (100, 100), controllers={"P1": "C1"})
        sweep = self.sweep(node, sim, kind="payee")
        msg, out = self.expect_stop(lambda: sweep.run(["P1"], 50))
        self.assert_stopped_at(sim, 0, 1)
        self.assertIn("inclusion stalled: best #100", msg)
        self.assertIn("pending: payee wave 1 stash P1 nonce 0", out)
        [row] = self.rows()
        self.assertEqual((row["pass"], row["stash"], row["status"], row["nonce"], row["wave"], row["best_block"]),
                         ("payee", "P1", "pending", 0, 1, 100))
        self.assertEqual(row["extrinsic_hash"], node.submitted[0])


if __name__ == "__main__":
    unittest.main()
