#!/usr/bin/env python3
"""Move bonded PDEX from the old "staking " lock to the staking hold with Staking.migrate_currency.

Runtime 392 keeps staked funds on a fungible hold. A stash moves from the lock to the hold lazily, at
its first ledger update, and a hold must leave the existential deposit (ED) free. A stash with
free - ED < ledger.total cannot hold its whole bond, so any ledger update on it fails with
Staking.NotEnoughFunds. payout_stakers writes the validator's ledger before paying anyone, so a
payout for such a validator fails for the validator and for all of its nominators.

Staking.migrate_currency(stash) is permissionless. It swaps the lock for a hold, trims the part of
the bond that cannot be held (Staking.CurrencyMigrated.force_withdraw; that amount stays in the
stash as free balance) and waives the fee when it succeeds.

This script submits only Staking.migrate_currency calls. By default it targets every stash still on
the staking lock that cannot hold its full bond; --all-locked targets every stash still on the lock.
Re-running it is safe: a stash that is already migrated is no longer a target.

With --payee it then runs a second pass that submits only Staking.update_payee calls: every stash
whose Staking.Payee is still the deprecated Controller gets Account(controller), the account its
rewards already go to. Paying such a stash makes pallet-staking log a runtime::defensive error on
every node; update_payee is permissionless and waives the fee when it succeeds.

See README.md next to this file for the procedure.
"""
import argparse
import hashlib
import json
import re
import sys
import time
from collections import Counter

from substrateinterface import Keypair, KeypairType, SubstrateInterface
from substrateinterface.exceptions import SubstrateRequestException

STAKING_LOCK = "0x" + b"staking ".hex()
MORTAL_PERIOD = 64        # blocks a signed call stays valid; a dropped call can never land after that
STALL_BLOCKS = 4          # blocks without a new inclusion before the pool is checked for dropped calls
FINALITY_TIMEOUT = 900    # seconds to wait for finality before giving up
MAX_ATTEMPTS = 3          # first submission plus resubmissions of a dropped call
CHUNK = 400               # storage keys per state_queryStorageAt request
SKIP = {  # why a stash is skipped: (before its wave, before resubmitting a dropped call)
    "currency": ("no staking lock before submission (already migrated)",
                 "call dropped; stash no longer on the staking lock before resubmission"),
    "payee": ("payee no longer Controller before submission",
              "call dropped; payee no longer Controller before resubmission"),
}


def utc():
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def log(msg):
    print(f"{utc()} {msg}", flush=True)


def xt_hash(raw):
    return "0x" + hashlib.blake2b(raw, digest_size=32).hexdigest()


def lock_id(x):
    return x if isinstance(x, str) else "0x" + bytes(x).hex()


def is_staking_hold(hold_id):
    # RuntimeHoldReason::Staking(pallet_staking::HoldReason::Staking) decodes as {"Staking": "Staking"}
    return hold_id == "Staking" or (isinstance(hold_id, dict) and "Staking" in hold_id)


def account_view(acct, locks, holds, ledger):
    d = acct["data"]
    v = {"free": int(d["free"]), "reserved": int(d["reserved"]), "frozen": int(d["frozen"]),
         "providers": acct["providers"], "consumers": acct["consumers"],
         "staking_lock": sum(int(x["amount"]) for x in locks or [] if lock_id(x["id"]) == STAKING_LOCK),
         "staking_hold": sum(int(x["amount"]) for x in holds or [] if is_staking_hold(x["id"])),
         "ledger": None}
    if ledger:
        v["ledger"] = {"total": int(ledger["total"]), "active": int(ledger["active"]),
                       "unlocking": sum(int(c["value"]) for c in ledger["unlocking"]),
                       "chunks": len(ledger["unlocking"])}
    return v


def expect(v, ed, is_validator, min_bond):
    """What migrate_currency will do to this stash (pallet-staking do_migrate_currency)."""
    led = v["ledger"]
    max_hold = max(0, v["free"] - ed) + v["staking_hold"]
    trim = max(0, led["total"] - max_hold)
    active_after = led["active"] - min(led["active"], trim)
    below = is_validator and active_after < min_bond
    return {"max_hold": max_hold, "expected_trim": trim, "active_after": active_after,
            "below_min_after": below, "top_up": (min_bond - active_after) if below else 0}


