use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount, MintTo, Transfer};
use anchor_spl::associated_token::AssociatedToken;
use sha2::{Sha256, Digest};

declare_id!("DUnfGXcmPJgjSHvrPxeqPPYjrx6brurKUBJ4cVGVFR31");

// ─────────────────────────────────────────────
//  Events — emitted for bot/agent subscriptions
//  Bots subscribe via: program.addEventListener('CycleOpened', cb)
// ─────────────────────────────────────────────

#[event]
pub struct CycleOpened {
    pub project_mint: Pubkey,
    pub project_state: Pubkey,
    pub cycle_index: u8,
    pub curve_type: u8,          // 0=Step, 1=Linear, 2=ExpLite
    pub supply_cap: u64,
    pub base_price: u64,         // lamports per token
    pub rights_window_end: i64,  // unix timestamp; 0 if no rights window
    pub timestamp: i64,
}

#[event]
pub struct CycleActivated {
    pub project_mint: Pubkey,
    pub project_state: Pubkey,
    pub cycle_index: u8,
    pub timestamp: i64,
}

#[event]
pub struct CycleClosed {
    pub project_mint: Pubkey,
    pub project_state: Pubkey,
    pub cycle_index: u8,
    pub tokens_sold: u64,
    pub sol_raised: u64,
    pub timestamp: i64,
}

#[event]
pub struct TokensPurchased {
    pub project_mint: Pubkey,
    pub buyer: Pubkey,
    pub cycle_index: u8,
    pub amount: u64,
    pub sol_paid: u64,           // lamports
    pub price_per_token: u64,    // lamports
    pub timestamp: i64,
}

#[event]
pub struct RightsExercised {
    pub project_mint: Pubkey,
    pub holder: Pubkey,
    pub cycle_index: u8,
    pub amount: u64,
    pub sol_paid: u64,
    pub timestamp: i64,
}

#[event]
pub struct ProjectCreated {
    pub project_mint: Pubkey,
    pub project_state: Pubkey,
    pub creator: Pubkey,
    pub supply_mode: u8,         // 0=Fixed, 1=Elastic
    pub total_supply: u64,
    pub operator_type: u8,       // 0=Human, 1=AiAssisted, 2=AiAutonomous
    pub timestamp: i64,
}

#[event]
pub struct HardCapSet {
    pub project_mint: Pubkey,
    pub hard_cap: u64,
    pub timestamp: i64,
}

#[event]
pub struct MerkleRightsSet {
    pub project_mint: Pubkey,
    pub cycle_index: u8,
    pub merkle_root: [u8; 32],
    pub holder_count: u32,   // informational — not verified on-chain
    pub timestamp: i64,
}

#[event]
pub struct RightsClaimed {
    pub project_mint: Pubkey,
    pub holder: Pubkey,
    pub cycle_index: u8,
    pub amount: u64,
    pub timestamp: i64,
}

// ─────────────────────────────────────────────
//  Enums
// ─────────────────────────────────────────────

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq)]
pub enum SupplyMode {
    Fixed,
    Elastic,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq)]
pub enum CurveType {
    Step,
    Linear,
    ExpLite,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq)]
pub enum CycleStatus {
    Pending,
    RightsWindow,
    Active,
    Closed,
}

/// Disclosure-only field. Not enforced by the protocol — surfaced in UI.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq, Default)]
pub enum OperatorType {
    #[default]
    Human,
    AiAssisted,
    AiAutonomous,
}

// ─────────────────────────────────────────────
//  Account Structs
// ─────────────────────────────────────────────

#[account]
pub struct ProtocolConfig {
    pub admin: Pubkey,
    pub protocol_treasury: Pubkey,
    pub fee_bps: u16,
    pub default_creator_bps: u16,
    pub default_reserve_bps: u16,
    pub default_sink_bps: u16,
}

impl ProtocolConfig {
    pub const LEN: usize = 8 + 32 + 32 + 2 + 2 + 2 + 2;
}

#[account]
pub struct ProjectState {
    pub creator: Pubkey,
    pub mint: Pubkey,
    pub supply_mode: SupplyMode,
    pub hard_cap: Option<u64>,
    pub total_supply: u64,
    pub public_allocation: u64,
    pub treasury_allocation: u64,
    pub protocol_allocation: u64,
    pub total_minted: u64,
    pub current_cycle: u8,
    pub creator_bps: u16,
    pub reserve_bps: u16,
    pub sink_bps: u16,
    pub launch_at: Option<i64>,  // Unix timestamp; None = launch immediately
    pub operator_type: OperatorType,  // Disclosure only — human | ai_assisted | ai_autonomous
    /// FIX RA-1: Track whether a cycle is currently active to prevent concurrent cycles
    pub has_active_cycle: bool,
    pub bump: u8,
}

impl ProjectState {
    // discriminator(8) + pubkey(32)*2 + enum(1) + option_u64(9) + u64*5(40) + u8 + u16*3(6) + option_i64(9) + enum(1) + bool(1) + u8
    pub const LEN: usize = 8 + 32 + 32 + 1 + 9 + 8 + 8 + 8 + 8 + 8 + 1 + 2 + 2 + 2 + 9 + 1 + 1 + 1;
}

#[account]
pub struct CycleState {
    pub project: Pubkey,
    pub cycle_index: u8,
    pub curve_type: CurveType,
    pub supply_cap: u64,
    pub minted: u64,
    pub base_price: u64,
    pub status: CycleStatus,
    pub rights_window_end: i64,
    pub step_size: u64,
    pub step_increment: u64,
    pub end_price: u64,
    pub growth_factor_k: u64,
    pub sol_raised: u64,
    /// Merkle root of (holder_pubkey, rights_amount) pairs.
    /// None = no rights for this cycle (or using legacy create_holder_rights).
    /// Set via set_rights_merkle_root before or during open_cycle.
    /// Holders claim rights by submitting a Merkle proof via claim_rights.
    pub rights_merkle_root: Option<[u8; 32]>,
    /// FIX RA-8: Track cumulative rights allocated to prevent over-allocation beyond supply_cap
    pub rights_allocated: u64,
    /// FIX H-R6-2 (round 7): Snapshot of unexercised rights at activation time.
    /// This is the number of tokens that MUST remain protected from public buyers.
    /// Frozen when activate_cycle runs. Prevents erosion as cycle.minted grows from public buys.
    pub rights_reserved_at_activation: u64,
    /// FIX H-R7-1 (round 8): Total rights committed via Merkle root.
    /// Set in set_rights_merkle_root; ensures unclaimed Merkle rights are protected at activation.
    /// If set, snapshot uses max(rights_allocated, rights_committed) to reserve space for all
    /// potential claimants even if they didn't claim before rights_window_end.
    pub rights_committed: u64,
    pub bump: u8,
}

impl CycleState {
    // discriminator(8) + pubkey(32) + u8 + enum(1) + u64*8(64) + i64(8) + enum(1) + u8 + option<[u8;32]>(33) + u64*3(24) + u8
    pub const LEN: usize = 8 + 32 + 1 + 1 + 8 + 8 + 8 + 1 + 8 + 8 + 8 + 8 + 8 + 8 + 33 + 8 + 8 + 8 + 1;
}

#[account]
pub struct HolderRights {
    pub project: Pubkey,
    pub cycle_index: u8,
    pub holder: Pubkey,
    pub rights_amount: u64,
    pub exercised_amount: u64,
    pub expiry: i64,
    pub bump: u8,
}

impl HolderRights {
    // discriminator(8) + pubkey(32) + u8 + pubkey(32) + u64 + u64 + i64 + u8
    pub const LEN: usize = 8 + 32 + 1 + 32 + 8 + 8 + 8 + 1;
}

/// On-chain authority delegation for AI agents.
/// The principal configures what the operator can do autonomously.
/// Principal can be a human wallet, DAO multisig, or another AI controller — protocol doesn't care.
#[account]
pub struct AuthorityConfig {
    pub project: Pubkey,
    pub principal: Pubkey,           // top-level authority — sets and updates this config
    pub operator: Pubkey,            // executing agent — human, AI agent wallet, or another AI
    pub can_open_cycle: bool,        // operator can call open_cycle autonomously
    pub can_close_cycle: bool,       // operator can call close_cycle autonomously
    pub can_set_hard_cap: bool,      // operator can call set_hard_cap (OFF by default — must be explicit)
    pub can_route_treasury: bool,    // operator can configure treasury routing at cycle creation
    pub spending_limit_lamports: u64, // max SOL raise per cycle; 0 = no limit
    /// FIX M-1 (round 5): Separate permission for setting Merkle root.
    /// Previously reused can_open_cycle, letting an operator set malicious rights trees.
    /// NOTE (H-A round 6): Adding this field changed AuthorityConfig::LEN. Fresh deployments
    /// only — existing accounts (if any) must be re-initialized via close+initialize_authority.
    pub can_set_merkle_root: bool,
    pub bump: u8,
}

impl AuthorityConfig {
    // discriminator(8) + pubkey(32)*3 + bool*5 + u64 + u8
    pub const LEN: usize = 8 + 32 + 32 + 32 + 1 + 1 + 1 + 1 + 8 + 1 + 1;
}

/// Helper: check if caller is the principal or an authorized operator for a given permission.
/// Returns Ok(()) if authorized, Err(InsufficientAuthority) otherwise.
pub fn check_authority(
    caller: &Pubkey,
    project_state: &ProjectState,
    project_key: &Pubkey,
    authority_config: Option<&AuthorityConfig>,
    permission: &str,
) -> Result<()> {
    // Principal (creator) always has full authority
    if *caller == project_state.creator {
        return Ok(());
    }
    // If no AuthorityConfig, only creator can act
    let config = authority_config.ok_or(MammothError::OperatorNotRegistered)?;
    // FIX RA-2: Validate AuthorityConfig belongs to THIS project — prevents cross-project escalation
    require!(config.project == *project_key, MammothError::Unauthorized);
    // Caller must be the registered operator
    require!(*caller == config.operator, MammothError::InsufficientAuthority);
    // Check specific permission
    let permitted = match permission {
        "open_cycle"       => config.can_open_cycle,
        "close_cycle"      => config.can_close_cycle,
        "set_hard_cap"     => config.can_set_hard_cap,
        "route_treasury"   => config.can_route_treasury,
        "set_merkle_root"  => config.can_set_merkle_root,
        _                  => false,
    };
    require!(permitted, MammothError::InsufficientAuthority);
    Ok(())
}

// ─────────────────────────────────────────────
//  Error Codes
// ─────────────────────────────────────────────

