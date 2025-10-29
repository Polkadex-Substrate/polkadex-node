# POLKADEX DEMOCRACY UPGRADE + SESSION KEY ROTATION
## Step-by-Step Guide Using Polkadot.js Apps UI
###  EVENTS & API METHODS

---

## PREPARATION

### Step 0: Access Polkadot.js Apps
1. Open **Polkadot.js Apps**: https://polkadot.js.org/apps/
2. Connect to your local node: `ws://127.0.0.1:9944`
3. Verify connection shows **"polkadex-official"** and **spec version 378**

### Step 0.1: Prepare Runtime File
1. Build your runtime:
   ```bash
   cargo build --release --features runtime-benchmarks
   ```
2. Get the runtime WASM file from:
   ```
   ./target/release/wasm32-unknown-unknown/release/node_polkadex_runtime.compact.compressed.wasm
   ```

---

## PHASE 1: TECHNICAL COMMITTEE PROPOSAL

### Step 1.1: Create Technical Committee Proposal

1. **Navigate**: `Developer` → `Extrinsics`
2. **Select**:
   - **using the selected account**: Choose a **Technical Committee member account**
   - **submit the following extrinsic**: `technicalCommittee`
   - **method**: `propose(threshold, proposal, lengthBound)`

3. **Fill Parameters**:
   - **threshold**: `2` (2 out of 3 tech committee members)
   - **proposal**:
     - Select `system` → `setCode(code)`
     - **code**: Upload your runtime WASM file
   - **lengthBound**: `2000000` (auto-filled based on file size)

4. **Submit**: Click `Submit Transaction`
5. **Sign**: Enter password and submit

**Expected Event**: `technicalCommittee.Proposed` - 

---

## PHASE 2: TECHNICAL COMMITTEE VOTING

### Step 2.1: First Technical Committee Vote

1. **Navigate**: `Developer` → `Extrinsics`
2. **Select**:
   - **using the selected account**: First tech committee member
   - **submit the following extrinsic**: `technicalCommittee`
   - **method**: `vote(proposal, index, approve)`

3. **Fill Parameters**:
   - **proposal**: Copy the proposal hash from the `Proposed` event
   - **index**: `0` (first proposal)
   - **approve**: `Yes` (toggle on)

4. **Submit Transaction** and **Sign**

### Step 2.2: Second Technical Committee Vote

1. **Repeat Step 2.1** with:
   - **using the selected account**: Second tech committee member
   - **Same parameters** as Step 2.1

**Expected Events**:
- `technicalCommittee.Voted` - 
- `technicalCommittee.Approved` - 
- `technicalCommittee.Executed` - 
- `system.CodeUpdated` - 

---

## PHASE 3: VERIFY RUNTIME UPGRADE

### Step 3.1: Check Runtime Version

1. **Navigate**: `Developer` → `RPC calls`
2. **Select**: `state` → `getRuntimeVersion()`
3. **Submit**: Click `Submit RPC call`
4. **Verify**: `specVersion` should be updated (e.g., 379 or higher)

**Alternative Check**:
1. **Navigate**: `Network` → `Explorer`
2. **Look for**: Recent `system.CodeUpdated` event

---

## PHASE 4: GENERATE SESSION KEYS

### Step 4.1: Generate Keys for Each Validator

**For Validator 1 (Port 9944)**:
1. **Navigate**: `Developer` → `RPC calls`
2. **Select**: `author` → `rotateKeys()`
3. **endpoint**: Ensure connected to `ws://127.0.0.1:9944`
4. **Submit**: Click `Submit RPC call`
5. **Copy Result**: The long hex string (session keys)

**For Validator 2 (Port 9946)**:
1. **Change endpoint**: Connect to `ws://127.0.0.1:9946`
2. **Repeat**: `author` → `rotateKeys()`
3. **Copy Result**: Session keys for validator 2

**For Validator 3 (Port 9948)**:
1. **Change endpoint**: Connect to `ws://127.0.0.1:9948`
2. **Repeat**: `author` → `rotateKeys()`
3. **Copy Result**: Session keys for validator 3

**Example Output**:
```
0xd0c37884d9d555331551efd1c8f29121971107d6e3ce100773e660a32b30750f2e1df2cad9d0110e29a2b2a6b511e0528bb1f71c5f8af92201063c47c72f372998eec3a0f623333c95a4ee3eaa9b11ca81f9b3792e37b91affa6448763a33b7ba4c8f6d8461351c8329a4881465c4d558dc532f0d410e84124debeba1d536b1ce4ed8575e8a32b55cf41fb96a5ad5a827e7890c3a004dbad5eb6753edd5d2b564e112f8470825ff3968d1dae06f833f66fae55186c6910729feda83407a12537029b43dc6a95ea7f0d437344a10a34b754d4be37bfc21790d245f29a905b762a47
```

