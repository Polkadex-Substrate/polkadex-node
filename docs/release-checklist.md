# Release checklist

Every release of `polkadex-node` follows this list, top to bottom. Nothing is skipped because it is small. If a step cannot be done, the release waits. How each step is done is in `release-procedures.md`; this file is only the boxes.

Roles: **Dev** writes the code and opens PRs. **Reviewers** are two people who are not the author. **Release manager** signs and publishes. **Ops** runs the team's own nodes. One person may hold more than one role, but the author of a change never reviews or signs it.

## 1. Before the tag

- [ ] Every change is on `testnet` through a PR with two approvals. No direct pushes.
- [ ] `testnet` has run with these changes on the testnet chain for at least 3 days with no incident (procedure 1). Ops calls it in writing.
- [ ] The same changes are merged to `mainnet` through a PR with two approvals.
- [ ] CI has run on the exact commit being tagged and every job is green.
- [ ] The pin check job passed.
- [ ] If the runtime changed: `spec_version` bumped, and `transaction_version` bumped if any call or extension changed shape.
- [ ] If the runtime changed: unfiltered try-runtime against a live mainnet snapshot, log attached to the PR, every migration listed with what it touches and whether it is guarded.
- [ ] Release notes draft written: what changed, what operators must do, whether validators must rotate keys.
- [ ] Release manager and Ops have agreed a date and a window.

## 2. Tag

- [ ] Tag is on the `mainnet` branch, named `vX.Y.Z`, annotated. The workflow's branch gate passed (procedure 2).
- [ ] Tag commit hash recorded in the release notes.

## 3. Build and verify

- [ ] The release workflow produced binary, `.deb`, `.rpm`, `customSpecRaw.json` and `SHA256SUMS` as a draft release.
- [ ] Release manager verified the checksums locally.
- [ ] Release manager confirmed the build used the pinned toolchain and the pin check passed.
- [ ] Dev ran the install test on clean Ubuntu 22.04 and Rocky 9 (procedure 3). Logs attached to the draft.

## 4. Sign and publish

- [ ] Release manager signed `SHA256SUMS` and uploaded the signature.
- [ ] Release notes render correctly and include the verify instructions.
- [ ] Release manager published the release.

## 5. The team's own nodes first

- [ ] Ops upgraded the public RPC node(s). Back at head and serving.
- [ ] Ops upgraded the bootnode(s).
- [ ] Telemetry shows the new version on the team's nodes.
- [ ] Stop rule: a team node not back at head within 30 minutes halts the release. Release manager decides. Validators are not contacted until this section is complete.

## 6. Validators

- [ ] Announcement sent: release link, verify steps, client upgrade window, migration doc link.
- [ ] If a runtime upgrade follows: enactment date and the key rotation instruction included.
- [ ] A named person watches telemetry during the window and contacts validators that drop.
- [ ] Stop rule: validators holding over 10 percent of active stake off for more than an hour pauses the announcement and the upgrade date.
- [ ] External communications sent to users and exchanges, with a prepared statement for failure. Ops owns the wording.

## 7. Runtime upgrade (only when the release carries one)

- [ ] Client compatibility confirmed by the three checks in procedure 4, results recorded. Old-client validators keep producing and finalising after enactment.
- [ ] Validators holding at least two thirds of active stake are known to be online and following the chain the day before enactment.
- [ ] BEEFY is unstarted: `beefy.genesisBlock()` reads `None` on testnet after the soak and on mainnet after enactment (procedure 5).
- [ ] The chill call for validators on placeholder keys has an owner and an issue before any BEEFY start date is announced (procedure 5).
- [ ] The `set_code` proposal goes through governance. The runtime test that no pallet named `Sudo` exists passed on the tagged commit.
- [ ] At enactment: Ops watches block production and finality for two sessions. A stall escalates immediately.
- [ ] After enactment: validators rotate keys if required. Ops confirms the new session key set on the team's validators.

## 8. After

- [ ] Release notes updated with anything learned.
- [ ] Follow-up items filed as issues with an owner.
- [ ] This checklist and the procedures updated if a step was missing or wrong.

## Rollback

A client release that misbehaves: validators reinstall the previous package. Packages for previous releases stay attached to their GitHub release, never deleted.

A runtime upgrade that misbehaves cannot be rolled back by validators. It needs a new runtime upgrade through governance. This is why section 1 is not optional.