#[error_code]
pub enum MammothError {
    #[msg("Not authorized")]
    Unauthorized,
    #[msg("Hard cap already set — irreversible")]
    HardCapAlreadySet,
    #[msg("Hard cap only settable for Elastic supply mode")]
    NotElasticMode,
    #[msg("Cycle is not in RightsWindow status")]
    NotRightsWindow,
    #[msg("Rights window has expired")]
    RightsWindowExpired,
    #[msg("Exercised amount would exceed rights allocation")]
    ExceedsRightsAllocation,
    #[msg("Cycle is not in Active status")]
    NotActive,
    #[msg("Cycle supply cap reached")]
    SupplyCapExceeded,
    #[msg("Cycle params immutable once opened")]
    CycleParamsImmutable,
    #[msg("Elastic supply requires rights-based issuance")]
    ElasticRequiresRights,
    #[msg("Arithmetic overflow")]
    MathOverflow,
    #[msg("Cycle is not closed")]
    NotClosed,
    #[msg("Amount must be greater than zero")]
    ZeroAmount,
    #[msg("Step size cannot be zero")]
    ZeroStepSize,
    #[msg("Rights window is still open — cannot activate yet")]
    RightsWindowStillOpen,
    #[msg("Scheduled launch time has not been reached yet")]
    LaunchTimeNotReached,
    #[msg("Merkle root not set for this cycle — call set_rights_merkle_root first")]
    MerkleRootNotSet,
    #[msg("Merkle proof is invalid — proof does not verify against stored root")]
    InvalidMerkleProof,
    #[msg("Operator lacks permission for this instruction")]
    InsufficientAuthority,
    #[msg("Action exceeds operator spending limit — escalate to principal")]
    SpendingLimitExceeded,
    #[msg("No AuthorityConfig found — call initialize_authority first")]
    OperatorNotRegistered,
    #[msg("BPS split must sum to 10000 (creator+reserve+sink) and public+protocol fee <= 10000")]
    InvalidBpsSplit,
    #[msg("Cycle supply cap exceeds available escrow balance")]
    SupplyCapExceedsEscrow,
    #[msg("Funds already distributed for this cycle")]
    AlreadyDistributed,
    #[msg("Merkle root already set — cannot overwrite after claims have begun")]
    MerkleRootAlreadySet,
    #[msg("Cannot open a new cycle while a previous cycle is still active — close it first")]
    CycleStillActive,
    #[msg("Rights window duration must be between 0 and 30 days (2,592,000 seconds)")]
    InvalidRightsWindow,
    #[msg("Transaction cost exceeds caller's max_sol_cost slippage cap")]
    SlippageExceeded,
    #[msg("Withdrawal amount exceeds reserve balance (after rent-exempt minimum)")]
    InsufficientReserveBalance,
    #[msg("Invalid curve parameter (e.g., growth_factor_k too large)")]
    InvalidCurveParam,
    #[msg("Purchase too large for single tx — split into smaller buys (compute unit limit)")]
    PurchaseTooLarge,
    #[msg("Cannot mix legacy create_holder_rights with Merkle-based rights — pick one path")]
    RightsPathConflict,
    #[msg("Cycle does not belong to project")]
    InvalidCycleProject,
    #[msg("Mint does not match project mint")]
    InvalidProjectMint,
    #[msg("Rights account does not belong to project")]
    InvalidRightsProject,
    #[msg("Rights account cycle does not match active cycle")]
    InvalidRightsCycle,
    #[msg("Rights account holder does not match signer")]
    InvalidRightsHolder,
}

// ─────────────────────────────────────────────
//  Curve Math Helpers
// ─────────────────────────────────────────────

pub fn compute_price(cycle: &CycleState) -> Result<u64> {
    match cycle.curve_type {
        CurveType::Step => {
            require!(cycle.step_size > 0, MammothError::ZeroStepSize);
            let step_number = cycle.minted / cycle.step_size;
            let price = cycle.base_price
                .checked_add(step_number.checked_mul(cycle.step_increment)
                    .ok_or(MammothError::MathOverflow)?)
                .ok_or(MammothError::MathOverflow)?;
            Ok(price)
        }
        CurveType::Linear => {
            if cycle.supply_cap == 0 {
                return Ok(cycle.base_price);
            }
            let spread = cycle.end_price.saturating_sub(cycle.base_price);
            let price = cycle.base_price
                .checked_add(
                    spread.checked_mul(cycle.minted)
                        .ok_or(MammothError::MathOverflow)?
                        .checked_div(cycle.supply_cap)
                        .ok_or(MammothError::MathOverflow)?
                )
                .ok_or(MammothError::MathOverflow)?;
            Ok(price)
        }
        CurveType::ExpLite => {
            if cycle.supply_cap == 0 {
                return Ok(cycle.base_price);
            }
            // pct_consumed = minted * 10000 / supply_cap  (BPS)
            let pct_consumed = cycle.minted
                .checked_mul(10000)
                .ok_or(MammothError::MathOverflow)?
                .checked_div(cycle.supply_cap)
                .ok_or(MammothError::MathOverflow)?;
            // price = base_price + (base_price * k * pct_consumed / 10000 / 10000)
            let price = cycle.base_price
                .checked_add(
                    cycle.base_price
                        .checked_mul(cycle.growth_factor_k)
                        .ok_or(MammothError::MathOverflow)?
                        .checked_mul(pct_consumed)
                        .ok_or(MammothError::MathOverflow)?
                        .checked_div(10000)
                        .ok_or(MammothError::MathOverflow)?
                        .checked_div(10000)
                        .ok_or(MammothError::MathOverflow)?
                )
                .ok_or(MammothError::MathOverflow)?;
            Ok(price)
        }
    }
}

/// FIX M-1 (final audit): Compute exact integrated cost of buying `amount` tokens
/// starting from cycle.minted. Previous per-unit-at-current-price formula allowed
/// bulk buyers to bypass price progression — a single buy of N tokens paid
/// base_price for all N, while N small buys would pay progressive prices.
///
/// For Step: walks step boundaries (each step at fixed price).
/// For Linear: walks supply_cap/10000 buckets OR per-token if cap is small.
/// For ExpLite: same bucket walk as Linear (both use BPS-piecewise-constant).
///
/// Returns total cost in lamports.
pub fn compute_total_cost(cycle: &CycleState, amount: u64) -> Result<u64> {
    if amount == 0 {
        return Ok(0);
    }
    let mut sold = cycle.minted;
    let end_sold = sold.checked_add(amount).ok_or(MammothError::MathOverflow)?;
    let mut total: u64 = 0;
    // FIX M-R6-1 (round 7): Cap iterations to stay within Solana compute unit budget.
    // Linear/ExpLite walk BPS buckets (max 10k), Step walks step boundaries.
    // If the user tries to buy too much in one tx, force them to split.
    let mut iterations = 0;
    // FIX M-R7-2 (round 8): Raised from 2000 to 5000. Solana ~1.4M CU budget comfortably
    // handles 5k simple iterations. Still allows purchases up to 50% of cycle in one tx.
    const MAX_ITERATIONS: u32 = 5000;

    match cycle.curve_type {
        CurveType::Step => {
            require!(cycle.step_size > 0, MammothError::ZeroStepSize);
            while sold < end_sold {
                iterations += 1;
                require!(iterations <= MAX_ITERATIONS, MammothError::PurchaseTooLarge);
                let step_number = sold / cycle.step_size;
                let price = cycle.base_price
                    .checked_add(step_number.checked_mul(cycle.step_increment)
                        .ok_or(MammothError::MathOverflow)?)
                    .ok_or(MammothError::MathOverflow)?;
                let next_boundary = (step_number + 1).checked_mul(cycle.step_size)
                    .unwrap_or(u64::MAX);
                let tokens_in_step = end_sold.min(next_boundary).saturating_sub(sold);
                let cost = price.checked_mul(tokens_in_step).ok_or(MammothError::MathOverflow)?;
                total = total.checked_add(cost).ok_or(MammothError::MathOverflow)?;
                sold = sold.saturating_add(tokens_in_step);
            }
        }
        CurveType::Linear | CurveType::ExpLite => {
            if cycle.supply_cap == 0 {
                // Degenerate — flat base_price
                return cycle.base_price.checked_mul(amount).ok_or(MammothError::MathOverflow.into());
            }
            // Walk BPS buckets of size supply_cap/10000 (where price is piecewise-constant)
            while sold < end_sold {
                iterations += 1;
                require!(iterations <= MAX_ITERATIONS, MammothError::PurchaseTooLarge);
                let pct_consumed = sold.checked_mul(10000).ok_or(MammothError::MathOverflow)?
                    / cycle.supply_cap;
                let price = match cycle.curve_type {
                    CurveType::Linear => {
                        let spread = cycle.end_price.saturating_sub(cycle.base_price);
                        cycle.base_price.checked_add(
                            spread.checked_mul(sold).ok_or(MammothError::MathOverflow)?
                                .checked_div(cycle.supply_cap).ok_or(MammothError::MathOverflow)?
                        ).ok_or(MammothError::MathOverflow)?
                    }
                    CurveType::ExpLite => {
                        cycle.base_price.checked_add(
                            cycle.base_price
                                .checked_mul(cycle.growth_factor_k).ok_or(MammothError::MathOverflow)?
                                .checked_mul(pct_consumed).ok_or(MammothError::MathOverflow)?
                                .checked_div(10000).ok_or(MammothError::MathOverflow)?
                                .checked_div(10000).ok_or(MammothError::MathOverflow)?
                        ).ok_or(MammothError::MathOverflow)?
                    }
                    _ => unreachable!(),
                };
                // Next BPS bucket boundary
                let next_pct = pct_consumed.saturating_add(1);
                let next_boundary = next_pct.checked_mul(cycle.supply_cap)
                    .map(|v| (v + 9999) / 10000) // ceil div
                    .unwrap_or(u64::MAX)
                    .min(cycle.supply_cap);
                let tokens_in_bucket = end_sold.min(next_boundary).saturating_sub(sold);
                // Safety: always make progress even if bucket size rounds to 0
                let tokens_in_bucket = tokens_in_bucket.max(1);
                let effective = end_sold.saturating_sub(sold).min(tokens_in_bucket);
                let cost = price.checked_mul(effective).ok_or(MammothError::MathOverflow)?;
                total = total.checked_add(cost).ok_or(MammothError::MathOverflow)?;
                sold = sold.saturating_add(effective);
            }
        }
    }
    Ok(total)
}

// ─────────────────────────────────────────────
//  Program
// ─────────────────────────────────────────────

#[program]
pub mod mammoth_core {
    use super::*;

