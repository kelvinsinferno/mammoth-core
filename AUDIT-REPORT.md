# Mammoth Protocol -- Full Security & Code Audit Report

**Date:** 2026-04-11
**Auditor:** Claude Opus 4.6 (automated deep audit)
**Scope:** mammoth (contract), mammoth-sdk, mammoth-mcp, mammoth-android

---

## Executive Summary

| Project | CRITICAL | HIGH | MEDIUM | LOW | INFO | Total |
|---------|----------|------|--------|-----|------|-------|
| Smart Contract (lib.rs) | 4 | 5 | 7 | 5 | 3 | 24 |
| SDK (mammoth-sdk) | 4 | 6 | 8 | 5 | 4 | 27 |
| MCP Server (mammoth-mcp) | 1 | 3 | 4 | 3 | 2 | 13 |
| Android App (mammoth-android) | 3 | 6 | 9 | 7 | 3 | 28 |
| **TOTAL** | **12** | **20** | **28** | **20** | **12** | **92** |

**Most urgent:** The smart contract has a **fund theft vulnerability** in `close_cycle` that allows anyone to steal the creator's share of SOL from auto-closed cycles. Fix this before any mainnet deployment.

---

## SMART CONTRACT (lib.rs) -- 24 Issues

### CRITICAL

#### SC-1: close_cycle sends SOL to any signer, not the actual creator -- FUND THEFT
- **Lines:** 828-837, 1422-1449
- **Description:** The `close_cycle` authorization uses OR logic: `project.creator == ctx.accounts.creator.key() || cycle.status == CycleStatus::Closed`. When a cycle auto-closes (supply cap reached in `buy_tokens`), the second condition is true, so **anyone** can call `close_cycle`. The SOL distribution on lines 864-880 sends `creator_share` to whoever `ctx.accounts.creator` is -- not the actual project creator. An attacker passes their own wallet as `creator` and steals the creator's share.
- **Impact:** Attacker steals creator_bps share (typically 50%) of all SOL raised in any auto-closed cycle.
- **Fix:** Add constraint: `#[account(mut, constraint = creator.key() == project_state.creator @ MammothError::Unauthorized)]`

#### SC-2: close_cycle can be called repeatedly, draining all cycle lamports
- **Lines:** 828-899
- **Description:** No guard prevents `close_cycle` from being called again after funds are distributed. The status check allows entry when `Closed`. Each call recalculates `distributable` from remaining lamports and sends them out. Repeated calls drain the CycleState PDA below rent-exempt minimum.
- **Impact:** Repeated fund drainage; potential account destruction.
- **Fix:** Add `require!(cycle.status == CycleStatus::Active, ...)` before distribution, or track a `distributed` boolean.

#### SC-3: exercise_rights has no supply_cap check -- can drain escrow
- **Lines:** 626-709
- **Description:** `exercise_rights` checks personal rights allocation but never checks `cycle.minted + amount <= cycle.supply_cap`. If total allocated rights exceed supply_cap, holders can exercise beyond the cycle limit, draining the entire project escrow.
- **Impact:** Tokens minted beyond cycle supply cap. Could drain entire escrow.
- **Fix:** Add: `require!(cycle.minted.checked_add(amount)? <= cycle.supply_cap, MammothError::SupplyCapExceeded);`

#### SC-4: exercise_rights takes no protocol fee -- fee bypass
- **Lines:** 646-665
- **Description:** When rights are exercised, SOL transfers at base_price with no protocol fee. Compare with `buy_tokens` which deducts `fee_bps` (2%). This creates a fee-free path that bypasses protocol revenue.
- **Impact:** Protocol treasury receives zero fees from rights exercises. May be intentional but needs explicit documentation.

### HIGH

#### SC-5: Price is per raw token unit, not per token -- economic miscalculation
- **Lines:** 722-726, 324-377
- **Description:** `compute_price` returns lamports per smallest unit (with 6 decimals, 1 token = 1,000,000 units). `total_cost = price * amount` where amount is raw units. Base_price documentation doesn't clarify this, leading to pricing confusion.
- **Impact:** Tokens either wildly overpriced or underpriced depending on caller's understanding.
- **Fix:** Document clearly that base_price is per smallest unit, or adjust the calculation.

#### SC-6: sol_raised tracks inconsistent amounts
- **Lines:** 703-705 vs 795-797
- **Description:** `exercise_rights` adds `total_cost` (gross) to `sol_raised`. `buy_tokens` adds `net_cost` (after fee). sol_raised is neither gross nor net consistently.
- **Impact:** Incorrect accounting for off-chain systems.