---

## PHASE 5: SUBMIT SESSION KEYS

### Step 5.1-5.3: Submit Keys for All Validators

1. **Navigate**: `Developer` → `Extrinsics`
2. **Connect**: Back to main node `ws://127.0.0.1:9944`
3. **Select**:
   - **using the selected account**: **Alice/Bob/Charlie** (respective stash accounts)
   - **submit the following extrinsic**: `session`
   - **method**: `setKeys(keys, proof)`

4. **Fill Parameters**:
   - **keys**: Paste the session keys from respective validator
   - **proof**: `0x00` (empty)

**Expected Events**: `session.NewSession` - 

---

## PHASE 6: MONITOR SESSION TRANSITION

### Step 6.1: Check Current Session

1. **Navigate**: `Developer` → `Chain state`
2. **Select**: `session` → `currentIndex()`
3. **Submit**: Note the current session number

### Step 6.2: Check Queued Keys

1. **Navigate**: `Developer` → `Chain state`
2. **Select**: `session` → `queuedKeys()`
3. **Submit**: Verify your new keys are queued

**Expected**: Should show 3 entries with your new session keys

### Step 6.3: Calculate Next Session Time

**Current session duration**: 4 hours (1200 blocks)

1. **Navigate**: `Network` → `Explorer`
2. **Check current block**: Note the block number
3. **Calculate**:
   - Session starts every 1200 blocks
   - If current block is 2400, next session at block 3600
   - Time remaining = (3600 - 2400) × 12 seconds ÷ 3600 = ~4 hours

---

## PHASE 7: VERIFY SESSION KEY ACTIVATION

### Step 7.1: Wait for Session Change

**Monitor**: `Network` → `Explorer` for `session.NewSession` event

### Step 7.2: Verify Active Keys

**After session change**:

1. **Navigate**: `Developer` → `Chain state`
2. **Select**: `session` → `validators()`
3. **Submit**: Should show your validator accounts

4. **Check individual keys**:
   - **Select**: `session` → `nextKeys(AccountId)`
   - **AccountId**: Enter Alice's account
   - **Submit**: Should show the new session keys

### Step 7.3: Verify BEEFY Activation

1. **Navigate**: `Developer` → `RPC calls`
2. **Select**: `state` → `call(method, data)`
3. **method**: `BeefyApi_validator_set` -  API
4. **data**: `0x`
5. **Submit**: Should show BEEFY authorities with your new keys

**Check BEEFY Genesis**:
1. **method**: `BeefyApi_beefy_genesis` -  API
2. **data**: `0x`
3. **Expected**: Should show a block number (not null)

---

## PHASE 8: NODE BINARY UPDATE

### Step 8.1: Prepare New Binary

After runtime upgrade, validators should update their node binaries to match the new runtime version.

**Build New Binary**:
```bash
cargo build --release --features runtime-benchmarks

ls -la ./target/release/polkadex-node
```

### Step 8.2: Update Validators (One by One)

**IMPORTANT**: Update validators **one at a time** to maintain network stability.

**For Each Validator**:

1. **Stop the validator**:
   ```bash
   # Find the process
   ps aux | grep polkadex-node

   # Stop gracefully
   kill -TERM <process_id>
   ```

2. **Backup old binary** (optional):
   ```bash
   mv ./target/release/polkadex-node ./target/release/polkadex-node.old
   ```

3. **Replace with new binary**:
   ```bash
   # Copy new binary
   cp ./target/release/polkadex-node /path/to/validator/directory/

   # Ensure executable permissions
   chmod +x ./target/release/polkadex-node
   ```

4. **Restart validator**:
   ```bash
   # Use your existing start script
   ./scripts/start_validator1.sh
   # or
   ./scripts/start_validator2.sh
   # or
   ./scripts/start_validator3.sh
   ```

5. **Verify restart**:
   ```bash
   # Check logs
   tail -f /tmp/alice.log

   # Check version via RPC
   curl -H "Content-Type: application/json" \
        -d '{"id":1, "jsonrpc":"2.0", "method": "system_version", "params": []}' \
        http://127.0.0.1:9944
   ```