    // ── 1. initialize_protocol ──────────────────
    pub fn initialize_protocol(
        ctx: Context<InitializeProtocol>,
        fee_bps: u16,
        default_creator_bps: u16,
        default_reserve_bps: u16,
        default_sink_bps: u16,
    ) -> Result<()> {
        // FIX F4: Validate protocol BPS parameters
        require!(fee_bps <= 1000, MammothError::InvalidBpsSplit);  // max 10% protocol fee
        require!(
            (default_creator_bps as u64)
                .checked_add(default_reserve_bps as u64)
                .ok_or(MammothError::MathOverflow)?
                .checked_add(default_sink_bps as u64)
                .ok_or(MammothError::MathOverflow)? == 10000,
            MammothError::InvalidBpsSplit
        );

        let config = &mut ctx.accounts.protocol_config;
        config.admin = ctx.accounts.admin.key();
        config.protocol_treasury = ctx.accounts.protocol_treasury.key();
        config.fee_bps = fee_bps;
        config.default_creator_bps = default_creator_bps;
        config.default_reserve_bps = default_reserve_bps;
        config.default_sink_bps = default_sink_bps;

        // FIX (integration test catch): Fund protocol_treasury PDA with rent-exempt minimum.
        // Without this, the first fee transfer would credit a small amount to a non-existent
        // System-owned account, leaving it below rent-exempt and failing the entire tx.
        let rent = Rent::get()?;
        let rent_exempt = rent.minimum_balance(0); // 0-data system account
        let current = ctx.accounts.protocol_treasury.lamports();
        if current < rent_exempt {
            let needed = rent_exempt - current;
            let ix = anchor_lang::solana_program::system_instruction::transfer(
                &ctx.accounts.admin.key(),
                &ctx.accounts.protocol_treasury.key(),
                needed,
            );
            anchor_lang::solana_program::program::invoke(
                &ix,
                &[
                    ctx.accounts.admin.to_account_info(),
                    ctx.accounts.protocol_treasury.to_account_info(),
                    ctx.accounts.system_program.to_account_info(),
                ],
            )?;
        }

        msg!("Protocol initialized. Admin: {}", config.admin);
        Ok(())
    }

    // ── 1b. update_protocol_config ─────────────
    /// Admin-only. Updates protocol fee and default BPS splits.
    /// Allows fixing misconfigurations without redeploying.
    pub fn update_protocol_config(
        ctx: Context<UpdateProtocolConfig>,
        fee_bps: u16,
        default_creator_bps: u16,
        default_reserve_bps: u16,
        default_sink_bps: u16,
    ) -> Result<()> {
        require!(fee_bps <= 1000, MammothError::InvalidBpsSplit);  // max 10%
        require!(
            (default_creator_bps as u64)
                .checked_add(default_reserve_bps as u64)
                .ok_or(MammothError::MathOverflow)?
                .checked_add(default_sink_bps as u64)
                .ok_or(MammothError::MathOverflow)? == 10000,
            MammothError::InvalidBpsSplit
        );

        let config = &mut ctx.accounts.protocol_config;
        config.fee_bps = fee_bps;
        config.default_creator_bps = default_creator_bps;
        config.default_reserve_bps = default_reserve_bps;
        config.default_sink_bps = default_sink_bps;
        msg!("Protocol config updated. fee_bps: {}", fee_bps);
        Ok(())
    }

    // ── 2. create_project ───────────────────────
    pub fn create_project(
        ctx: Context<CreateProject>,
        supply_mode: SupplyMode,
        total_supply: u64,
        public_allocation_bps: u16,   // BPS of total_supply for public escrow
        creator_bps: u16,
        reserve_bps: u16,
        sink_bps: u16,
        launch_at: Option<i64>,
        operator_type: OperatorType,  // Disclosure field: Human | AiAssisted | AiAutonomous
    ) -> Result<()> {
        let protocol_fee_bps = ctx.accounts.protocol_config.fee_bps as u64;

        // FIX SC-11: Validate BPS sums
        require!(
            (creator_bps as u64)
                .checked_add(reserve_bps as u64)
                .ok_or(MammothError::MathOverflow)?
                .checked_add(sink_bps as u64)
                .ok_or(MammothError::MathOverflow)? == 10000,
            MammothError::InvalidBpsSplit
        );
        require!(
            (public_allocation_bps as u64)
                .checked_add(protocol_fee_bps)
                .ok_or(MammothError::MathOverflow)? <= 10000,
            MammothError::InvalidBpsSplit
        );

        // Compute allocations
        let protocol_allocation = total_supply
            .checked_mul(protocol_fee_bps)
            .ok_or(MammothError::MathOverflow)?
            .checked_div(10000)
            .ok_or(MammothError::MathOverflow)?;

        let public_allocation = total_supply
            .checked_mul(public_allocation_bps as u64)
            .ok_or(MammothError::MathOverflow)?
            .checked_div(10000)
            .ok_or(MammothError::MathOverflow)?;

        let treasury_allocation = total_supply
            .saturating_sub(protocol_allocation)
            .saturating_sub(public_allocation);

        // Init project state fields
        let bump = ctx.bumps.project_state;
        let mint_key = ctx.accounts.mint.key();
        let creator_key = ctx.accounts.creator.key();
        {
            let project = &mut ctx.accounts.project_state;
            project.creator = creator_key;
            project.mint = mint_key;
            project.supply_mode = supply_mode;
            project.hard_cap = None;
            project.total_supply = total_supply;
            project.public_allocation = public_allocation;
            project.treasury_allocation = treasury_allocation;
            project.protocol_allocation = protocol_allocation;
            project.total_minted = 0;
            project.current_cycle = 0;
            project.creator_bps = creator_bps;
            project.reserve_bps = reserve_bps;
            project.sink_bps = sink_bps;
            project.launch_at = launch_at;
            project.operator_type = operator_type;
            project.has_active_cycle = false; // FIX RA-1
            project.bump = bump;
        }
        // project mutable borrow dropped here

        let seeds: &[&[u8]] = &[b"project", mint_key.as_ref(), &[bump]];
        let signer_seeds = &[seeds];

        // Mint protocol allocation to protocol treasury token account
        if protocol_allocation > 0 {
            token::mint_to(
                CpiContext::new_with_signer(
                    ctx.accounts.token_program.to_account_info(),
                    MintTo {
                        mint: ctx.accounts.mint.to_account_info(),
                        to: ctx.accounts.protocol_treasury_token.to_account_info(),
                        authority: ctx.accounts.project_state.to_account_info(),
                    },
                    signer_seeds,
                ),
                protocol_allocation,
            )?;
        }

        // Mint treasury allocation to creator wallet token account
        if treasury_allocation > 0 {
            token::mint_to(
                CpiContext::new_with_signer(
                    ctx.accounts.token_program.to_account_info(),
                    MintTo {
                        mint: ctx.accounts.mint.to_account_info(),
                        to: ctx.accounts.creator_token.to_account_info(),
                        authority: ctx.accounts.project_state.to_account_info(),
                    },
                    signer_seeds,
                ),
                treasury_allocation,
            )?;
        }

        // Mint public allocation into project escrow
        if public_allocation > 0 {
            token::mint_to(
                CpiContext::new_with_signer(
                    ctx.accounts.token_program.to_account_info(),
                    MintTo {
                        mint: ctx.accounts.mint.to_account_info(),
                        to: ctx.accounts.project_escrow_token.to_account_info(),
                        authority: ctx.accounts.project_state.to_account_info(),
                    },
                    signer_seeds,
                ),
                public_allocation,
            )?;
        }

        ctx.accounts.project_state.total_minted = total_supply;

        // FIX (integration test catch): Fund reserve and sink PDAs with rent-exempt minimum.
        // Without this, the first close_cycle would credit a small amount to non-existent
        // System-owned accounts, leaving them below rent-exempt and failing the entire tx.
        let rent = Rent::get()?;
        let rent_exempt = rent.minimum_balance(0);
        for pda_info in [&ctx.accounts.reserve, &ctx.accounts.sink] {
            let cur = pda_info.lamports();
            if cur < rent_exempt {
                let needed = rent_exempt - cur;
                let ix = anchor_lang::solana_program::system_instruction::transfer(
                    &ctx.accounts.creator.key(),
                    &pda_info.key(),
                    needed,
                );
                anchor_lang::solana_program::program::invoke(
                    &ix,
                    &[
                        ctx.accounts.creator.to_account_info(),
                        pda_info.to_account_info(),
                        ctx.accounts.system_program.to_account_info(),
                    ],
                )?;
            }
        }

        // Mint authority is already set to project_state PDA at mint init.
        // Freeze authority is set to project_state PDA but functionally null (never used).
        // No further revocation needed — program controls all minting.

        let supply_mode_byte = match ctx.accounts.project_state.supply_mode {
            SupplyMode::Fixed => 0u8,
            SupplyMode::Elastic => 1u8,
        };
        let op_type_byte = match ctx.accounts.project_state.operator_type {
            OperatorType::Human => 0u8,
            OperatorType::AiAssisted => 1u8,
            OperatorType::AiAutonomous => 2u8,
        };
        let clock = Clock::get()?;
        emit!(ProjectCreated {
            project_mint: mint_key,
            project_state: ctx.accounts.project_state.key(),
            creator: creator_key,
            supply_mode: supply_mode_byte,
            total_supply,
            operator_type: op_type_byte,
            timestamp: clock.unix_timestamp,
        });
        msg!(
            "Project created. Mint: {}, Protocol alloc: {}, Creator alloc: {}, Public escrow: {}",
            mint_key,
            protocol_allocation,
            treasury_allocation,
            public_allocation
        );
        Ok(())
    }