#### SC-7: close_cycle ignores authority delegation
- **Lines:** 828-837
- **Description:** Only checks `project.creator == creator.key()`, never uses `check_authority`. The `AuthorityConfig.can_close_cycle` field is dead code.
- **Impact:** Delegated operators cannot close cycles despite having permission.

#### SC-8: spending_limit_lamports is never enforced
- **Lines:** 233, 244-268
- **Description:** `AuthorityConfig.spending_limit_lamports` is stored but never checked. An operator with a 1 SOL limit could open a cycle raising 1000 SOL.
- **Impact:** AI operator spending guardrails are non-functional.

#### SC-9: No validation that cycle supply_cap <= available escrow
- **Lines:** 549-623
- **Description:** A creator can open a cycle with supply_cap = 1 trillion tokens with only 500k in escrow. No cumulative cap tracking across cycles.
- **Impact:** Impossible-to-fill cycles; confusing buy failures at purchase time.

### MEDIUM

#### SC-10: Linear curve pricing affected by rights-exercised tokens
- **Lines:** 335-347
- **Description:** Price interpolation uses `minted / supply_cap` but `minted` includes rights-window tokens at base_price. First public buyer may pay significantly above base_price.

#### SC-11: BPS splits not validated to sum correctly in create_project
- **Lines:** 407-546
- **Description:** `creator_bps + reserve_bps + sink_bps` should equal 10000 but is never checked. `saturating_sub` silently handles underflow.
- **Impact:** Silent incorrect token/SOL allocation.

#### SC-12: Merkle root can be overwritten repeatedly
- **Lines:** 996-1026
- **Description:** Creator can set new merkle root after some holders have claimed, invalidating remaining claims.

#### SC-13: set_rights_merkle_root uses "open_cycle" permission
- **Line:** 1009
- **Description:** Reuses open_cycle permission instead of dedicated permission. Permission conflation.

#### SC-14: rights_window_duration can be negative (instant expiry)
- **Line:** 590

#### SC-15: cycle_index is u8 -- max 255 cycles per project
- **Lines:** 163, 580, 598-600
- **Impact:** Projects permanently unable to create new cycles after 255.

#### SC-16: Only one operator per project (AuthorityConfig PDA)
- **Lines:** 1534-1541

### LOW

#### SC-17: activate_cycle is permissionless (documented as intentional)
#### SC-18: HolderRights expiry field is never checked
#### SC-19-20: ProjectState::LEN and CycleState::LEN calculations (verified correct)
#### SC-21: No constraint linking cycle_state.project to project_state in ExerciseRights (PDA seeds sufficient)

### INFO

#### SC-22: Major test coverage gaps (no tests for exercise_rights, close_cycle, Elastic mode, Step/Exp curves, merkle claims, authority)
#### SC-23: Elastic supply mode appears incomplete (no additional minting logic)
#### SC-24: Freeze authority set but unused

---

## SDK (mammoth-sdk) -- 27 Issues

### CRITICAL

#### SDK-1: Hash function mismatch: SHA-256 vs Keccak-256
- **File:** merkle.js:41-45
- **Description:** Comments say keccak256 everywhere, but implementation uses `createHash('sha256')`. If on-chain uses keccak, all Merkle proofs silently fail. Rights become unexercisable.
- **Fix:** Verify on-chain hash function and match it. Update comments regardless.

#### SDK-2: Linear/ExpLite buy quote uses spot price instead of integrating the curve
- **File:** curves.js:181-194
- **Description:** `computeBuyQuote` divides budget by current spot price for non-step curves. This overestimates tokens out because price increases as tokens are bought. For a linear curve from 0.001 to 0.01 SOL, error can be 10-25%.
- **Fix:** Integrate the curve (trapezoidal for linear, integral of exponential for ExpLite).

#### SDK-3: computeBuyQuote returns stale newPrice for linear/ExpLite
- **File:** curves.js:190
- **Description:** Returns pre-purchase price, not post-purchase price.

#### SDK-4: Fee calculation uses floating-point -- won't match on-chain integer math
- **File:** curves.js:127-128
- **Description:** `solIn * (feeBps / 10000)` uses float division. On-chain uses integer: `amount * 200 / 10000`. Rounding differences can cause transaction failures.
- **Fix:** Convert to lamports first, compute as integers.

### HIGH

#### SDK-5: cycleIndex >= 256 silently wraps via Buffer.from([cycleIndex])
- **File:** pdas.js:64
- **Description:** 256 becomes 0, 257 becomes 1. Operations target wrong cycle.
- **Impact:** Fund loss or unintended state changes.

#### SDK-6: createProject does not pass operatorType to on-chain instruction
- **File:** instructions.js:83-92
- **Description:** `operatorType` is silently dropped. AI disclosure metadata never stored on-chain.

