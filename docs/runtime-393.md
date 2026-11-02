# Runtime 393: staked PDEX votes again, and stuck stakers can pay fees again

Plan as of 30 Oct 2026.

## In short

- Since runtime 392 went live on 29 Oct, staked PDEX cannot be used to vote once it has moved into the new staking compartment (explained below). @LegoMatrix spotted this and reported it in the main Telegram group before 392 went live. Thank you, Lego. We let 392 go ahead and promised a small, separate fix.
- While building that fix, we found a second problem caused by the same change. About 300 staking accounts cannot send any transaction, because the fee check refuses them. Their PDEX is safe: nothing is lost and nothing has left their accounts.
- Runtime 393 fixes both problems and changes nothing else. A free cleanup call fixes most of the stuck accounts even before 393.
- No other vote runs before 393 is live.

## What changed in 392

Think of your account as a wallet with a main pocket. The main pocket is your free balance.

- Before 392, staked PDEX stayed in the main pocket with a "do not spend" sticker on it.
- From 392, staked PDEX moves into a separate locked compartment. More than half of all stashes have moved since 29 Oct, and every other stash moves the first time its stake changes. Your total does not change. Wallets show staked PDEX in the compartment as "reserved" instead of "locked".

The compartment is the new standard way to keep staked funds. The trouble is that two parts of the chain still look only at the main pocket.

## Problem 1: staked PDEX cannot vote

The voting code checks your vote against the main pocket only. It does not see the compartment. Once your stake is in the compartment:

- In referendums, you can vote or delegate only your unstaked PDEX.
- In the council election, your vote counts only your unstaked PDEX.

Votes cast before 392 keep their full weight. Stashes that still have only the old sticker can still vote with their stake for now, because it is still in the main pocket.

## Problem 2: some stakers cannot pay fees

The fee check also looks only at the main pocket, and it refuses the fee whenever a sticker or a vote lock is bigger than what is in that pocket. It catches two groups.

1. **An old sticker left behind.** This is most of the stuck accounts. The first time a stake changes after 392, usually when a reward is restaked, the stake moves into the compartment but the old sticker stays on. The fee check then sees a lock bigger than the main pocket and refuses every fee.
2. **A vote lock from before 392.** This is about 30 accounts. They voted before 392 with PDEX that was then in the main pocket. That PDEX is now in the compartment, so their vote lock is bigger than the main pocket.

This also means the vote fix cannot ship alone. If 393 only let staked PDEX vote, everyone who then voted with it would end up in the second group, unable to send any transaction, not even one to remove the vote. So the fee fix ships together with the vote fix.

## What 393 changes

1. **Votes count free plus staked PDEX.** That is exactly what they counted before 392. Deposits still do not count, as before; deposits are PDEX set aside for things like an identity, a proposal or the council voting bond. Polkadot's newer governance counts deposits too. That would be a change of policy rather than a fix, so if the community wants it, it can go to its own referendum.
2. **Fees are taken from the PDEX that is really free.** A vote lock may then sit over staked PDEX without blocking fees. This is the way the Polkadot SDK recommends. Fees cost the same as before, nobody's fee check gets stricter, and fees still go entirely to the treasury.

Nothing else changes.

## What we do besides 393

- **Now:** a free cleanup call that removes the old sticker and keeps the whole stake bonded. It fixes almost all of the first group right away. Anyone can send it for any stash, and it costs nothing when it succeeds.
- **Right after 393:** the same cleanup for about 1,500 stashes that still have only the old sticker, so they never get stuck. We wait for 393 because some of them also carry a vote lock, and today's fee check would catch them.
- **About 25 of the stuck accounts have only the 1 PDEX minimum free.** They need someone to send them a little PDEX, even after 393. Receiving PDEX costs nothing.

## What you need to do

- **No node upgrade is expected for 393.** The fix adds no new node functions, and we will confirm this before the vote. Node v7.0.1, a separate release planned for the same week, will be recommended because it limits disk use.
- **If your account cannot pay fees now,** the cleanup call or 393 will fix it. If you have only 1 PDEX free, ask someone to send you a little PDEX.
- **After 393,** you can vote and delegate with your staked PDEX. Keep a little PDEX outside any vote to pay fees: if every PDEX you have is locked by a vote, nothing is left for a fee. That was already true before 392.

## The steps

1. Build and test the fix.
2. Code review by the team and an outside reviewer.
3. The council proposes 393, and the technical committee fast-tracks it.
4. A 2-day referendum. It still runs under today's rule, so staked PDEX that is already in the compartment cannot vote in it yet.
5. 393 goes live about an hour after the vote ends.

We will post the exact times in the main group before the vote opens.
