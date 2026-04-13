# Contract 1: mammoth_token — SPL Token (Fixed Supply)

**Status:** Spec locked / Pre-build
**Chain:** Solana
**Framework:** Anchor
**Program ID:** TBD (assigned on deploy)

---

## Purpose

Deploy the project token on Solana. Establishes total supply, mints genesis allocation, enforces the 2% protocol stake, and transfers mint authority to the Cycle Manager PDA.

This is the foundation layer. Every other Mammoth contract depends on this being solid first.

---

## Responsibilities

### 1. Token Initialization
- Accept: `name`, `symbol`, `decimals` (default: 6), `total_supply` (default: 1,000,000,000)
- Mint full supply at genesis — no post-deploy minting in fixed supply mode
- Store metadata on-chain via Metaplex Token Metadata Program

### 2. Genesis Allocation Split
At deploy time, supply divides as follows:

```
total_supply
  └── protocol_allocation = total_supply * 2%  → Mammoth treasury wallet
  └── public_allocation                         → Protocol escrow PDA (released per cycle)
  └── treasury_allocation                       → Creator's wallet
```

The public/treasury split ratio is set by the creator at deploy. Protocol 2% is taken off the top first.

### 3. Mint Authority Transfer
- After genesis mint, mint authority transfers to the Cycle Manager PDA
- Transfer happens **in the same transaction as the genesis mint** — not a follow-up call
- Creator cannot retain mint authority — enforced at contract level, not convention

### 4. Freeze Authority
- Set to `null` at genesis — no exceptions
- Freeze authority is a rug vector with no legitimate use in this protocol

---

## On-Chain State

```rust
#[account]
pub struct TokenConfig {
    pub creator: Pubkey,
    pub total_supply: u64,
    pub public_allocation: u64,       // escrowed, released per cycle
    pub treasury_allocation: u64,     // sent to creator wallet at genesis
    pub protocol_allocation: u64,     // 2% → Mammoth treasury
    pub supply_mode: SupplyMode,      // FixedSupply | ElasticCycleBounded
    pub cycle_manager: Pubkey,        // receives mint authority
    pub deployed_at: i64,             // Unix timestamp
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub enum SupplyMode {
    FixedSupply,
    ElasticCycleBounded,
}
```

---

## Instructions

### `initialize_token`
Accounts:
- `creator` (signer)
- `mint` (new SPL mint)
- `token_config` (PDA: seeds = ["token_config", mint.key()])
- `mammoth_treasury` (Mammoth's protocol wallet — hardcoded)
- `creator_token_account` (receives treasury_allocation)
- `escrow_pda` (receives public_allocation)
- `metadata_account` (Metaplex)
- `token_program`
- `system_program`
- `rent`

Params:
- `name: String`
- `symbol: String`
- `uri: String` (metadata URI)
- `total_supply: u64`
- `treasury_pct: u8` (creator's % of post-protocol supply; remainder = public_allocation)
- `cycle_manager: Pubkey`

Validation:
- `treasury_pct` must be 0–100
- `cycle_manager` must be a valid pubkey
- `total_supply` must be > 0

Post-conditions:
- Mint authority = `cycle_manager`
- Freeze authority = `null`
- `protocol_allocation` transferred to `mammoth_treasury`
- `treasury_allocation` transferred to `creator_token_account`
- `public_allocation` held in `escrow_pda`

---

## Key Constraints

| Constraint | Enforcement |
|------------|-------------|
| Total supply immutable after deploy | No `set_supply` instruction |
| Mint authority transfers in same tx | Atomic — no separate transfer call |
| Protocol 2% non-configurable | Hardcoded constant in contract |
| Freeze authority null | Set in initialize, no setter |
| No admin override | No `upgrade_authority` backdoor |

---

## Dependencies

- **SPL Token Program** — standard token operations
- **Metaplex Token Metadata Program** — on-chain name/symbol/URI
- **Cycle Manager PDA** — address must be derived or passed in before deploy

---

## Out of Scope (this contract)

- Elastic supply mode (second pass)
- Cycle logic
- Rights issuance
- Treasury routing per cycle

---

## Design Decisions (Locked 2026-03-26)

1. **Multisig creators** — `creator` supports BOTH regular wallet and multisig. Enforce via Anchor's `Signer` constraint; multisig wallets are compatible natively.
2. **Metadata storage** — IPFS URI stored on-chain via Metaplex Token Metadata Program. Minimal on-chain footprint; name/symbol/URI pointer only.
3. **Protocol treasury wallet** — PDA governed by the Mammoth program. Not hardcoded. Derived from program seeds.
4. **Upgrade authority** — Retain Solana upgrade authority for MVP. Burn it post-audit when protocol is battle-tested. Same program ID survives upgrades — no user migration needed.
5. **Project ID scheme** — Mint pubkey is the canonical project identifier across all contracts. No separate project ID. Every contract seeds PDAs from the mint pubkey.

---

## What Comes Next

**Contract 2: Cycle Manager** — receives mint authority from this contract, governs cycle state, allocation release, bonding curve pricing, and cycle termination.