#### SDK-7: MammothClient._getProgram creates new Program instance on every call
- **File:** client.js:63-86
- **Impact:** Significant performance degradation in bot loops.

#### SDK-8: Error parser false positives (msg.includes('6000') matches unrelated strings)
- **File:** errors.js:104
- **Impact:** Errors misclassified, retry logic takes wrong branches.

#### SDK-9: Floating-point precision loss in solToLamports
- **File:** curves.js:34
- **Description:** `Math.floor(0.1 * 1e9)` can lose 1 lamport due to float representation.

#### SDK-10: index.js imports non-existent MammothMonitor from merkle.js
- **File:** index.js:26
- **Description:** Silent `undefined` assignment. Copy-paste error.

### MEDIUM

#### SDK-11: getCycleSnapshot missing ExpLite price computation
- **File:** monitor.js:314-322
- **Description:** ExpLite curves always show base_price regardless of tokens sold.

#### SDK-12: No BPS sum validation in createProject
- **File:** instructions.js:52-113

#### SDK-13: fetchAllProjects has no pagination -- breaks at scale
- **File:** queries.js:22-32
- **Description:** `getProgramAccounts` loads everything into memory. Will hit RPC limits at thousands of projects.

#### SDK-14: N+1 query problem in monitor bot utilities
- **File:** monitor.js:210-275
- **Description:** 100 projects = 101+ RPC calls. Hits rate limits.

#### SDK-15: Step curve partial fill loses remaining budget
- **File:** curves.js:155-159

#### SDK-16: buildRightsTree pro-rata floor rounding loses rights
- **File:** merkle.js:149
- **Description:** 1000 holders with 1 token each, allocation=999: each gets floor(999/1000)=0. All rights lost.

#### SDK-17: Single-entry Merkle tree produces empty proof
- **File:** merkle.js:175-191

#### SDK-18: ExpLite overflow with large k values (Math.exp returns Infinity)
- **File:** curves.js:89

### LOW

#### SDK-19-23: totalSupply falsy check, null publicKey stub, fetchAuthorityConfig swallows all errors, empty catch blocks in monitor, removeListener cleanup

### INFO

#### SDK-24-27: Duplicate JSDoc blocks, mixed sync/async imports, inconsistent LAMPORTS_PER_SOL sources

---

## MCP SERVER (mammoth-mcp) -- 13 Issues

### CRITICAL

#### MCP-1: No confirmation gate on buy execution
- **Lines:** 311-338
- **Description:** `mammoth_buy_tokens` executes an on-chain purchase the moment an LLM agent calls it. No confirmation, no spending limit, no human-in-the-loop. A prompt-injected agent can drain the wallet.
- **Impact:** Adversarial prompt injection via on-chain project names could instruct the agent to buy tokens on a malicious cycle.
- **Fix:** Add spending cap via `MAMMOTH_MAX_BUY_LAMPORTS` env var. Require confirmation token before execution.

### HIGH

#### MCP-2: Wallet key could leak in error messages
- **Lines:** 43-60
- **Description:** If `Keypair.fromSecretKey` throws with input in message, raw secret key leaks to stderr. Current catch blocks are safe but fragile.

#### MCP-3: Malformed wallet key silently degrades to read-only
- **Lines:** 40-63
- **Description:** If `MAMMOTH_WALLET_KEY` is set but unparseable, server starts read-only. Operator thinks buys are armed.
- **Fix:** Exit with error if key is present but unparseable.

#### MCP-4: No validation that mintAddress is a valid Solana public key
- **Lines:** 125, 262, 316
- **Description:** `z.string()` with no format constraint. Garbage strings forwarded to SDK.

### MEDIUM

#### MCP-5: Buy quote hardcodes fee at 200 bps
#### MCP-6: Buy quote is flat-price approximation, ignores curve movement
#### MCP-7: Unbounded result sets from get_open_cycles / get_projects
#### MCP-8: Explorer URL hardcoded to devnet regardless of cluster

### LOW

#### MCP-9: MammothClient constructor failure not caught at startup
#### MCP-10: walletAddress not validated in check_rights
#### MCP-11: signAllTransactions mutation pattern

### INFO

#### MCP-12: No rate limiting on tool calls
#### MCP-13: Hardcoded version string

---

## ANDROID APP (mammoth-android) -- 28 Issues

### CRITICAL

#### APP-1: Mock wallet enabled by default with no production guard
- **File:** wallet.js:118, ProfileScreen.js:34
- **Description:** `connectWallet({ useMock: true })` is hardcoded. No `__DEV__` gate, no build variant check. Mock wallet uses pass-through signTransaction that would submit unsigned transactions.
- **Fix:** Gate behind `__DEV__`. Add runtime assertion rejecting mock on mainnet.