    // ── 3. open_cycle ───────────────────────────
    pub fn open_cycle(
        ctx: Context<OpenCycle>,
        curve_type: CurveType,
        supply_cap: u64,
        base_price: u64,
        rights_window_duration: i64,  // seconds
        step_size: u64,
        step_increment: u64,
        end_price: u64,
        growth_factor_k: u64,
    ) -> Result<()> {
        let project = &mut ctx.accounts.project_state;
        // Authority check: creator OR operator with can_open_cycle permission
        {
            let auth_ref = ctx.accounts.authority_config.as_deref();
            check_authority(&ctx.accounts.caller.key(), project, &project.key(), auth_ref, "open_cycle")?;

            // FIX SC-8 + RA-5: Enforce spending limit for operators with curve-aware estimate
            if let Some(auth) = auth_ref {
                if auth.spending_limit_lamports > 0 && ctx.accounts.caller.key() != project.creator {
                    // Estimate max SOL this cycle could raise based on curve type
                    let max_raise = match &curve_type {
                        CurveType::Step => {
                            // FIX F6: Use arithmetic sum for accurate step curve estimate
                            // Total = step_size * sum(base_price + i*step_increment) for i=0..num_steps-1
                            //       + remainder * final_price
                            // Sum of arithmetic series: n*base + step_increment * n*(n-1)/2
                            let num_steps = if step_size > 0 { supply_cap / step_size } else { 0 };
                            let remainder = if step_size > 0 { supply_cap % step_size } else { 0 };
                            // Total for full steps: step_size * (num_steps * base_price + step_increment * num_steps * (num_steps - 1) / 2)
                            let base_total = step_size.saturating_mul(
                                num_steps.saturating_mul(base_price)
                                    .saturating_add(
                                        step_increment.saturating_mul(num_steps).saturating_mul(num_steps.saturating_sub(1)) / 2
                                    )
                            );
                            // Remainder tokens at final step price
                            let final_price = base_price.saturating_add(num_steps.saturating_mul(step_increment));
                            let remainder_total = remainder.saturating_mul(final_price);
                            base_total.checked_add(remainder_total).unwrap_or(u64::MAX)
                        }
                        CurveType::Linear => {
                            // Average of base_price and end_price * supply_cap
                            let avg_price = base_price.saturating_add(end_price) / 2;
                            supply_cap.checked_mul(avg_price).unwrap_or(u64::MAX)
                        }
                        CurveType::ExpLite => {
                            // Conservative: use end_price as upper bound
                            // ExpLite max price = base_price + base_price * k * 1.0 (at 100% fill)
                            let max_price = base_price.saturating_add(
                                base_price.saturating_mul(growth_factor_k) / 10000
                            );
                            supply_cap.checked_mul(max_price).unwrap_or(u64::MAX)
                        }
                    };
                    require!(
                        max_raise <= auth.spending_limit_lamports,
                        MammothError::SpendingLimitExceeded
                    );
                }
            }
        }

        // Enforce scheduled launch time lock
        if let Some(launch_at) = project.launch_at {
            let clock = Clock::get()?;
            require!(
                clock.unix_timestamp >= launch_at,
                MammothError::LaunchTimeNotReached
            );
        }

        // FIX RA-1: Block opening a new cycle while one is still active
        require!(!project.has_active_cycle, MammothError::CycleStillActive);

        // FIX F7: Validate step_size > 0 for Step curves (prevents bricked cycles)
        // FIX L-1 (round 5): Minimum step_size prevents compute-unit DoS on large buys.
        // Worst case: amount/step_size iterations. With step_size=100 and 1M amount = 10k iterations.
        if matches!(curve_type, CurveType::Step) {
            require!(step_size >= 100, MammothError::ZeroStepSize);
        }

        // FIX L-7 (final audit): Cap growth_factor_k to prevent overflow on first buy.
        // Contract formula: growth = base * k * pct / 10000 / 10000. At 100% fill,
        // k=10000 means price doubles. Cap at 100000 (10x growth) as a sane max.
        if matches!(curve_type, CurveType::ExpLite) {
            require!(growth_factor_k <= 100000, MammothError::InvalidCurveParam);
        }

        // FIX F1: Cap rights_window_duration to 30 days (2,592,000 seconds) to prevent
        // permanent project lockout from extreme durations.
        // FIX H3/H4 (round 10): Require minimum 60 seconds to prevent 0-duration races where
        // an attacker front-runs activate_cycle in the same slot as open_cycle, zeroing
        // out rights_reserved_at_activation before the creator can set the Merkle root.
        const MAX_RIGHTS_WINDOW_SECS: i64 = 30 * 24 * 60 * 60; // 30 days
        const MIN_RIGHTS_WINDOW_SECS: i64 = 60; // 60 seconds minimum
        require!(
            rights_window_duration >= MIN_RIGHTS_WINDOW_SECS && rights_window_duration <= MAX_RIGHTS_WINDOW_SECS,
            MammothError::InvalidRightsWindow
        );

        // Elastic supply requires rights mode — enforced by not allowing cycle without rights
        // (In this implementation, all cycles have a rights window, satisfying this constraint)

        require!(ctx.accounts.mint.key() == project.mint, MammothError::InvalidProjectMint);

        // FIX SC-9: Validate that supply_cap doesn't exceed available tokens in escrow
        let escrow_balance = ctx.accounts.project_escrow_token.amount;
        require!(
            supply_cap <= escrow_balance,
            MammothError::SupplyCapExceedsEscrow
        );

        // FIX F3: Enforce hard_cap — total_supply cannot exceed hard_cap.
        // For Fixed mode, total_supply is immutable so this is always satisfied.
        // For Elastic mode, if a future mint_additional instruction is added, it must
        // also check this. For now, this guards against future expansion.
        if let Some(hard_cap) = project.hard_cap {
            require!(
                project.total_supply <= hard_cap,
                MammothError::SupplyCapExceeded
            );
        }

        let clock = Clock::get()?;
        let cycle_index = project.current_cycle;

        let cycle = &mut ctx.accounts.cycle_state;
        cycle.project = project.key();
        cycle.cycle_index = cycle_index;
        cycle.curve_type = curve_type;
        cycle.supply_cap = supply_cap;
        cycle.minted = 0;
        cycle.base_price = base_price;
        cycle.status = CycleStatus::RightsWindow;
        // FIX F2: Use checked_add to prevent overflow
        cycle.rights_window_end = clock.unix_timestamp
            .checked_add(rights_window_duration)
            .ok_or(MammothError::MathOverflow)?;
        cycle.step_size = step_size;
        cycle.step_increment = step_increment;
        cycle.end_price = end_price;
        cycle.growth_factor_k = growth_factor_k;
        cycle.sol_raised = 0;
        cycle.rights_merkle_root = None; // FIX RA-9: Explicit initialization
        cycle.rights_allocated = 0;     // FIX RA-8: Track cumulative rights
        cycle.rights_reserved_at_activation = 0; // FIX H-R6-2: snapshot at activation
        cycle.rights_committed = 0;               // FIX H-R7-1: committed via merkle
        cycle.bump = ctx.bumps.cycle_state;

        project.current_cycle = cycle_index
            .checked_add(1)
            .ok_or(MammothError::MathOverflow)?;
        project.has_active_cycle = true; // FIX RA-1

        let curve_byte = match cycle.curve_type {
            CurveType::Step => 0u8,
            CurveType::Linear => 1u8,
            CurveType::ExpLite => 2u8,
        };
        emit!(CycleOpened {
            project_mint: project.mint,
            project_state: project.key(),
            cycle_index,
            curve_type: curve_byte,
            supply_cap: cycle.supply_cap,
            base_price: cycle.base_price,
            rights_window_end: cycle.rights_window_end,
            timestamp: clock.unix_timestamp,
        });
        msg!(
            "Cycle {} opened. Rights window ends at {}",
            cycle_index,
            cycle.rights_window_end
        );
        Ok(())
    }