class Chain:
    def __init__(self, url):
        self.s = SubstrateInterface(url=url)
        self.ss58 = self.s.ss58_format
        self.decimals = self.s.token_decimals or 12
        self.symbol = self.s.token_symbol or ""
        self.ed = int(self.s.get_constant("Balances", "ExistentialDeposit").value)
        self.blocks = {}
        self.events = {}

    def amount(self, planck):
        return f"{planck / 10 ** self.decimals:,.4f}"

    def rpc(self, method, params):
        return self.s.rpc_request(method, params)["result"]

    def best(self):
        return int(self.rpc("chain_getHeader", [])["number"], 16)

    def finalized(self):
        h = self.rpc("chain_getFinalizedHead", [])
        return int(self.rpc("chain_getHeader", [h])["number"], 16), h

    def hash_at(self, n):
        return self.rpc("chain_getBlockHash", [n])

    def block(self, h):
        b = self.blocks.get(h)
        if b is None:
            raw = self.rpc("chain_getBlock", [h])["block"]
            b = {"number": int(raw["header"]["number"], 16), "parent": raw["header"]["parentHash"],
                 "xts": [xt_hash(bytes.fromhex(x[2:])) for x in raw["extrinsics"]]}
            self.blocks[h] = b
        return b

    def xt_events(self, h, idx):
        if h not in self.events:
            self.events[h] = [e.value for e in self.s.get_events(block_hash=h)]
        return [e for e in self.events[h] if e.get("extrinsic_idx") == idx]

    def pool(self):
        return {xt_hash(bytes.fromhex(x[2:])) for x in self.rpc("author_pendingExtrinsics", [])}

    def query(self, module, item, params, block_hash):
        return self.s.query(module, item, params, block_hash=block_hash).value

    def keys(self, module, item, block_hash):
        return {k.value for k, _ in self.s.query_map(module, item, page_size=1000, block_hash=block_hash)}

    def multi(self, entries, block_hash):
        """entries: [(tag, module, item, params)] -> {tag: value}"""
        out = {}
        for i in range(0, len(entries), CHUNK):
            part = entries[i:i + CHUNK]
            keys = [self.s.create_storage_key(m, it, p) for _, m, it, p in part]
            tags = {k.to_hex(): e[0] for k, e in zip(keys, part)}
            for k, v in self.s.query_multi(keys, block_hash=block_hash):
                out[tags[k.to_hex()]] = v.value
        return out

    def snapshot(self, stashes, block_hash):
        entries = []
        for st in stashes:
            entries += [((st, "a"), "System", "Account", [st]), ((st, "l"), "Balances", "Locks", [st]),
                        ((st, "h"), "Balances", "Holds", [st]), ((st, "b"), "Staking", "Bonded", [st])]
        raw = self.multi(entries, block_hash)
        ctrl = {st: raw[(st, "b")] for st in stashes if raw.get((st, "b"))}
        ledgers = self.multi([(st, "Staking", "Ledger", [c]) for st, c in ctrl.items()], block_hash)
        return {st: account_view(raw[(st, "a")], raw[(st, "l")], raw[(st, "h")], ledgers.get(st))
                for st in stashes}

    def module_error(self, de):
        if isinstance(de, dict) and "Module" in de:
            m = de["Module"]
            idx, err = (m[0], m[1]) if isinstance(m, (list, tuple)) else (m["index"], m["error"])
            if isinstance(err, str):
                err = int(err[2:4], 16)  # [u8; 4], the first byte is the error index
            try:
                pallet = next(p.value["name"] for p in self.s.metadata.pallets if p.value["index"] == idx)
                return f"{pallet}.{self.s.metadata.get_module_error(module_index=idx, error_index=err).name}"
            except Exception:
                return f"Module({idx},{err})"
        return json.dumps(de, default=str)


def load_signer(path, ss58):
    """The file holds a mnemonic, a secret URI or a 0x seed. Its content is never printed."""
    try:
        with open(path) as f:
            secret = f.read().strip()
        if re.fullmatch(r"0x[0-9a-fA-F]{64}", secret):
            kp = Keypair.create_from_seed(secret, ss58_format=ss58, crypto_type=KeypairType.SR25519)
        else:
            kp = Keypair.create_from_uri(secret, ss58_format=ss58, crypto_type=KeypairType.SR25519)
    except Exception:
        kp = None
    secret = None
    if kp is None:
        sys.exit("cannot load an sr25519 key from --signer-file (expected a mnemonic, a secret URI or a 0x seed)")
    return kp