#### APP-2: Mock signTransaction is pass-through -- would submit unsigned txs
- **File:** wallet.js:97-98
- **Description:** `signTransaction: async (tx) => tx` returns the transaction without signing.

#### APP-3: BuyPanel accepts negative amounts
- **File:** BuyPanel.js:53-55
- **Description:** `parseFloat("-0.5")` is not caught by the existing checks. Negative amounts flow into curve calculations producing negative token counts.
- **Fix:** Add `if (v <= 0) return null;`

### HIGH

#### APP-4: No treasury BPS sum validation in LaunchWizard
- **File:** LaunchWizardScreen.js:36-48

#### APP-5: Linear/Exp-Lite buy quote ignores price curve (same as SDK-2)
- **File:** BuyPanel.js:39-50

#### APP-6: curves.js floating-point accumulation vs on-chain integer math
- **File:** curves.js:10-41

#### APP-7: Single-byte cycleIndex overflows at 256 (same as SDK-5)
- **File:** solana.js:45-49

#### APP-8: No wallet-disconnect handling mid-transaction
- **File:** BuyPanel.js:53-73, CreateCycleScreen.js:55-91

#### APP-9: Mock data shapes will break real on-chain integration
- **File:** data.js (entire file)

### MEDIUM

#### APP-10: Stale closure over projects in AppContext.connectWallet
#### APP-11: Balance fetch race with disconnect (setTimeout, no cancellation)
#### APP-12: No validation for negative BPS values in LaunchWizard
#### APP-13: Missing validation for stepIncrement, endPrice, expK in CreateCycleScreen
#### APP-14: Portfolio holdings randomized on every mount
#### APP-15: useEffect missing loadHoldings in dependency array
#### APP-16: CreatorDashboard "Connect Wallet" navigates to wrong screen
#### APP-17: Skeleton animation loop never cleaned up (memory leak)
#### APP-18: Toast animated values not reset between shows

### LOW

#### APP-19: Pull-to-refresh no error handling
#### APP-20: TabIcon ignores color prop
#### APP-21: Missing "COMPLETED" filter (doesn't match mock data status)
#### APP-22: IronHide treasury routing sums to 98%
#### APP-23: fmtTokens edge cases (999999 -> "1000K")
#### APP-24: fmtSOL crashes on null input
#### APP-25: Cycle history uses array index as key

### INFO

#### APP-26: XSS risk minimal (React Native Text)
#### APP-27: No hardcoded secrets found
#### APP-28: Empty states properly handled

---

## Priority Fix Order

### IMMEDIATE (before any deployment)
1. **SC-1**: close_cycle fund theft (contract)
2. **SC-2**: close_cycle repeated drain (contract)
3. **SC-3**: exercise_rights supply cap bypass (contract)
4. **MCP-1**: Buy execution without confirmation gate (MCP)

### HIGH PRIORITY (before public use)
5. **SDK-1**: Verify hash function matches on-chain (SDK)
6. **SC-9**: Validate cycle supply_cap vs escrow (contract)
7. **SC-8**: Enforce spending_limit_lamports (contract)
8. **SDK-2**: Fix curve integration in buy quotes (SDK)
9. **SDK-4**: Fix floating-point fee calculation (SDK)
10. **SDK-5**: Validate cycleIndex range (SDK)
11. **APP-1**: Gate mock wallet behind __DEV__ (Android)
12. **SC-11**: Validate BPS sums (contract)

### BEFORE MAINNET
13. All remaining HIGH issues
14. Add comprehensive test coverage (SC-22)
15. Complete Elastic supply mode (SC-23)
16. Wire up real wallet integration (Android)
17. Add pagination to fetchAllProjects (SDK)

---

## Cross-Cutting Themes

1. **Floating-point vs integer math**: The SDK and Android app use JavaScript floats for price/fee calculations that must match on-chain Rust integer math. This mismatch pattern appears in 6+ issues across all JS codebases.

2. **Missing input validation**: BPS sums, negative amounts, cycleIndex bounds, and mint address format are rarely validated at any layer. The contract relies on runtime panics, the SDK passes garbage through, and the app has minimal form validation.

3. **Incomplete authority delegation**: `spending_limit_lamports`, `can_close_cycle`, and `operatorType` are all stored but never enforced or transmitted. The AI operator guardrail system is non-functional.

4. **Test coverage gaps**: The contract tests cover happy-path basics only. exercise_rights, close_cycle (the most critical paths), Elastic mode, and authority delegation are completely untested.

5. **Mock vs real data boundary**: The Android app is entirely mock-driven with no adapter layer. When real on-chain integration happens, every screen will need rework.