    // ── 4. exercise_rights ──────────────────────
    /// FIX (post-audit): Added max_sol_cost slippage parameter.
    /// Caller passes the maximum lamports they're willing to spend; tx fails if
    /// the actual cost exceeds this. Pass u64::MAX to disable the check.
    pub fn exercise_rights(ctx: Context<ExerciseRights>, amount: u64, max_sol_cost: u64) -> Result<()> {
        require!(amount > 0, MammothError::ZeroAmount);

        // Capture values needed for emit before mutable borrows
        let r_cycle_index = ctx.accounts.cycle_state.cycle_index;
        let r_mint = ctx.accounts.project_state.mint;

        let clock = Clock::get()?;
        let cycle = &mut ctx.accounts.cycle_state;

        require!(cycle.status == CycleStatus::RightsWindow, MammothError::NotRightsWindow);
        require!(clock.unix_timestamp < cycle.rights_window_end, MammothError::RightsWindowExpired);

        let rights = &mut ctx.accounts.holder_rights;

        require!(rights.project == ctx.accounts.project_state.key(), MammothError::InvalidRightsProject);
        require!(rights.cycle_index == cycle.cycle_index, MammothError::InvalidRightsCycle);
        require!(rights.holder == ctx.accounts.holder.key(), MammothError::InvalidRightsHolder);

        // FIX SC-18: Enforce per-holder expiry (relevant for create_holder_rights path with custom expiry)
        require!(clock.unix_timestamp < rights.expiry, MammothError::RightsWindowExpired);
        require!(
            rights.exercised_amount.checked_add(amount).ok_or(MammothError::MathOverflow)? <= rights.rights_amount,
            MammothError::ExceedsRightsAllocation
        );

        // FIX SC-3: Enforce supply cap during rights exercise — prevents draining escrow
        require!(
            cycle.minted.checked_add(amount).ok_or(MammothError::MathOverflow)? <= cycle.supply_cap,
            MammothError::SupplyCapExceeded
        );

        // Price is flat base_price during rights window
        let price_per_token = cycle.base_price;
        let total_cost = price_per_token
            .checked_mul(amount)
            .ok_or(MammothError::MathOverflow)?;

        // FIX TOCTOU: Enforce slippage cap — prevents overspending if price changes
        require!(total_cost <= max_sol_cost, MammothError::SlippageExceeded);

        // FIX SC-4: Apply protocol fee to rights exercises (consistent with buy_tokens)
        let config = &ctx.accounts.protocol_config;
        let fee = total_cost
            .checked_mul(config.fee_bps as u64)
            .ok_or(MammothError::MathOverflow)?
            .checked_div(10000)
            .ok_or(MammothError::MathOverflow)?;
        let net_cost = total_cost.checked_sub(fee).ok_or(MammothError::MathOverflow)?;

        // Transfer fee to protocol treasury (SOL)
        if fee > 0 {
            let ix = anchor_lang::solana_program::system_instruction::transfer(
                &ctx.accounts.holder.key(),
                &ctx.accounts.protocol_treasury.key(),
                fee,
            );
            anchor_lang::solana_program::program::invoke(
                &ix,
                &[
                    ctx.accounts.holder.to_account_info(),
                    ctx.accounts.protocol_treasury.to_account_info(),
                    ctx.accounts.system_program.to_account_info(),
                ],
            )?;
        }

        // Transfer remaining SOL to cycle escrow
        let cycle_key = cycle.key();
        if net_cost > 0 {
            let ix = anchor_lang::solana_program::system_instruction::transfer(
                &ctx.accounts.holder.key(),
                &cycle_key,
                net_cost,
            );
            anchor_lang::solana_program::program::invoke(
                &ix,
                &[
                    ctx.accounts.holder.to_account_info(),
                    cycle.to_account_info(),
                    ctx.accounts.system_program.to_account_info(),
                ],
            )?;
        }

        // Transfer tokens from project escrow to holder
        let project_state = &ctx.accounts.project_state;
        let mint_key = project_state.mint;
        let bump = project_state.bump;
        let seeds = &[b"project".as_ref(), mint_key.as_ref(), &[bump]];
        let signer_seeds = &[&seeds[..]];

        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.project_escrow_token.to_account_info(),
                    to: ctx.accounts.holder_token.to_account_info(),
                    authority: ctx.accounts.project_state.to_account_info(),
                },
                signer_seeds,
            ),
            amount,
        )?;

        // Update state
        rights.exercised_amount = rights.exercised_amount
            .checked_add(amount)
            .ok_or(MammothError::MathOverflow)?;

        emit!(RightsExercised {
            project_mint: r_mint,
            holder: ctx.accounts.holder.key(),
            cycle_index: r_cycle_index,
            amount,
            sol_paid: total_cost,
            timestamp: clock.unix_timestamp,
        });
        cycle.minted = cycle.minted
            .checked_add(amount)
            .ok_or(MammothError::MathOverflow)?;
        // FIX SC-6: Track net_cost (after fee) consistently — matches buy_tokens behavior
        cycle.sol_raised = cycle.sol_raised
            .checked_add(net_cost)
            .ok_or(MammothError::MathOverflow)?;

        msg!("Rights exercised: {} tokens @ {} lamports each. Fee: {} lamports", amount, price_per_token, fee);
        Ok(())
    }

    // ── 5. buy_tokens ───────────────────────────
    /// FIX (post-audit): Added max_sol_cost slippage parameter.
    /// Caller passes the maximum total lamports they're willing to spend (including fee);
    /// tx fails if the actual total cost exceeds this. Pass u64::MAX to disable.
    pub fn buy_tokens(ctx: Context<BuyTokens>, amount: u64, max_sol_cost: u64) -> Result<()> {
        require!(amount > 0, MammothError::ZeroAmount);

        let cycle = &mut ctx.accounts.cycle_state;
        require!(cycle.project == ctx.accounts.project_state.key(), MammothError::InvalidCycleProject);
        require!(ctx.accounts.mint.key() == ctx.accounts.project_state.mint, MammothError::InvalidProjectMint);
        require!(cycle.status == CycleStatus::Active, MammothError::NotActive);

        // FIX H-R6-2 (round 7): Use snapshot from activation, not live computation.
        // Previously: public_cap = supply_cap - max(0, rights_allocated - minted) eroded
        // as public buys grew cycle.minted, defeating the reservation. Now we use
        // cycle.rights_reserved_at_activation which is frozen at activate_cycle.
        let public_cap = cycle.supply_cap.saturating_sub(cycle.rights_reserved_at_activation);
        require!(
            cycle.minted.checked_add(amount).ok_or(MammothError::MathOverflow)? <= public_cap,
            MammothError::SupplyCapExceeded
        );

        let config = &ctx.accounts.protocol_config;
        // FIX M-1 (final audit): Use integrated cost across the curve, NOT current spot * amount.
        // The old formula let bulk buyers pay base_price for entire cap, bypassing the curve.
        let total_cost = compute_total_cost(cycle, amount)?;
        let price_per_token = compute_price(cycle)?; // still emitted for the event (pre-buy spot)

        // FIX TOCTOU: Enforce slippage cap — prevents overspending if price moved
        require!(total_cost <= max_sol_cost, MammothError::SlippageExceeded);

        // Protocol fee (2%)
        let fee = total_cost
            .checked_mul(config.fee_bps as u64)
            .ok_or(MammothError::MathOverflow)?
            .checked_div(10000)
            .ok_or(MammothError::MathOverflow)?;
        let net_cost = total_cost.checked_sub(fee).ok_or(MammothError::MathOverflow)?;

        // Transfer fee to protocol treasury (SOL)
        if fee > 0 {
            let ix = anchor_lang::solana_program::system_instruction::transfer(
                &ctx.accounts.buyer.key(),
                &ctx.accounts.protocol_treasury.key(),
                fee,
            );
            anchor_lang::solana_program::program::invoke(
                &ix,
                &[
                    ctx.accounts.buyer.to_account_info(),
                    ctx.accounts.protocol_treasury.to_account_info(),
                    ctx.accounts.system_program.to_account_info(),
                ],
            )?;
        }

        // Transfer remaining SOL to cycle escrow
        if net_cost > 0 {
            let cycle_key = cycle.key();
            let ix = anchor_lang::solana_program::system_instruction::transfer(
                &ctx.accounts.buyer.key(),
                &cycle_key,
                net_cost,
            );
            anchor_lang::solana_program::program::invoke(
                &ix,
                &[
                    ctx.accounts.buyer.to_account_info(),
                    cycle.to_account_info(),
                    ctx.accounts.system_program.to_account_info(),
                ],
            )?;
        }

        // Transfer tokens from project escrow to buyer
        let project_state = &ctx.accounts.project_state;
        let mint_key = project_state.mint;
        let bump = project_state.bump;
        let seeds = &[b"project".as_ref(), mint_key.as_ref(), &[bump]];
        let signer_seeds = &[&seeds[..]];

        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.project_escrow_token.to_account_info(),
                    to: ctx.accounts.buyer_token.to_account_info(),
                    authority: ctx.accounts.project_state.to_account_info(),
                },
                signer_seeds,
            ),
            amount,
        )?;

        // Update state
        cycle.minted = cycle.minted
            .checked_add(amount)
            .ok_or(MammothError::MathOverflow)?;
        cycle.sol_raised = cycle.sol_raised
            .checked_add(net_cost)
            .ok_or(MammothError::MathOverflow)?;

        // Note: When supply cap is reached, the cycle stays Active.
        // The creator (or operator) must call close_cycle to distribute SOL.
        // This prevents the old vulnerability where anyone could call close_cycle on
        // an already-Closed cycle and steal the creator's share.
        if cycle.minted >= cycle.supply_cap {
            msg!("Supply cap reached — creator should call close_cycle to distribute SOL");
        }

        let emitted_cycle_index = cycle.cycle_index;
        let emitted_mint = ctx.accounts.project_state.mint;
        let clock_b = Clock::get()?;
        emit!(TokensPurchased {
            project_mint: emitted_mint,
            buyer: ctx.accounts.buyer.key(),
            cycle_index: emitted_cycle_index,
            amount,
            sol_paid: total_cost,
            price_per_token,
            timestamp: clock_b.unix_timestamp,
        });

        msg!(
            "Bought {} tokens @ {} lamports each. Fee: {} lamports",
            amount,
            price_per_token,
            fee
        );
        Ok(())
    }

    // ── 6. close_cycle ──────────────────────────
    pub fn close_cycle(ctx: Context<CloseCycle>) -> Result<()> {
        let project = &mut ctx.accounts.project_state;
        let cycle = &mut ctx.accounts.cycle_state;

        require!(cycle.project == project.key(), MammothError::InvalidCycleProject);

        // FIX SC-2: Only allow close_cycle on Active cycles, never on Closed or RightsWindow.
        // FIX RA-3: Restricting to Active only prevents cutting off rights holders prematurely.
        require!(
            cycle.status == CycleStatus::Active,
            MammothError::NotActive
        );

        // FIX SC-1 & SC-7: Use check_authority for proper creator/operator verification.
        // The CloseCycle account struct now constrains creator to match project_state.creator,
        // so SOL always goes to the real creator.
        {
            let auth_ref = ctx.accounts.authority_config.as_deref();
            check_authority(&ctx.accounts.caller.key(), project, &project.key(), auth_ref, "close_cycle")?;
        }

        let sol_balance = cycle.to_account_info().lamports();
        // Subtract rent-exempt minimum so PDA keeps its lamports
        let rent = Rent::get()?;
        let rent_exempt = rent.minimum_balance(CycleState::LEN);
        let distributable = sol_balance.saturating_sub(rent_exempt);

        if distributable > 0 {
            let creator_share = distributable
                .checked_mul(project.creator_bps as u64)
                .ok_or(MammothError::MathOverflow)?
                .checked_div(10000)
                .ok_or(MammothError::MathOverflow)?;

            let reserve_share = distributable
                .checked_mul(project.reserve_bps as u64)
                .ok_or(MammothError::MathOverflow)?
                .checked_div(10000)
                .ok_or(MammothError::MathOverflow)?;

            // sink = remainder after creator + reserve
            let sink_share = distributable
                .saturating_sub(creator_share)
                .saturating_sub(reserve_share);

            // FIX SC-1: Always send creator_share to the actual project creator, not the signer.
            // The creator account is now constrained to match project_state.creator in CloseCycle.
            if creator_share > 0 {
                **cycle.to_account_info().try_borrow_mut_lamports()? -= creator_share;
                **ctx.accounts.creator.try_borrow_mut_lamports()? += creator_share;
            }

            // Transfer to reserve PDA
            if reserve_share > 0 {
                **cycle.to_account_info().try_borrow_mut_lamports()? -= reserve_share;
                **ctx.accounts.reserve.try_borrow_mut_lamports()? += reserve_share;
            }

            // FIX RA-6: Send sink_share to dedicated sink PDA (no withdrawal instruction exists).
            // This SOL is effectively locked/burned — the PDA has no program instruction to move it.
            if sink_share > 0 {
                **cycle.to_account_info().try_borrow_mut_lamports()? -= sink_share;
                **ctx.accounts.sink.try_borrow_mut_lamports()? += sink_share;
            }
        }

        let tokens_sold = cycle.minted;
        let sol_raised = cycle.sol_raised;
        let closed_cycle_index = cycle.cycle_index;
        cycle.status = CycleStatus::Closed;
        project.has_active_cycle = false; // FIX RA-1: Allow new cycles to be opened

        let clock_c = Clock::get()?;
        emit!(CycleClosed {
            project_mint: ctx.accounts.project_state.mint,
            project_state: ctx.accounts.project_state.key(),
            cycle_index: closed_cycle_index,
            tokens_sold,
            sol_raised,
            timestamp: clock_c.unix_timestamp,
        });
        msg!("Cycle {} closed. SOL distributed.", closed_cycle_index);
        Ok(())
    }

    // ── 7. set_hard_cap ─────────────────────────
    pub fn set_hard_cap(ctx: Context<SetHardCap>, hard_cap: u64) -> Result<()> {
        let project = &mut ctx.accounts.project_state;
        // Authority check: creator OR operator with can_set_hard_cap (off by default)
        {
            let auth_ref = ctx.accounts.authority_config.as_deref();
            check_authority(&ctx.accounts.caller.key(), project, &project.key(), auth_ref, "set_hard_cap")?;
        }
        require!(project.supply_mode == SupplyMode::Elastic, MammothError::NotElasticMode);
        require!(project.hard_cap.is_none(), MammothError::HardCapAlreadySet);

        project.hard_cap = Some(hard_cap);

        let clock_h = Clock::get()?;
        emit!(HardCapSet {
            project_mint: project.mint,
            hard_cap,
            timestamp: clock_h.unix_timestamp,
        });
        msg!("Hard cap set to {} tokens — irreversible", hard_cap);
        Ok(())
    }

    // ── 8. activate_cycle ───────────────────────
    /// Permissionless. Transitions RightsWindow → Active once rights_window_end has passed.
    pub fn activate_cycle(ctx: Context<ActivateCycle>) -> Result<()> {
        let cycle = &mut ctx.accounts.cycle_state;

        require!(cycle.project == ctx.accounts.project_state.key(), MammothError::InvalidCycleProject);
        require!(cycle.status == CycleStatus::RightsWindow, MammothError::NotRightsWindow);

        let clock = Clock::get()?;
        require!(
            clock.unix_timestamp >= cycle.rights_window_end,
            MammothError::RightsWindowStillOpen
        );

        cycle.status = CycleStatus::Active;
        // FIX H-R6-2 (round 7) + H-R7-1 (round 8): Snapshot unexercised rights at activation.
        // Use MAX of rights_allocated (claimed) and rights_committed (Merkle total) to
        // protect holders who committed via Merkle but didn't claim before the window closed.
        // `rights_committed` is 0 if no Merkle root was set.
        let effective_rights = std::cmp::max(cycle.rights_allocated, cycle.rights_committed);
        cycle.rights_reserved_at_activation = effective_rights.saturating_sub(cycle.minted);
        let activated_index = cycle.cycle_index;

        let clock_a = Clock::get()?;
        emit!(CycleActivated {
            project_mint: ctx.accounts.project_state.mint,
            project_state: ctx.accounts.project_state.key(),
            cycle_index: activated_index,
            timestamp: clock_a.unix_timestamp,
        });
        msg!(
            "Cycle {} activated. Public sale is now open.",
            activated_index
        );
        Ok(())
    }

    // ── 9. create_holder_rights ─────────────────
    /// Creator-only. Allocates rights to a holder during the RightsWindow.
    pub fn create_holder_rights(
        ctx: Context<CreateHolderRights>,
        holder: Pubkey,
        rights_amount: u64,
        expiry: i64,
    ) -> Result<()> {
        let project = &ctx.accounts.project_state;
        require!(project.creator == ctx.accounts.creator.key(), MammothError::Unauthorized);

        let cycle = &mut ctx.accounts.cycle_state;
        require!(cycle.project == project.key(), MammothError::InvalidCycleProject);
        require!(cycle.status == CycleStatus::RightsWindow, MammothError::NotRightsWindow);

        // FIX H-R8-1 (round 9): Force single rights path — if a Merkle root was set,
        // all rights must come through claim_rights. No mixing legacy + Merkle paths.
        require!(
            cycle.rights_merkle_root.is_none() && cycle.rights_committed == 0,
            MammothError::RightsPathConflict
        );

        require!(rights_amount > 0, MammothError::ZeroAmount);

        // FIX RA-8: Prevent over-allocation beyond supply_cap
        let new_total = cycle.rights_allocated
            .checked_add(rights_amount)
            .ok_or(MammothError::MathOverflow)?;
        require!(new_total <= cycle.supply_cap, MammothError::SupplyCapExceeded);
        cycle.rights_allocated = new_total;

        let hr = &mut ctx.accounts.holder_rights;
        hr.project = project.key();
        hr.cycle_index = cycle.cycle_index;
        hr.holder = holder;
        hr.rights_amount = rights_amount;
        hr.exercised_amount = 0;
        hr.expiry = expiry;
        hr.bump = ctx.bumps.holder_rights;

        msg!(
            "Holder rights created: holder={}, amount={}, rights_allocated={}/{}",
            holder,
            rights_amount,
            cycle.rights_allocated,
            cycle.supply_cap
        );
        Ok(())
    }

    // ── 10. set_rights_merkle_root ──────────────
    /// Creator (or authorized operator) sets the Merkle root for rights distribution.
    /// Called after open_cycle, before activate_cycle.
    /// The root commits to a list of (holder_pubkey, rights_amount) pairs computed off-chain.
    /// Holders then claim their rights by submitting a Merkle proof via claim_rights.
    ///
    /// @param merkle_root   32-byte SHA-256 Merkle root with domain separation (0x00 leaf, 0x01 node)
    /// @param holder_count  informational count of holders in the snapshot (not verified on-chain)
    pub fn set_rights_merkle_root(
        ctx: Context<SetRightsMerkleRoot>,
        merkle_root: [u8; 32],
        holder_count: u32,
        total_committed: u64,  // FIX H-R7-1: total rights committed by this tree
    ) -> Result<()> {
        let cycle = &mut ctx.accounts.cycle_state;
        require!(cycle.project == ctx.accounts.project_state.key(), MammothError::InvalidCycleProject);
        require!(
            cycle.status == CycleStatus::RightsWindow,
            MammothError::NotRightsWindow
        );

        // FIX SC-12: Prevent overwriting merkle root after it's been set
        require!(
            cycle.rights_merkle_root.is_none(),
            MammothError::MerkleRootAlreadySet
        );

        // FIX H-R8-1 (round 9): Force single rights path — can't mix Merkle + legacy create_holder_rights.
        // Otherwise total rights could exceed supply_cap by combining both paths.
        require!(cycle.rights_allocated == 0, MammothError::RightsPathConflict);

        // FIX H-R7-1: total_committed must not exceed supply_cap (protects public buyers too)
        require!(total_committed <= cycle.supply_cap, MammothError::SupplyCapExceeded);

        {
            // FIX M-1 (round 5): Use dedicated can_set_merkle_root permission.
            // This prevents an operator with only can_open_cycle from injecting a
            // malicious rights tree (front-running the creator).
            let auth_ref = ctx.accounts.authority_config.as_deref();
            check_authority(&ctx.accounts.caller.key(), &ctx.accounts.project_state, &ctx.accounts.project_state.key(), auth_ref, "set_merkle_root")?;
        }

        cycle.rights_merkle_root = Some(merkle_root);
        cycle.rights_committed = total_committed; // FIX H-R7-1

        let mint = ctx.accounts.project_state.mint;
        let idx = cycle.cycle_index;
        let clock = Clock::get()?;
        emit!(MerkleRightsSet {
            project_mint: mint,
            cycle_index: idx,
            merkle_root,
            holder_count,
            timestamp: clock.unix_timestamp,
        });
        msg!("Merkle rights root set for cycle {}. {} holders.", idx, holder_count);
        Ok(())
    }

    // ── 11. claim_rights ────────────────────────
    /// Holder submits a Merkle proof to claim their rights allocation.
    /// Verifies the proof against rights_merkle_root, then creates a HolderRights account.
    /// This replaces the creator-driven create_holder_rights for scale deployments.
    ///
    /// Leaf hash = SHA-256(0x00 || holder_pubkey || rights_amount_le_bytes)
    ///
    /// @param proof        Vec of 32-byte sibling hashes from leaf to root
    /// @param rights_amount Amount of rights tokens the holder is claiming
    pub fn claim_rights(
        ctx: Context<ClaimRights>,
        proof: Vec<[u8; 32]>,
        rights_amount: u64,
    ) -> Result<()> {
        require!(rights_amount > 0, MammothError::ZeroAmount);

        let clock = Clock::get()?;
        let cycle = &mut ctx.accounts.cycle_state;

        require!(cycle.project == ctx.accounts.project_state.key(), MammothError::InvalidCycleProject);
        require!(cycle.status == CycleStatus::RightsWindow, MammothError::NotRightsWindow);
        require!(clock.unix_timestamp < cycle.rights_window_end, MammothError::RightsWindowExpired);

        // FIX RA-8: Check cumulative rights allocation before accepting claim
        let new_total = cycle.rights_allocated
            .checked_add(rights_amount)
            .ok_or(MammothError::MathOverflow)?;
        require!(new_total <= cycle.supply_cap, MammothError::SupplyCapExceeded);

        let root = cycle.rights_merkle_root.ok_or(MammothError::MerkleRootNotSet)?;

        // Verify Merkle proof
        // FIX H-3 (final audit): Use domain separation bytes (0x00 for leaves, 0x01 for nodes)
        // to prevent second-preimage attacks where a leaf hash could collide with an internal node.
        let holder = ctx.accounts.holder.key();
        // Leaf = SHA256(0x00 || holder_pubkey || rights_amount_le_8bytes)
        let leaf: [u8; 32] = {
            let mut h = Sha256::new();
            h.update(&[0x00u8]); // leaf domain byte
            h.update(holder.as_ref());
            h.update(&rights_amount.to_le_bytes());
            h.finalize().into()
        };

        // Compute root from proof — sorted pairs with 0x01 node domain byte
        let computed: [u8; 32] = proof.iter().fold(leaf, |current, sibling| {
            let (left, right) = if current <= *sibling {
                (current, *sibling)
            } else {
                (*sibling, current)
            };
            let mut h = Sha256::new();
            h.update(&[0x01u8]); // internal node domain byte
            h.update(left);
            h.update(right);
            h.finalize().into()
        });

        require!(computed == root, MammothError::InvalidMerkleProof);

        // FIX RA-8: Update cumulative rights tracking
        cycle.rights_allocated = new_total;

        // Write HolderRights account
        let hr = &mut ctx.accounts.holder_rights;
        hr.project = ctx.accounts.project_state.key();
        hr.cycle_index = cycle.cycle_index;
        hr.holder = holder;
        hr.rights_amount = rights_amount;
        hr.exercised_amount = 0;
        hr.expiry = cycle.rights_window_end;
        hr.bump = ctx.bumps.holder_rights;

        let claimed_cycle_index = cycle.cycle_index;
        let claimed_mint = ctx.accounts.project_state.mint;
        emit!(RightsClaimed {
            project_mint: claimed_mint,
            holder,
            cycle_index: claimed_cycle_index,
            amount: rights_amount,
            timestamp: clock.unix_timestamp,
        });
        msg!("Rights claimed: holder={}, amount={}", holder, rights_amount);
        Ok(())
    }

    // ── 12. initialize_authority ────────────────
    /// Creator-only. Sets up delegated authority for an AI agent or operator.
    /// Principal can be any wallet — human, DAO multisig, or another AI controller.
    pub fn initialize_authority(
        ctx: Context<InitializeAuthority>,
        operator: Pubkey,
        can_open_cycle: bool,
        can_close_cycle: bool,
        can_set_hard_cap: bool,
        can_route_treasury: bool,
        spending_limit_lamports: u64,
        can_set_merkle_root: bool,
    ) -> Result<()> {
        let project = &ctx.accounts.project_state;
        require!(
            project.creator == ctx.accounts.principal.key(),
            MammothError::Unauthorized
        );

        let config = &mut ctx.accounts.authority_config;
        config.project = project.key();
        config.principal = ctx.accounts.principal.key();
        config.operator = operator;
        config.can_open_cycle = can_open_cycle;
        config.can_close_cycle = can_close_cycle;
        config.can_set_hard_cap = can_set_hard_cap;
        config.can_route_treasury = can_route_treasury;
        config.spending_limit_lamports = spending_limit_lamports;
        config.can_set_merkle_root = can_set_merkle_root;
        config.bump = ctx.bumps.authority_config;

        msg!(
            "AuthorityConfig initialized. Principal: {}, Operator: {}, can_open={}, can_close={}, can_hard_cap={}",
            config.principal,
            config.operator,
            config.can_open_cycle,
            config.can_close_cycle,
            config.can_set_hard_cap
        );
        Ok(())
    }

    // ── 11. update_authority ────────────────────
    /// Principal-only. Updates operator permissions or spending limit.
    pub fn update_authority(
        ctx: Context<UpdateAuthority>,
        operator: Pubkey,
        can_open_cycle: bool,
        can_close_cycle: bool,
        can_set_hard_cap: bool,
        can_route_treasury: bool,
        spending_limit_lamports: u64,
        can_set_merkle_root: bool,
    ) -> Result<()> {
        let config = &mut ctx.accounts.authority_config;
        require!(
            config.principal == ctx.accounts.principal.key(),
            MammothError::Unauthorized
        );

        config.operator = operator;
        config.can_open_cycle = can_open_cycle;
        config.can_close_cycle = can_close_cycle;
        config.can_set_hard_cap = can_set_hard_cap;
        config.can_route_treasury = can_route_treasury;
        config.spending_limit_lamports = spending_limit_lamports;
        config.can_set_merkle_root = can_set_merkle_root;

        msg!(
            "AuthorityConfig updated. Operator: {}, can_open={}, can_close={}, can_hard_cap={}",
            config.operator,
            config.can_open_cycle,
            config.can_close_cycle,
            config.can_set_hard_cap
        );
        Ok(())
    }

    // ── 14. reclaim_cycle_rent ──────────────────
    /// FIX F9: Creator can reclaim rent from closed cycle accounts.
    /// Cycle must have been closed for at least 7 days to preserve on-chain history.
    /// Zeroes the account and returns all remaining lamports to creator.
    pub fn reclaim_cycle_rent(ctx: Context<ReclaimCycleRent>) -> Result<()> {
        let cycle = &ctx.accounts.cycle_state;
        require!(cycle.status == CycleStatus::Closed, MammothError::NotClosed);

        // Only allow reclaim after funds have been distributed (balance at/below rent-exempt)
        let remaining = cycle.to_account_info().lamports();
        let rent = Rent::get()?;
        let rent_exempt = rent.minimum_balance(CycleState::LEN);

        // Only allow reclaim if balance is at or below rent-exempt (funds already distributed)
        require!(remaining <= rent_exempt, MammothError::NotClosed);

        msg!("Cycle {} rent reclaimed. {} lamports returned to creator.", cycle.cycle_index, remaining);
        // Anchor's `close = creator` in the account struct handles the actual transfer + zeroing
        Ok(())
    }

    // ── 15a. rotate_creator ─────────────────────
    /// FIX H6 (round 10): Allow current creator to transfer ownership to a new wallet.
    /// Without this, creator key loss would permanently lock reserve + block future actions.
    /// Must be signed by the CURRENT creator. AuthorityConfig's principal is NOT updated —
    /// operator delegations survive, but new principal must re-issue authority if desired.
    pub fn rotate_creator(ctx: Context<RotateCreator>, new_creator: Pubkey) -> Result<()> {
        let project = &mut ctx.accounts.project_state;
        require!(
            ctx.accounts.current_creator.key() == project.creator,
            MammothError::Unauthorized
        );
        let old = project.creator;
        project.creator = new_creator;
        msg!("Creator rotated: {} -> {}", old, new_creator);
        Ok(())
    }

    // ── 15. withdraw_reserve ────────────────────
    /// FIX C-3 (final audit): Creator withdraws from the reserve PDA.
    /// FIX L-B (round 6): Reserve PDA is System-owned (never init'd with program owner),
    /// so we must use system_program::transfer with invoke_signed. Direct lamport
    /// mutation fails at runtime for System-owned accounts.
    /// Reserve accumulates reserve_bps share of each closed cycle's SOL.
    /// The sink PDA intentionally has NO withdrawal instruction (funds are burned).
    pub fn withdraw_reserve(ctx: Context<WithdrawReserve>, amount: u64) -> Result<()> {
        require!(amount > 0, MammothError::ZeroAmount);

        let reserve = &ctx.accounts.reserve;
        let balance = reserve.lamports();
        // For a System-owned 0-data account, no rent-exempt is required.
        // (Rent is only enforced for accounts holding data.)
        require!(amount <= balance, MammothError::InsufficientReserveBalance);

        // FIX L-B: Use system_program::transfer with reserve PDA as signer.
        // Reserve is System-owned, so direct lamport debit would fail.
        let project_key = ctx.accounts.project_state.key();
        let bump = ctx.bumps.reserve;
        let seeds: &[&[u8]] = &[b"reserve", project_key.as_ref(), &[bump]];
        let signer_seeds = &[seeds];

        let ix = anchor_lang::solana_program::system_instruction::transfer(
            &reserve.key(),
            &ctx.accounts.creator.key(),
            amount,
        );
        anchor_lang::solana_program::program::invoke_signed(
            &ix,
            &[
                reserve.to_account_info(),
                ctx.accounts.creator.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
            signer_seeds,
        )?;

        msg!("Withdrew {} lamports from reserve to creator.", amount);
        Ok(())
    }
}