def plan(c, block_hash, all_locked):
    stashes = sorted(c.keys("Staking", "Bonded", block_hash))
    validators = c.keys("Staking", "Validators", block_hash)
    nominators = c.keys("Staking", "Nominators", block_hash)
    min_bond = int(c.query("Staking", "MinValidatorBond", [], block_hash))
    snap = c.snapshot(stashes, block_hash)
    stats = Counter()
    rows = []
    for st in stashes:
        v = snap[st]
        stats["bonded_stashes"] += 1
        if v["ledger"] is None:
            stats["bonded_without_ledger"] += 1
            continue
        if v["staking_lock"] == 0:
            stats["not_on_staking_lock"] += 1
            continue
        stats["on_staking_lock"] += 1
        e = expect(v, c.ed, st in validators, min_bond)
        if e["expected_trim"] > 0:
            stats["cannot_hold_full_bond"] += 1
        elif not all_locked:
            continue
        role = "validator" if st in validators else "nominator" if st in nominators else "idle"
        rows.append({"stash": st, "role": role, "bonded": v["ledger"]["total"], "active": v["ledger"]["active"],
                     "free": v["free"], "staking_lock": v["staking_lock"], "staking_hold": v["staking_hold"],
                     "lock_matches_ledger": v["staking_lock"] == v["ledger"]["total"], **e})
    rows.sort(key=lambda r: (r["role"] != "validator", -r["expected_trim"], r["stash"]))
    return rows, stats, validators, min_bond


def print_plan(c, rows, stats, min_bond, block_n, all_locked):
    print(f"# plan at finalized #{block_n}: ED {c.amount(c.ed)} {c.symbol}, MinValidatorBond {c.amount(min_bond)} {c.symbol}")
    print("# stash role bonded active free expected_trim active_after below_min_after(top_up)")
    for r in rows:
        flag = f"BELOW_MIN(top_up {c.amount(r['top_up'])})" if r["below_min_after"] else "-"
        print(f"{r['stash']} {r['role']} {c.amount(r['bonded'])} {c.amount(r['active'])} {c.amount(r['free'])} "
              f"{c.amount(r['expected_trim'])} {c.amount(r['active_after'])} {flag}")
    trims = [r["expected_trim"] for r in rows]
    summary = {"block": block_n, "target": "all stashes on the staking lock" if all_locked else
               "stashes on the staking lock that cannot hold their full bond", **stats,
               "targets": len(rows), "targets_validators": sum(r["role"] == "validator" for r in rows),
               "targets_nominators": sum(r["role"] == "nominator" for r in rows),
               "targets_idle": sum(r["role"] == "idle" for r in rows),
               "lock_differs_from_ledger": sum(not r["lock_matches_ledger"] for r in rows),
               "expected_trim_total": sum(trims), "expected_trim_max": max(trims or [0]),
               "validators_below_min_after": sum(r["below_min_after"] for r in rows),
               "top_up_total": sum(r["top_up"] for r in rows)}
    print("# summary " + json.dumps(summary))
    print(f"# {len(rows)} targets ({summary['targets_validators']} validators), expected trim "
          f"{c.amount(summary['expected_trim_total'])} {c.symbol} (max {c.amount(summary['expected_trim_max'])}), "
          f"{summary['validators_below_min_after']} validators end below MinValidatorBond "
          f"(top-up {c.amount(summary['top_up_total'])} {c.symbol} in all)")
    return summary


def payee_plan(c, block_hash, validators):
    """Stashes whose Staking.Payee is still the deprecated Controller. RewardDestination::Controller pays
    Staking.Bonded(stash), the controller, so Account(controller) keeps the same destination."""
    payees = {k.value: v.value for k, v in c.s.query_map("Staking", "Payee", page_size=1000, block_hash=block_hash)}
    stashes = sorted(st for st, v in payees.items() if v == "Controller")
    raw = c.multi([((st, k), "Staking", item, [st]) for st in stashes
                   for k, item in (("b", "Bonded"), ("n", "Nominators"))], block_hash)
    ledgers = c.multi([(st, "Staking", "Ledger", [raw[(st, "b")]]) for st in stashes if raw.get((st, "b"))],
                      block_hash)
    rows = []
    for st in stashes:
        ctrl, led = raw.get((st, "b")), ledgers.get(st)
        rows.append({"stash": st, "role": "validator" if st in validators else "nominator" if raw.get((st, "n"))
                     else "idle", "controller": ctrl, "bonded": bool(ctrl and led and led["stash"] == st),
                     "payee_after": {"Account": ctrl} if ctrl else None})
    return rows