### Step 8.3: Verify All Validators Updated

**Check Each Validator**:

1. **Validator 1**: `curl -s http://127.0.0.1:9944` → `system_version`
2. **Validator 2**: `curl -s http://127.0.0.1:9946` → `system_version`
3. **Validator 3**: `curl -s http://127.0.0.1:9948` → `system_version`

**Expected**: All should show the same updated version

### Step 8.4: Verify Network Health

1. **Check Block Production**:
   - Navigate: `Network` → `Explorer`
   - Verify: Blocks are being produced continuously
   - Check: All 3 validators are authoring blocks

2. **Check Finalization**:
   - Verify: `finalized` block number is advancing
   - Check: No large gaps between `best` and `finalized`

3. **Check BEEFY** (if applicable):
   - RPC: `BeefyApi_validator_set` shows all 3 validators
   - Logs: No BEEFY errors in validator logs

---

##  EVENT NAMES & API METHODS

### Technical Committee Events:
- `technicalCommittee.Proposed` - 
- `technicalCommittee.Voted` - 
- `technicalCommittee.Approved` - 
- `technicalCommittee.Executed` - 

### System Events:
- `system.CodeUpdated` - 

### Session Events:
- `session.NewSession` - 

### BEEFY API Methods:
- `BeefyApi_beefy_genesis` (returns `Option<BlockNumber>`) - 
- `BeefyApi_validator_set` (returns `Option<ValidatorSet<BeefyId>>`) - 

### Standard RPC Methods:
- `author_rotateKeys()` - 
- `state_getRuntimeVersion()` - 
- `system_version()` - 

---

## MONITORING DASHBOARD

### Key Pages to Monitor:

1. **Network → Explorer**: Watch for events
2. **Network → Staking**: Check validator status
3. **Developer → Chain state**: Query current state
4. **Developer → RPC calls**: Check BEEFY status

### Important Events to Watch:
- `technicalCommittee.Proposed`
- `technicalCommittee.Voted`
- `technicalCommittee.Executed`
- `system.CodeUpdated`
- `session.NewSession`

### Key Queries:
```
session.currentIndex()          // Current session number
session.queuedKeys()           // Keys waiting for next session
session.validators()           // Active validators
session.nextKeys(AccountId)    // Specific validator keys
```

---

## TROUBLESHOOTING

### If Technical Committee Proposal Fails:
- Check that account is actually a tech committee member
- Verify runtime WASM file is correct and under 2MB
- Ensure threshold (2) is correct for your committee size

### If Session Keys Don't Activate:
- Verify keys were submitted before session boundary
- Check that validator accounts are bonded and validating
- Ensure session keys were generated from the correct validator nodes

### If BEEFY Doesn't Activate:
- Check that BEEFY genesis block is set (not null)
- Verify all validators have BEEFY keys in their session keys
- Wait for the session after keys become active

### Node Update Issues:
- **Binary won't start**: Check file permissions (`chmod +x`)
- **Version mismatch**: Verify you built from correct commit/branch
- **Network partition**: Update validators one at a time, wait for sync

### Session Key Issues:
- **Keys not active**: Wait for next session boundary (every 4 hours)
- **BEEFY not working**: Verify genesis block is set and all validators have BEEFY keys

---

## DEMOCRACY PARAMETERS REFERENCE

### Technical Committee Configuration:
- **Instance**: `pallet_collective::Instance2`
- **Members**: Technical experts responsible for fast-track proposals
- **Motion Duration**: 7 days (`TechnicalMotionDuration = 7 * DAYS`)
- **Max Proposals**: 100 concurrent proposals
- **Voting Threshold**: 2/3 majority required for fast-track (`EnsureProportionAtLeast<2, 3>`)

### Council Configuration:
- **Instance**: `pallet_collective::Instance1`
- **Motion Duration**: 7 days (`CouncilMotionDuration = 7 * DAYS`)
- **Max Proposals**: 100 concurrent proposals
- **Voting Threshold**: 1/2 majority for external proposals (`EnsureProportionAtLeast<1, 2>`)

### Democracy Parameters:
```rust
LaunchPeriod: 15 * DAYS          
VotingPeriod: 15 * DAYS         
FastTrackVotingPeriod: 3 * HOURS 
EnactmentPeriod: 30 * DAYS      
VoteLockingPeriod: 30 * DAYS     
CooloffPeriod: 28 * DAYS         
MinimumDeposit: 100 * PDEX     
```