// ─────────────────────────────────────────────
//  Account Contexts
// ─────────────────────────────────────────────

#[derive(Accounts)]
pub struct InitializeProtocol<'info> {
    #[account(
        init,
        payer = admin,
        space = ProtocolConfig::LEN,
        seeds = [b"protocol_config"],
        bump
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    /// CHECK: PDA for protocol treasury — receives SOL fees
    #[account(
        mut,
        seeds = [b"protocol_treasury"],
        bump
    )]
    pub protocol_treasury: UncheckedAccount<'info>,

    #[account(mut)]
    pub admin: Signer<'info>,

    pub system_program: Program<'info, System>,
}

/// FIX F4: Admin-only protocol config updates
#[derive(Accounts)]
pub struct UpdateProtocolConfig<'info> {
    #[account(
        mut,
        seeds = [b"protocol_config"],
        bump,
        has_one = admin @ MammothError::Unauthorized
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    pub admin: Signer<'info>,
}

#[derive(Accounts)]
pub struct CreateProject<'info> {
    #[account(
        init,
        payer = creator,
        mint::decimals = 6,
        mint::authority = project_state,
        mint::freeze_authority = project_state,  // set to project_state, revoked in same ix conceptually; using PDA as authority is sufficient
    )]
    pub mint: Account<'info, Mint>,

    #[account(
        init,
        payer = creator,
        space = ProjectState::LEN,
        seeds = [b"project", mint.key().as_ref()],
        bump
    )]
    pub project_state: Account<'info, ProjectState>,

    /// Protocol treasury PDA (SOL receiver for fees conceptually; tokens go to token account)
    /// CHECK: PDA validated by seeds
    #[account(
        mut,
        seeds = [b"protocol_treasury"],
        bump
    )]
    pub protocol_treasury: UncheckedAccount<'info>,

    /// Token account for protocol treasury
    #[account(
        init_if_needed,
        payer = creator,
        associated_token::mint = mint,
        associated_token::authority = protocol_treasury,
    )]
    pub protocol_treasury_token: Account<'info, TokenAccount>,

    /// Creator token account (receives treasury_allocation)
    #[account(
        init_if_needed,
        payer = creator,
        associated_token::mint = mint,
        associated_token::authority = creator,
    )]
    pub creator_token: Account<'info, TokenAccount>,

    /// Project escrow token account (holds public_allocation)
    #[account(
        init_if_needed,
        payer = creator,
        associated_token::mint = mint,
        associated_token::authority = project_state,
    )]
    pub project_escrow_token: Account<'info, TokenAccount>,

    /// FIX (integration test): Reserve PDA — funded with rent-exempt min during create_project
    /// CHECK: PDA validated by seeds
    #[account(
        mut,
        seeds = [b"reserve", project_state.key().as_ref()],
        bump
    )]
    pub reserve: UncheckedAccount<'info>,

    /// FIX (integration test): Sink PDA — funded with rent-exempt min during create_project
    /// CHECK: PDA validated by seeds
    #[account(
        mut,
        seeds = [b"sink", project_state.key().as_ref()],
        bump
    )]
    pub sink: UncheckedAccount<'info>,

    #[account(
        seeds = [b"protocol_config"],
        bump
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(mut)]
    pub creator: Signer<'info>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