def print_payee_plan(rows, enabled):
    print(f"# payee pass{'' if enabled else ' (not sent without --payee)'}: stashes whose Staking.Payee is the "
          f"deprecated Controller; update_payee sets Account(controller), the account their rewards go to now")
    print("# stash role controller payee_after")
    for r in rows:
        dest = f"Account({r['controller']})" if r["bonded"] else "NOT_BONDED(update_payee cannot fix it)"
        print(f"{r['stash']} {r['role']} {r['controller']} {dest}")
    targets = [r for r in rows if r["bonded"]]
    summary = {"controller_payees": len(rows), "targets": len(targets), "not_bonded": len(rows) - len(targets),
               "validators": sum(r["role"] == "validator" for r in targets),
               "nominators": sum(r["role"] == "nominator" for r in targets),
               "idle": sum(r["role"] == "idle" for r in targets),
               "controller_differs_from_stash": sum(r["controller"] != r["stash"] for r in targets)}
    print("# payee summary " + json.dumps(summary))
    return summary


class Sweep:
    def __init__(self, c, kp, out_path, validators, min_bond):
        self.c, self.kp, self.validators, self.min_bond = c, kp, validators, min_bond
        self.out = open(out_path, "a")
        self.rows = []           # every row written, in order
        self.open = {}           # tx hash -> tx, submitted and not yet found in a finalized block
        self.fin_scanned = None  # last finalized block already searched for our calls
        self.kind = "currency"   # the pass that run() works on: "currency" or "payee"
        self.controllers = {}    # payee pass: stash -> controller

    def write(self, row):
        row = {"t": utc(), "pass": self.kind, **row}
        self.rows.append(row)
        self.out.write(json.dumps(row, default=str) + "\n")
        self.out.flush()

    def send(self, stashes, attempt):
        """Sign one call per stash with consecutive explicit nonces and submit them.
        Returns (submitted txs, (stash, error) for the first rejected submission or None)."""
        birth, _ = self.c.finalized()
        nonce = int(self.c.rpc("system_accountNextIndex", [self.kp.ss58_address]))
        txs = []
        for st in stashes:
            if self.kind == "payee":
                call = self.c.s.compose_call("Staking", "update_payee", {"controller": self.controllers[st]})
            else:
                call = self.c.s.compose_call("Staking", "migrate_currency", {"stash": st})
            xt = self.c.s.create_signed_extrinsic(call=call, keypair=self.kp, nonce=nonce,
                                                  era={"period": MORTAL_PERIOD, "current": birth})
            h = xt_hash(bytes(xt.data.data))
            try:
                if self.c.rpc("author_submitExtrinsic", [str(xt.data)]) != h:
                    sys.exit("the node reports a different extrinsic hash than computed here; stopping")
            except SubstrateRequestException as e:
                err = e.args[0] if e.args else str(e)
                if not (isinstance(err, dict) and err.get("code") == 1013):  # 1013: already in the pool
                    return txs, (st, err)
            txs.append({"stash": st, "hash": h, "nonce": nonce, "birth": birth, "attempt": attempt,
                        "kind": self.kind})
            self.open[h] = txs[-1]
            nonce += 1
        return txs, None

    def wait_inclusion(self, txs, from_n):
        """Follow the best chain until every call is in a block, or the pool no longer has the missing
        ones, or they expired. Returns the highest block number seen with one of them."""
        pending = {t["hash"] for t in txs}
        expire = max(t["birth"] for t in txs) + MORTAL_PERIOD
        n, top, idle = from_n, from_n, 0
        while pending:
            best = self.c.best()
            while n <= best and pending:
                hit = pending.intersection(self.c.block(self.c.hash_at(n))["xts"])
                if hit:
                    pending -= hit
                    top, idle = n, 0
                else:
                    idle += 1
                n += 1
            if not pending or n > expire:
                break
            if idle >= STALL_BLOCKS:
                if not pending & self.c.pool():
                    break  # dropped from the pool
                idle = 0
            time.sleep(1.0)
        return top

    def wait_finality(self, n):
        t0 = time.time()
        while True:
            fin, _ = self.c.finalized()
            if fin >= n:
                return fin
            if time.time() - t0 > FINALITY_TIMEOUT:
                sys.exit(f"finality stalled: finalized #{fin}, waiting for #{n}. Stop and check the network; "
                         f"re-running the script later is safe")
            time.sleep(2.0)

    def scan_finalized(self):
        """Search every newly finalized block for our open calls and record what each one did."""
        fin, _ = self.c.finalized()
        found = {}
        for m in range(self.fin_scanned + 1, fin + 1):
            h = self.c.hash_at(m)
            for i, xh in enumerate(self.c.block(h)["xts"]):
                if xh in self.open:
                    found.setdefault((m, h), []).append((self.open.pop(xh), i))
        self.fin_scanned = max(self.fin_scanned, fin)
        for (m, h), items in sorted(found.items()):
            self.record(m, h, items)

    def record(self, m, h, items):
        payee = [(t, i) for t, i in items if t["kind"] == "payee"]
        if payee:
            self.record_payee(m, h, payee)
        items = [(t, i) for t, i in items if t["kind"] == "currency"]
        if not items:
            return
        stashes = [t["stash"] for t, _ in items]
        before = self.c.snapshot(stashes, self.c.block(h)["parent"])
        after = self.c.snapshot(stashes, h)
        for t, idx in items:
            evs = self.c.xt_events(h, idx)
            name = lambda e: f"{e['module_id']}.{e['event_id']}"
            ok = any(name(e) == "System.ExtrinsicSuccess" for e in evs)
            err = next((self.c.module_error(e["attributes"]["dispatch_error"]) for e in evs
                        if name(e) == "System.ExtrinsicFailed"), None)
            fee = sum(int(e["attributes"]["actual_fee"]) for e in evs
                      if name(e) == "TransactionPayment.TransactionFeePaid")
            mig = [e["attributes"] for e in evs if name(e) == "Staking.CurrencyMigrated"]
            b, a = before[t["stash"]], after[t["stash"]]
            is_val = t["stash"] in self.validators
            self.write({"stash": t["stash"], "pass": "currency", "status": "ok" if ok else "error", "error": err,
                        "block": m, "block_hash": h, "extrinsic_index": idx, "extrinsic_hash": t["hash"],
                        "nonce": t["nonce"], "attempt": t["attempt"], "fee": fee,
                        "force_withdraw": int(mig[0]["force_withdraw"]) if mig else None,
                        "currency_migrated_event": bool(mig), "validator": is_val,
                        "expected_trim": expect(b, self.c.ed, is_val, self.min_bond)["expected_trim"]
                        if b["ledger"] else None,
                        "free_plus_reserved_unchanged": b["free"] + b["reserved"] == a["free"] + a["reserved"],
                        "before": b, "after": a})

    def record_payee(self, m, h, items):
        """update_payee: the payee before (parent block) and after (inclusion block). The destination is
        unchanged when Controller, which pays Staking.Bonded(stash), became Account of that controller."""
        stashes = [t["stash"] for t, _ in items]
        read = lambda bh: self.c.multi([((st, k), "Staking", item, [st]) for st in stashes
                                        for k, item in (("p", "Payee"), ("b", "Bonded"))], bh)
        before, after = read(self.c.block(h)["parent"]), read(h)
        for t, idx in items:
            st = t["stash"]
            evs = self.c.xt_events(h, idx)
            name = lambda e: f"{e['module_id']}.{e['event_id']}"
            ok = any(name(e) == "System.ExtrinsicSuccess" for e in evs)
            err = next((self.c.module_error(e["attributes"]["dispatch_error"]) for e in evs
                        if name(e) == "System.ExtrinsicFailed"), None)
            fee = sum(int(e["attributes"]["actual_fee"]) for e in evs
                      if name(e) == "TransactionPayment.TransactionFeePaid")
            ctrl, p0, p1 = before.get((st, "b")), before.get((st, "p")), after.get((st, "p"))
            self.write({"stash": st, "pass": "payee", "status": "ok" if ok else "error", "error": err,
                        "block": m, "block_hash": h, "extrinsic_index": idx, "extrinsic_hash": t["hash"],
                        "nonce": t["nonce"], "attempt": t["attempt"], "fee": fee,
                        "validator": st in self.validators, "controller": ctrl,
                        "payee_before": p0, "payee_after": p1,
                        "destination_unchanged": p0 == "Controller" and p1 == {"Account": ctrl}})

    def settle(self, txs, from_n):
        """Inclusion, then finality, then results. Returns the calls not found in a finalized block."""
        top = self.wait_inclusion(txs, from_n)
        self.wait_finality(top)
        self.scan_finalized()
        return [t for t in txs if t["hash"] in self.open]

    def still_needed(self, stashes):
        """The stashes that still need this pass's call at the best block: still on the staking lock,
        or (payee pass) payee still Controller."""
        bh = self.c.hash_at(self.c.best())
        if self.kind == "payee":
            cur = self.c.multi([(st, "Staking", "Payee", [st]) for st in stashes], bh)
            return [st for st in stashes if cur.get(st) == "Controller"]
        cur = self.c.snapshot(stashes, bh)
        return [st for st in stashes if cur[st]["staking_lock"] > 0]

    def skip(self, st, why):
        self.write({"stash": st, "status": "skipped", "error": why, "fee": 0, "force_withdraw": None,
                    "validator": st in self.validators})

    def resubmit(self, t):
        """Send a dropped call again, alone, after an on-chain check that the stash still needs it (still
        on the lock, or payee still Controller). The new call takes the next free nonce, which is the
        dropped call's nonce, so at most one lands."""
        st, attempt, mine = t["stash"], t["attempt"], [t["hash"]]
        try:
            while True:
                self.wait_finality(self.c.best())
                self.scan_finalized()  # a dropped call may still have landed late
                if any(r.get("extrinsic_hash") in mine for r in self.rows):
                    return
                if not self.still_needed([st]):
                    self.skip(st, SKIP[self.kind][1])
                    return
                if attempt >= MAX_ATTEMPTS:
                    self.write({"stash": st, "status": "error", "error": f"dropped {attempt} times", "fee": 0,
                                "force_withdraw": None, "validator": st in self.validators})
                    return
                attempt += 1
                log(f"resubmitting the call for {st} alone (attempt {attempt})")
                from_n = self.c.best()
                txs, rejected = self.send([st], attempt)
                if rejected:
                    log(f"the pool rejected it: {rejected[1]}")
                    time.sleep(12)
                    continue
                mine.append(txs[0]["hash"])
                self.settle(txs, from_n)
        finally:
            for h in mine:
                self.open.pop(h, None)

    def run(self, targets, wave_size):
        self.fin_scanned = self.c.best() - 1
        queue = list(targets)
        waves = (len(queue) + wave_size - 1) // wave_size
        w = 0
        while queue:
            wave, queue = queue[:wave_size], queue[wave_size:]
            w += 1
            live = self.still_needed(wave)
            for st in wave:
                if st not in live:
                    self.skip(st, SKIP[self.kind][0])
            if not live:
                continue
            from_n = self.c.best()
            t0 = time.time()
            txs, rejected = self.send(live, 1)
            if rejected:
                st, err = rejected
                log(f"wave {w}: the pool rejected the call for {st}: {err}")
                if not txs:
                    sys.exit("the pool rejects the first call of a wave; stopping (re-running later is safe)")
                queue = live[len(txs):] + queue
            n_before = len(self.rows)
            dropped = self.settle(txs, from_n)
            for t in sorted(dropped, key=lambda x: x["nonce"]):
                self.resubmit(t)
            new = [r for r in self.rows[n_before:] if r["stash"] in {t["stash"] for t in txs}]
            ok = sum(r["status"] == "ok" for r in new)
            trimmed = sum(r.get("force_withdraw") or 0 for r in new if r["status"] == "ok")
            blocks = sorted({r["block"] for r in new if r.get("block")})
            detail = (f"destination unchanged {sum(1 for r in new if r.get('destination_unchanged'))}"
                      if self.kind == "payee" else f"trimmed {self.c.amount(trimmed)}")
            log(f"{self.kind} wave {w}/{waves}: {len(txs)} sent, {ok} ok, {len(new) - ok} not ok, blocks {blocks}, "
                f"{detail}, {time.time() - t0:.0f}s")
            if new and ok == 0 and all(r["status"] == "error" for r in new):
                sys.exit(f"every call of wave {w} failed ({Counter(r['error'] for r in new)}); stopping")