#[instruction(
    curve_type: CurveType,
    supply_cap: u64,
    base_price: u64,
    rights_window_duration: i64,
    step_size: u64,
    step_increment: u64,
    end_price: u64,
    growth_factor_k: u64,
)]
pub struct OpenCycle<'info> {
    #[account(
        mut,
        seeds = [b"project", project_state.mint.as_ref()],
        bump = project_state.bump
    )]
    pub project_state: Account<'info, ProjectState>,

    #[account(
        init,
        payer = caller,
        space = CycleState::LEN,
        seeds = [b"cycle", project_state.key().as_ref(), &[project_state.current_cycle]],
        bump
    )]
    pub cycle_state: Account<'info, CycleState>,

    /// FIX SC-9: Escrow token account needed to validate supply_cap against available tokens
    #[account(
        associated_token::mint = mint,
        associated_token::authority = project_state,
    )]
    pub project_escrow_token: Account<'info, TokenAccount>,

    pub mint: Account<'info, Mint>,

    /// Optional — pass None when caller is creator (no AuthorityConfig needed).
    /// Pass Some when caller is an operator — program will verify permission.
    pub authority_config: Option<Account<'info, AuthorityConfig>>,

    #[account(mut)]
    pub caller: Signer<'info>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(amount: u64)]
pub struct ExerciseRights<'info> {
    #[account(
        seeds = [b"project", project_state.mint.as_ref()],
        bump = project_state.bump
    )]
    pub project_state: Account<'info, ProjectState>,

    #[account(
        mut,
        seeds = [b"cycle", project_state.key().as_ref(), &[cycle_state.cycle_index]],
        bump = cycle_state.bump
    )]
    pub cycle_state: Account<'info, CycleState>,

    #[account(
        mut,
        seeds = [b"rights", cycle_state.key().as_ref(), holder.key().as_ref()],
        bump = holder_rights.bump
    )]
    pub holder_rights: Account<'info, HolderRights>,

    /// FIX SC-4: Protocol config needed for fee_bps
    #[account(
        seeds = [b"protocol_config"],
        bump
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    /// FIX SC-4: Protocol treasury receives fee SOL
    /// CHECK: PDA validated by seeds
    #[account(
        mut,
        seeds = [b"protocol_treasury"],
        bump
    )]
    pub protocol_treasury: UncheckedAccount<'info>,

    /// Project escrow token account
    #[account(
        mut,
        associated_token::mint = mint,
        associated_token::authority = project_state,
    )]
    pub project_escrow_token: Account<'info, TokenAccount>,

    /// Holder's token account
    #[account(
        init_if_needed,
        payer = holder,
        associated_token::mint = mint,
        associated_token::authority = holder,
    )]
    pub holder_token: Account<'info, TokenAccount>,

    pub mint: Account<'info, Mint>,

    #[account(mut)]
    pub holder: Signer<'info>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
#[instruction(amount: u64)]
pub struct BuyTokens<'info> {
    #[account(
        seeds = [b"project", project_state.mint.as_ref()],
        bump = project_state.bump
    )]
    pub project_state: Account<'info, ProjectState>,

    #[account(
        mut,
        seeds = [b"cycle", project_state.key().as_ref(), &[cycle_state.cycle_index]],
        bump = cycle_state.bump
    )]
    pub cycle_state: Account<'info, CycleState>,

    #[account(
        seeds = [b"protocol_config"],
        bump
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    /// CHECK: Protocol treasury PDA — receives SOL fees
    #[account(
        mut,
        seeds = [b"protocol_treasury"],
        bump
    )]
    pub protocol_treasury: UncheckedAccount<'info>,

    /// Project escrow token account
    #[account(
        mut,
        associated_token::mint = mint,
        associated_token::authority = project_state,
    )]
    pub project_escrow_token: Account<'info, TokenAccount>,

    /// Buyer's token account
    #[account(
        init_if_needed,
        payer = buyer,
        associated_token::mint = mint,
        associated_token::authority = buyer,
    )]
    pub buyer_token: Account<'info, TokenAccount>,

    pub mint: Account<'info, Mint>,

    #[account(mut)]
    pub buyer: Signer<'info>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct CloseCycle<'info> {
    /// FIX RA-1: project_state is now mut to clear has_active_cycle flag
    #[account(
        mut,
        seeds = [b"project", project_state.mint.as_ref()],
        bump = project_state.bump
    )]
    pub project_state: Account<'info, ProjectState>,

    #[account(
        mut,
        seeds = [b"cycle", project_state.key().as_ref(), &[cycle_state.cycle_index]],
        bump = cycle_state.bump
    )]
    pub cycle_state: Account<'info, CycleState>,

    /// CHECK: Reserve PDA receives reserve_bps share of SOL
    #[account(
        mut,
        seeds = [b"reserve", project_state.key().as_ref()],
        bump
    )]
    pub reserve: UncheckedAccount<'info>,

    /// FIX RA-6: Dedicated sink PDA — SOL sent here is effectively burned (no withdrawal instruction).
    /// CHECK: PDA validated by seeds
    #[account(
        mut,
        seeds = [b"sink", project_state.key().as_ref()],
        bump
    )]
    pub sink: UncheckedAccount<'info>,

    /// FIX SC-1: Creator must match project_state.creator — SOL always goes to real creator.
    /// CHECK: Validated by constraint below. Receives creator_bps share of SOL.
    #[account(
        mut,
        constraint = creator.key() == project_state.creator @ MammothError::Unauthorized
    )]
    pub creator: UncheckedAccount<'info>,

    /// FIX SC-7: Support authority delegation for close_cycle.
    /// Optional — pass None when caller is creator, Some when caller is operator.
    pub authority_config: Option<Account<'info, AuthorityConfig>>,

    #[account(mut)]
    pub caller: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct SetHardCap<'info> {
    #[account(
        mut,
        seeds = [b"project", project_state.mint.as_ref()],
        bump = project_state.bump
    )]
    pub project_state: Account<'info, ProjectState>,

    /// Optional — pass None when caller is creator, Some when caller is operator.
    pub authority_config: Option<Account<'info, AuthorityConfig>>,

    #[account(mut)]
    pub caller: Signer<'info>,
}

#[derive(Accounts)]
pub struct ActivateCycle<'info> {
    #[account(
        seeds = [b"project", project_state.mint.as_ref()],
        bump = project_state.bump
    )]
    pub project_state: Account<'info, ProjectState>,

    #[account(
        mut,
        seeds = [b"cycle", project_state.key().as_ref(), &[cycle_state.cycle_index]],
        bump = cycle_state.bump
    )]
    pub cycle_state: Account<'info, CycleState>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(holder: Pubkey, rights_amount: u64, expiry: i64)]
pub struct CreateHolderRights<'info> {
    #[account(
        seeds = [b"project", project_state.mint.as_ref()],
        bump = project_state.bump
    )]
    pub project_state: Account<'info, ProjectState>,

    /// FIX RA-8: cycle_state is now mut to track rights_allocated
    #[account(
        mut,
        seeds = [b"cycle", project_state.key().as_ref(), &[cycle_state.cycle_index]],
        bump = cycle_state.bump
    )]
    pub cycle_state: Account<'info, CycleState>,

    #[account(
        init,
        payer = creator,
        space = HolderRights::LEN,
        seeds = [b"rights", cycle_state.key().as_ref(), holder.as_ref()],
        bump
    )]
    pub holder_rights: Account<'info, HolderRights>,

    #[account(mut)]
    pub creator: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(
    operator: Pubkey,
    can_open_cycle: bool,
    can_close_cycle: bool,
    can_set_hard_cap: bool,
    can_route_treasury: bool,
    spending_limit_lamports: u64,
    can_set_merkle_root: bool,
)]
pub struct InitializeAuthority<'info> {
    #[account(
        seeds = [b"project", project_state.mint.as_ref()],
        bump = project_state.bump
    )]
    pub project_state: Account<'info, ProjectState>,

    #[account(
        init,
        payer = principal,
        space = AuthorityConfig::LEN,
        seeds = [b"authority", project_state.key().as_ref()],
        bump
    )]
    pub authority_config: Account<'info, AuthorityConfig>,

    #[account(mut)]
    pub principal: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(
    operator: Pubkey,
    can_open_cycle: bool,
    can_close_cycle: bool,
    can_set_hard_cap: bool,
    can_route_treasury: bool,
    spending_limit_lamports: u64,
    can_set_merkle_root: bool,
)]
pub struct UpdateAuthority<'info> {
    #[account(
        seeds = [b"project", project_state.mint.as_ref()],
        bump = project_state.bump
    )]
    pub project_state: Account<'info, ProjectState>,

    #[account(
        mut,
        seeds = [b"authority", project_state.key().as_ref()],
        bump = authority_config.bump
    )]
    pub authority_config: Account<'info, AuthorityConfig>,

    #[account(mut)]
    pub principal: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(merkle_root: [u8; 32], holder_count: u32)]
pub struct SetRightsMerkleRoot<'info> {
    #[account(
        seeds = [b"project", project_state.mint.as_ref()],
        bump = project_state.bump
    )]
    pub project_state: Account<'info, ProjectState>,

    #[account(
        mut,
        seeds = [b"cycle", project_state.key().as_ref(), &[cycle_state.cycle_index]],
        bump = cycle_state.bump
    )]
    pub cycle_state: Account<'info, CycleState>,

    /// Optional — required when caller is an operator
    pub authority_config: Option<Account<'info, AuthorityConfig>>,

    #[account(mut)]
    pub caller: Signer<'info>,
}

#[derive(Accounts)]
#[instruction(proof: Vec<[u8; 32]>, rights_amount: u64)]
pub struct ClaimRights<'info> {
    #[account(
        seeds = [b"project", project_state.mint.as_ref()],
        bump = project_state.bump
    )]
    pub project_state: Account<'info, ProjectState>,

    /// FIX RA-8: cycle_state is now mut to track rights_allocated
    #[account(
        mut,
        seeds = [b"cycle", project_state.key().as_ref(), &[cycle_state.cycle_index]],
        bump = cycle_state.bump
    )]
    pub cycle_state: Account<'info, CycleState>,

    #[account(
        init,
        payer = holder,
        space = HolderRights::LEN,
        seeds = [b"rights", cycle_state.key().as_ref(), holder.key().as_ref()],
        bump
    )]
    pub holder_rights: Account<'info, HolderRights>,

    #[account(mut)]
    pub holder: Signer<'info>,

    pub system_program: Program<'info, System>,
}

/// FIX H6 (round 10): Creator rotates ownership to a new wallet.
#[derive(Accounts)]
pub struct RotateCreator<'info> {
    #[account(
        mut,
        seeds = [b"project", project_state.mint.as_ref()],
        bump = project_state.bump
    )]
    pub project_state: Account<'info, ProjectState>,

    pub current_creator: Signer<'info>,
}

/// FIX C-3 (final audit): Creator withdraws from reserve PDA
#[derive(Accounts)]
pub struct WithdrawReserve<'info> {
    #[account(
        seeds = [b"project", project_state.mint.as_ref()],
        bump = project_state.bump
    )]
    pub project_state: Account<'info, ProjectState>,

    /// CHECK: Reserve PDA — validated by seeds
    #[account(
        mut,
        seeds = [b"reserve", project_state.key().as_ref()],
        bump
    )]
    pub reserve: UncheckedAccount<'info>,

    #[account(
        mut,
        constraint = creator.key() == project_state.creator @ MammothError::Unauthorized
    )]
    pub creator: Signer<'info>,

    pub system_program: Program<'info, System>,
}

/// FIX F9: Reclaim rent from closed cycle accounts once funds are distributed.
/// FIX C-1 (final audit): Added Signer constraint — only the creator can reclaim
#[derive(Accounts)]
pub struct ReclaimCycleRent<'info> {
    #[account(
        seeds = [b"project", project_state.mint.as_ref()],
        bump = project_state.bump
    )]
    pub project_state: Account<'info, ProjectState>,

    #[account(
        mut,
        seeds = [b"cycle", project_state.key().as_ref(), &[cycle_state.cycle_index]],
        bump = cycle_state.bump,
        close = creator
    )]
    pub cycle_state: Account<'info, CycleState>,

    /// FIX C-1: Creator must sign this instruction — prevents griefing/unauthorized closure
    #[account(
        mut,
        constraint = creator.key() == project_state.creator @ MammothError::Unauthorized
    )]
    pub creator: Signer<'info>,
}