def final_rows(rows):
    """One row per stash: the first success, else the latest row."""
    out = {}
    for r in rows:
        if out.get(r["stash"], {}).get("status") != "ok":
            out[r["stash"]] = r
    return out


def main():
    ap = argparse.ArgumentParser(description="Staking.migrate_currency sweep (staking lock -> hold) after runtime 392.")
    ap.add_argument("--rpc", required=True, help="websocket URL of a synced node, e.g. ws://127.0.0.1:9944")
    ap.add_argument("--signer-file", help="file holding the signer's mnemonic or secret URI (sr25519); "
                                          "needed unless --dry-run")
    ap.add_argument("--dry-run", action="store_true", help="list the target stashes and expected trims; send nothing")
    ap.add_argument("--all-locked", action="store_true",
                    help="target every stash still on the staking lock, not only those that cannot hold their bond")
    ap.add_argument("--payee", action="store_true",
                    help="after the currency pass, give every stash whose payee is the deprecated Controller "
                         "Account(controller) with Staking.update_payee (same destination, no defensive error)")
    ap.add_argument("--wave-size", type=int, default=50, help="calls per wave, about one block's worth (default 50)")
    ap.add_argument("--out", help="JSONL output, one line per stash and pass "
                                  "(default migrate_currency-<UTC time>.jsonl)")
    args = ap.parse_args()
    if not args.dry_run and not args.signer_file:
        ap.error("--signer-file is required unless --dry-run")

    c = Chain(args.rpc)
    rv = c.rpc("state_getRuntimeVersion", [])
    log(f"chain {c.rpc('system_chain', [])}, genesis {c.hash_at(0)}, {rv['specName']} spec {rv['specVersion']}")
    kp = load_signer(args.signer_file, c.ss58) if args.signer_file else None
    if kp:
        log(f"signer {kp.ss58_address}")

    fin_n, fin_h = c.finalized()
    rows, stats, validators, min_bond = plan(c, fin_h, args.all_locked)
    payees = payee_plan(c, fin_h, validators)
    if args.dry_run:
        summary = print_plan(c, rows, stats, min_bond, fin_n, args.all_locked)
        psummary = print_payee_plan(payees, args.payee)
        if args.out:
            with open(args.out, "a") as f:
                for r in rows:
                    f.write(json.dumps({"t": utc(), "pass": "currency", "status": "planned", **r}) + "\n")
                for r in payees:
                    f.write(json.dumps({"t": utc(), "pass": "payee", "status": "planned", **r}) + "\n")
                f.write(json.dumps({"t": utc(), "dry_run_summary": summary, "payee_summary": psummary}) + "\n")
        return 0

    out = args.out or time.strftime("migrate_currency-%Y%m%dT%H%M%SZ.jsonl", time.gmtime())
    trims = sum(r["expected_trim"] for r in rows)
    ptargets = [r for r in payees if r["bonded"]] if args.payee else []
    log(f"{len(rows)} targets ({sum(r['role'] == 'validator' for r in rows)} validators), expected trim "
        f"{c.amount(trims)} {c.symbol}, waves of {args.wave_size}, results in {out}")
    if args.payee:
        log(f"payee pass: {len(ptargets)} stashes with the deprecated Controller payee")
    if not rows and not ptargets:
        return 0
    ti_before = int(c.query("Balances", "TotalIssuance", [], fin_h))
    signer_before = int(c.query("System", "Account", [kp.ss58_address], fin_h)["data"]["free"])
    if signer_before < c.ed + 10 ** c.decimals:
        sys.exit(f"signer free balance {c.amount(signer_before)} is too low to pay the fee of a failed call")
    t0 = time.time()
    sweep = Sweep(c, kp, out, validators, min_bond)
    if rows:
        sweep.run([r["stash"] for r in rows], args.wave_size)
    elapsed = time.time() - t0
    if ptargets:
        t1 = time.time()
        sweep.kind = "payee"
        sweep.controllers = {r["stash"]: r["controller"] for r in ptargets}
        sweep.run([r["stash"] for r in ptargets], args.wave_size)
        p_elapsed = time.time() - t1

    fin_n2, fin_h2 = c.finalized()
    ti_after = int(c.query("Balances", "TotalIssuance", [], fin_h2))
    signer_after = int(c.query("System", "Account", [kp.ss58_address], fin_h2)["data"]["free"])
    crows = [r for r in sweep.rows if r["pass"] == "currency"]
    final = final_rows(crows)
    status = Counter(r["status"] for r in final.values())
    errors = Counter(r["error"] for r in final.values() if r["status"] == "error")
    fees = sum(r.get("fee") or 0 for r in crows)
    trimmed = sum(r.get("force_withdraw") or 0 for r in final.values() if r["status"] == "ok")
    trimmed_by = {s: r.get("force_withdraw") or 0 for s, r in final.items() if r["status"] == "ok"}
    vals = sorted(c.keys("Staking", "Validators", fin_h2))
    snap = c.snapshot(vals, fin_h2)
    below = [{"stash": v, "active": snap[v]["ledger"]["active"], "top_up": min_bond - snap[v]["ledger"]["active"],
              "trimmed_by_this_run": trimmed_by.get(v, 0)}
             for v in vals if snap[v]["ledger"] and snap[v]["ledger"]["active"] < min_bond]
    summary = {"finished": utc(), "elapsed_s": round(elapsed),
               "finalized_before": fin_n, "finalized_after": fin_n2, "targets": len(rows),
               "calls_in_blocks": sum(1 for r in crows if r.get("extrinsic_hash")),
               "resubmitted": sum(1 for r in crows if (r.get("attempt") or 1) > 1),
               "stashes": dict(status), "errors": dict(errors),
               "fees_total": fees, "signer_free_before": signer_before, "signer_free_after": signer_after,
               "trimmed_total": trimmed, "expected_trim_total": trims,
               "stashes_trimmed": sum(1 for v in trimmed_by.values() if v > 0),
               "free_plus_reserved_changed": sum(1 for r in final.values()
                                                 if r["status"] == "ok" and not r["free_plus_reserved_unchanged"]),
               "total_issuance_before": ti_before, "total_issuance_after": ti_after,
               "total_issuance_unchanged": ti_before == ti_after,
               "min_validator_bond": min_bond, "validators_below_min": below}
    payee_ok = True
    if args.payee:
        prows = [r for r in sweep.rows if r["pass"] == "payee"]
        pfinal = final_rows(prows)
        left = sorted(k.value for k, v in c.s.query_map("Staking", "Payee", page_size=1000, block_hash=fin_h2)
                      if v.value == "Controller")
        summary["payee"] = {
            "elapsed_s": round(p_elapsed) if ptargets else 0, "controller_payees": len(payees),
            "targets": len(ptargets), "not_bonded": len(payees) - sum(r["bonded"] for r in payees),
            "calls_in_blocks": sum(1 for r in prows if r.get("extrinsic_hash")),
            "resubmitted": sum(1 for r in prows if (r.get("attempt") or 1) > 1),
            "stashes": dict(Counter(r["status"] for r in pfinal.values())),
            "errors": dict(Counter(r["error"] for r in pfinal.values() if r["status"] == "error")),
            "fees_total": sum(r.get("fee") or 0 for r in prows),
            "destination_unchanged": sum(1 for r in pfinal.values() if r["status"] == "ok" and r["destination_unchanged"]),
            "ok": sum(1 for r in pfinal.values() if r["status"] == "ok"),
            "controller_payees_left": left}
        p = summary["payee"]
        payee_ok = not p["errors"] and p["destination_unchanged"] == p["ok"] and not left
    with open(out + ".summary.json", "w") as f:
        json.dump(summary, f, indent=1)

    print()
    print(f"stashes: {dict(status)}  calls in blocks: {summary['calls_in_blocks']} "
          f"(resubmitted {summary['resubmitted']})  time: {elapsed / 60:.1f} min")
    print(f"errors: {dict(errors) or 'none'}")
    print(f"fees charged: {c.amount(fees)} {c.symbol} (signer free {c.amount(signer_before)} -> {c.amount(signer_after)})")
    print(f"trimmed: {c.amount(trimmed)} {c.symbol} on {summary['stashes_trimmed']} stashes "
          f"(dry-run expected {c.amount(trims)}); free+reserved changed on {summary['free_plus_reserved_changed']} stashes")
    print(f"total issuance: {c.amount(ti_before)} -> {c.amount(ti_after)} "
          f"{'UNCHANGED' if ti_before == ti_after else 'CHANGED by ' + c.amount(ti_after - ti_before)}")
    print(f"validators below MinValidatorBond ({c.amount(min_bond)} {c.symbol}): {len(below)}")
    for b in sorted(below, key=lambda x: -x["top_up"]):
        print(f"  {b['stash']} active {c.amount(b['active'])} top up {c.amount(b['top_up'])}"
              f"{' (trimmed ' + c.amount(b['trimmed_by_this_run']) + ' now)' if b['trimmed_by_this_run'] else ''}")
    if args.payee:
        p = summary["payee"]
        print(f"payee pass: stashes {p['stashes']}  calls in blocks: {p['calls_in_blocks']} "
              f"(resubmitted {p['resubmitted']})  time: {p['elapsed_s'] / 60:.1f} min")
        print(f"payee errors: {p['errors'] or 'none'}")
        print(f"payee fees charged: {c.amount(p['fees_total'])} {c.symbol}")
        print(f"payee destination unchanged (Controller -> Account(same controller)): "
              f"{p['destination_unchanged']} of {p['ok']} ok; Controller payees left: {len(p['controller_payees_left'])}")
        for st in p["controller_payees_left"]:
            print(f"  {st}")
    print(f"results: {out}  summary: {out}.summary.json")
    return 0 if not errors and ti_before == ti_after and payee_ok else 1


if __name__ == "__main__":
    sys.exit(main())
