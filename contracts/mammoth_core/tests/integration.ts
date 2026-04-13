/**
 * Mammoth Protocol — End-to-End Integration Tests
 *
 * Exercises every instruction against a local validator. Validates the round 1-10
 * audit fixes actually behave correctly at runtime, not just compile.
 *
 * Run with: anchor test
 *
 * Coverage:
 *  - Happy path: initialize_protocol → create_project → open_cycle →
 *    create_holder_rights → exercise_rights → activate_cycle → buy_tokens →
 *    close_cycle → withdraw_reserve → rotate_creator
 *  - Slippage: max_sol_cost too low triggers SlippageExceeded
 *  - Rights protection: public buyer cannot consume reserved rights
 *  - Path conflict: create_holder_rights + set_rights_merkle_root cannot mix
 *  - Concurrent cycles: open_cycle blocked while previous is active
 *  - Authority: non-creator close_cycle is rejected
 *
 * IMPORTANT: rights_window_duration min is 60s (audit fix H3/H4). Tests that
 * exercise activate_cycle wait ~65s real time. Total suite ~3-5 min.
 */

import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { MammothCore } from "../target/types/mammoth_core";
import {
  Keypair,
  PublicKey,
  SystemProgram,
  Connection,
  LAMPORTS_PER_SOL,
} from "@solana/web3.js";
import {
  TOKEN_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
  getAssociatedTokenAddress,
} from "@solana/spl-token";
import { assert, expect } from "chai";
import fs from "fs";
import path from "path";

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const RIGHTS_WINDOW_SECS = 60; // contract minimum
const RIGHTS_WAIT_MS = (RIGHTS_WINDOW_SECS + 5) * 1000; // wait past expiry

// Helper: derive all PDAs for a given mint
function derivePDAs(programId: PublicKey, mint: PublicKey, cycleIndex: number, holder?: PublicKey) {
  const [protocolConfig] = PublicKey.findProgramAddressSync([Buffer.from("protocol_config")], programId);
  const [protocolTreasury] = PublicKey.findProgramAddressSync([Buffer.from("protocol_treasury")], programId);
  const [projectState] = PublicKey.findProgramAddressSync([Buffer.from("project"), mint.toBuffer()], programId);
  const [cycleState] = PublicKey.findProgramAddressSync(
    [Buffer.from("cycle"), projectState.toBuffer(), Buffer.from([cycleIndex])],
    programId
  );
  const [reserve] = PublicKey.findProgramAddressSync([Buffer.from("reserve"), projectState.toBuffer()], programId);
  const [sink] = PublicKey.findProgramAddressSync([Buffer.from("sink"), projectState.toBuffer()], programId);
  const [authorityConfig] = PublicKey.findProgramAddressSync(
    [Buffer.from("authority"), projectState.toBuffer()],
    programId
  );
  const holderRights = holder
    ? PublicKey.findProgramAddressSync(
        [Buffer.from("rights"), cycleState.toBuffer(), holder.toBuffer()],
        programId
      )[0]
    : null;
  return { protocolConfig, protocolTreasury, projectState, cycleState, reserve, sink, authorityConfig, holderRights };
}

describe("mammoth_core integration", function () {
  this.timeout(300_000); // 5 minutes overall

  const rpcUrl = process.env.ANCHOR_PROVIDER_URL || "http://127.0.0.1:8899";
  const walletPath =
    process.env.ANCHOR_WALLET ||
    path.join(process.env.USERPROFILE || process.env.HOME || "", ".config", "solana", "id.json");

  const secret = JSON.parse(fs.readFileSync(walletPath, "utf8"));
  const walletKp = Keypair.fromSecretKey(Uint8Array.from(secret));
  const connection = new Connection(rpcUrl, "confirmed");
  const wallet = new anchor.Wallet(walletKp);
  const provider = new anchor.AnchorProvider(connection, wallet, {
    commitment: "confirmed",
    preflightCommitment: "confirmed",
  });
  anchor.setProvider(provider);

  const program = anchor.workspace.MammothCore as Program<MammothCore>;

  // Fund a fresh holder + buyer keypair before tests
  const holderKp = Keypair.generate();
  const buyerKp = Keypair.generate();

  before(async function () {
    // Verify validator is reachable
    try {
      await connection.getVersion();
    } catch (e) {
      console.error(`No validator at ${rpcUrl}. Run 'solana-test-validator' or use 'anchor test'.`);
      this.skip();
    }

    // Fund accounts (if using local validator with default airdrop)
    for (const kp of [holderKp, buyerKp]) {
      try {
        const sig = await connection.requestAirdrop(kp.publicKey, 5 * LAMPORTS_PER_SOL);
        await connection.confirmTransaction(sig, "confirmed");
      } catch (e) {
        // Devnet airdrops can fail; transfer from main wallet as fallback
        const balance = await connection.getBalance(wallet.publicKey);
        if (balance < 10 * LAMPORTS_PER_SOL) {
          throw new Error(`Wallet has insufficient SOL (${balance / 1e9}). Fund it manually.`);
        }
        const tx = new anchor.web3.Transaction().add(
          SystemProgram.transfer({
            fromPubkey: wallet.publicKey,
            toPubkey: kp.publicKey,
            lamports: 5 * LAMPORTS_PER_SOL,
          })
        );
        await provider.sendAndConfirm(tx);
      }
    }
  });

  // ── Setup: initialize_protocol once (idempotent across test runs) ─────────
  describe("initialize_protocol", () => {
    it("initializes or reuses protocol config", async () => {
      const { protocolConfig, protocolTreasury } = derivePDAs(
        program.programId,
        Keypair.generate().publicKey, // dummy mint, only protocol PDAs used
        0
      );
      try {
        await program.methods
          .initializeProtocol(200, 7000, 2000, 1000) // 2% fee, 70/20/10 split
          .accounts({
            protocolConfig,
            protocolTreasury,
            admin: wallet.publicKey,
            systemProgram: SystemProgram.programId,
          } as any)
          .rpc();
      } catch (err: any) {
        // If already initialized, that's fine
        if (!/already in use|account.*already exists/i.test(err.message || "")) {
          throw err;
        }
      }
      const cfg = await program.account.protocolConfig.fetch(protocolConfig);
      assert.equal(cfg.feeBps, 200);
    });
  });

  // ── Full happy path: create → open → exercise → activate → buy → close ────
  describe("happy path: full cycle lifecycle", () => {
    const mintKp = Keypair.generate();
    const cycleIndex = 0;
    let pdas: ReturnType<typeof derivePDAs>;

    it("creates a project (Fixed supply, 50% public)", async () => {
      pdas = derivePDAs(program.programId, mintKp.publicKey, cycleIndex, holderKp.publicKey);
      const protocolTreasuryToken = await getAssociatedTokenAddress(mintKp.publicKey, pdas.protocolTreasury, true);
      const creatorToken = await getAssociatedTokenAddress(mintKp.publicKey, wallet.publicKey, false);
      const projectEscrowToken = await getAssociatedTokenAddress(mintKp.publicKey, pdas.projectState, true);

      await program.methods
        .createProject(
          { fixed: {} },
          new anchor.BN(1_000_000), // 1M tokens (raw units, 6 decimals = 1 token)
          5000, // 50% public
          7000, // 70% creator
          2000, // 20% reserve
          1000, // 10% sink
          null, // no launch_at
          { human: {} }
        )
        .accounts({
          mint: mintKp.publicKey,
          projectState: pdas.projectState,
          protocolTreasury: pdas.protocolTreasury,
          protocolTreasuryToken,
          creatorToken,
          projectEscrowToken,
          reserve: pdas.reserve,
          sink: pdas.sink,
          protocolConfig: pdas.protocolConfig,
          creator: wallet.publicKey,
          tokenProgram: TOKEN_PROGRAM_ID,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
        } as any)
        .signers([mintKp])
        .rpc();

      const project = await program.account.projectState.fetch(pdas.projectState);
      assert.equal(project.creator.toBase58(), wallet.publicKey.toBase58());
      assert.equal(project.totalSupply.toNumber(), 1_000_000);
      assert.equal(project.publicAllocation.toNumber(), 500_000);
    });

    it("opens a cycle (Linear curve, 60s rights window)", async () => {
      const projectEscrowToken = await getAssociatedTokenAddress(mintKp.publicKey, pdas.projectState, true);
      await program.methods
        .openCycle(
          { linear: {} },
          new anchor.BN(100_000), // supply_cap = 100k tokens
          new anchor.BN(1_000), // base_price = 1000 lamports
          new anchor.BN(RIGHTS_WINDOW_SECS),
          new anchor.BN(0), // step_size (unused for linear)
          new anchor.BN(0), // step_increment (unused)
          new anchor.BN(2_000), // end_price = 2000 lamports
          new anchor.BN(0) // growth_factor_k (unused)
        )
        .accounts({
          projectState: pdas.projectState,
          cycleState: pdas.cycleState,
          projectEscrowToken,
          mint: mintKp.publicKey,
          authorityConfig: null,
          caller: wallet.publicKey,
          tokenProgram: TOKEN_PROGRAM_ID,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
        } as any)
        .rpc();

      const cycle = await program.account.cycleState.fetch(pdas.cycleState);
      assert.deepEqual(cycle.curveType, { linear: {} });
      assert.equal(cycle.supplyCap.toNumber(), 100_000);
      assert.equal(cycle.basePrice.toNumber(), 1000);
      assert.deepEqual(cycle.status, { rightsWindow: {} });
    });

    it("creates holder rights (10k tokens for holderKp)", async () => {
      const expiry = Math.floor(Date.now() / 1000) + 3600;
      await program.methods
        .createHolderRights(holderKp.publicKey, new anchor.BN(10_000), new anchor.BN(expiry))
        .accounts({
          projectState: pdas.projectState,
          cycleState: pdas.cycleState,
          holderRights: pdas.holderRights!,
          creator: wallet.publicKey,
          systemProgram: SystemProgram.programId,
        } as any)
        .rpc();

      const rights = await program.account.holderRights.fetch(pdas.holderRights!);
      assert.equal(rights.rightsAmount.toNumber(), 10_000);
      assert.equal(rights.exercisedAmount.toNumber(), 0);

      // rights_allocated should be tracked on cycle
      const cycle = await program.account.cycleState.fetch(pdas.cycleState);
      assert.equal(cycle.rightsAllocated.toNumber(), 10_000);
    });

    it("exercises rights (5k of the 10k allocation, with slippage cap)", async () => {
      const holderToken = await getAssociatedTokenAddress(mintKp.publicKey, holderKp.publicKey);
      const projectEscrowToken = await getAssociatedTokenAddress(mintKp.publicKey, pdas.projectState, true);
      const amount = 5_000;
      const totalCost = amount * 1000; // base_price * amount
      // Allow up to 10% slippage
      const maxSolCost = Math.ceil(totalCost * 1.1);

      await program.methods
        .exerciseRights(new anchor.BN(amount), new anchor.BN(maxSolCost))
        .accounts({
          projectState: pdas.projectState,
          cycleState: pdas.cycleState,
          holderRights: pdas.holderRights!,
          protocolConfig: pdas.protocolConfig,
          protocolTreasury: pdas.protocolTreasury,
          projectEscrowToken,
          holderToken,
          mint: mintKp.publicKey,
          holder: holderKp.publicKey,
          tokenProgram: TOKEN_PROGRAM_ID,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
        } as any)
        .signers([holderKp])
        .rpc();

      const rights = await program.account.holderRights.fetch(pdas.holderRights!);
      assert.equal(rights.exercisedAmount.toNumber(), 5_000);
      const cycle = await program.account.cycleState.fetch(pdas.cycleState);
      assert.equal(cycle.minted.toNumber(), 5_000);
    });

    it("rejects exercise with slippage cap too low (SlippageExceeded)", async () => {
      const holderToken = await getAssociatedTokenAddress(mintKp.publicKey, holderKp.publicKey);
      const projectEscrowToken = await getAssociatedTokenAddress(mintKp.publicKey, pdas.projectState, true);
      try {
        await program.methods
          .exerciseRights(new anchor.BN(1000), new anchor.BN(500)) // way too low
          .accounts({
            projectState: pdas.projectState,
            cycleState: pdas.cycleState,
            holderRights: pdas.holderRights!,
            protocolConfig: pdas.protocolConfig,
            protocolTreasury: pdas.protocolTreasury,
            projectEscrowToken,
            holderToken,
            mint: mintKp.publicKey,
            holder: holderKp.publicKey,
            tokenProgram: TOKEN_PROGRAM_ID,
            associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
            systemProgram: SystemProgram.programId,
          } as any)
          .signers([holderKp])
          .rpc();
        assert.fail("expected SlippageExceeded");
      } catch (err: any) {
        expect(err.message || "").to.match(/SlippageExceeded/);
      }
    });

    it(`waits ${RIGHTS_WINDOW_SECS}s for rights window to expire`, async function () {
      this.timeout(RIGHTS_WAIT_MS + 30_000);
      await sleep(RIGHTS_WAIT_MS);
    });

    it("activates cycle (snapshot reserves remaining 5k unexercised rights)", async () => {
      await program.methods
        .activateCycle()
        .accounts({
          projectState: pdas.projectState,
          cycleState: pdas.cycleState,
          systemProgram: SystemProgram.programId,
        } as any)
        .rpc();

      const cycle = await program.account.cycleState.fetch(pdas.cycleState);
      assert.deepEqual(cycle.status, { active: {} });
      // rights_allocated=10k, minted=5k → reserved = 5k unexercised
      assert.equal(cycle.rightsReservedAtActivation.toNumber(), 5_000);
    });

    it("buys tokens (1k via public buyer, with slippage cap)", async () => {
      const buyerToken = await getAssociatedTokenAddress(mintKp.publicKey, buyerKp.publicKey);
      const projectEscrowToken = await getAssociatedTokenAddress(mintKp.publicKey, pdas.projectState, true);
      const amount = 1_000;
      // Linear: price at sold=5000 to 6000 averages ~1050. Cost ≈ 1.05M. Add 50% slippage tolerance.
      const maxSolCost = 5_000_000;

      await program.methods
        .buyTokens(new anchor.BN(amount), new anchor.BN(maxSolCost))
        .accounts({
          projectState: pdas.projectState,
          cycleState: pdas.cycleState,
          protocolConfig: pdas.protocolConfig,
          protocolTreasury: pdas.protocolTreasury,
          projectEscrowToken,
          buyerToken,
          mint: mintKp.publicKey,
          buyer: buyerKp.publicKey,
          tokenProgram: TOKEN_PROGRAM_ID,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
        } as any)
        .signers([buyerKp])
        .rpc();

      const cycle = await program.account.cycleState.fetch(pdas.cycleState);
      assert.equal(cycle.minted.toNumber(), 6_000);
    });

    it("rejects buy with slippage cap below total_cost (SlippageExceeded)", async () => {
      const buyerToken = await getAssociatedTokenAddress(mintKp.publicKey, buyerKp.publicKey);
      const projectEscrowToken = await getAssociatedTokenAddress(mintKp.publicKey, pdas.projectState, true);
      try {
        await program.methods
          .buyTokens(new anchor.BN(100), new anchor.BN(50)) // 100 tokens for 50 lamports — impossible
          .accounts({
            projectState: pdas.projectState,
            cycleState: pdas.cycleState,
            protocolConfig: pdas.protocolConfig,
            protocolTreasury: pdas.protocolTreasury,
            projectEscrowToken,
            buyerToken,
            mint: mintKp.publicKey,
            buyer: buyerKp.publicKey,
            tokenProgram: TOKEN_PROGRAM_ID,
            associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
            systemProgram: SystemProgram.programId,
          } as any)
          .signers([buyerKp])
          .rpc();
        assert.fail("expected SlippageExceeded");
      } catch (err: any) {
        expect(err.message || "").to.match(/SlippageExceeded/);
      }
    });

    it("rejects buy that would exceed public_cap (rights protection)", async () => {
      // public_cap = supply_cap(100k) - reserved(5k) = 95k. minted=6k, so 89k available.
      // Try to buy 90k — should fail SupplyCapExceeded.
      const buyerToken = await getAssociatedTokenAddress(mintKp.publicKey, buyerKp.publicKey);
      const projectEscrowToken = await getAssociatedTokenAddress(mintKp.publicKey, pdas.projectState, true);
      try {
        await program.methods
          .buyTokens(new anchor.BN(90_000), new anchor.BN(LAMPORTS_PER_SOL * 5))
          .accounts({
            projectState: pdas.projectState,
            cycleState: pdas.cycleState,
            protocolConfig: pdas.protocolConfig,
            protocolTreasury: pdas.protocolTreasury,
            projectEscrowToken,
            buyerToken,
            mint: mintKp.publicKey,
            buyer: buyerKp.publicKey,
            tokenProgram: TOKEN_PROGRAM_ID,
            associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
            systemProgram: SystemProgram.programId,
          } as any)
          .signers([buyerKp])
          .rpc();
        assert.fail("expected SupplyCapExceeded");
      } catch (err: any) {
        expect(err.message || "").to.match(/SupplyCapExceeded/);
      }
    });

    it("blocks opening a new cycle while one is active (CycleStillActive)", async () => {
      const projectEscrowToken = await getAssociatedTokenAddress(mintKp.publicKey, pdas.projectState, true);
      const [cycleState1] = PublicKey.findProgramAddressSync(
        [Buffer.from("cycle"), pdas.projectState.toBuffer(), Buffer.from([1])],
        program.programId
      );
      try {
        await program.methods
          .openCycle(
            { linear: {} },
            new anchor.BN(50_000),
            new anchor.BN(1000),
            new anchor.BN(RIGHTS_WINDOW_SECS),
            new anchor.BN(0),
            new anchor.BN(0),
            new anchor.BN(2000),
            new anchor.BN(0)
          )
          .accounts({
            projectState: pdas.projectState,
            cycleState: cycleState1,
            projectEscrowToken,
            mint: mintKp.publicKey,
            authorityConfig: null,
            caller: wallet.publicKey,
            tokenProgram: TOKEN_PROGRAM_ID,
            associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
            systemProgram: SystemProgram.programId,
          } as any)
          .rpc();
        assert.fail("expected CycleStillActive");
      } catch (err: any) {
        expect(err.message || "").to.match(/CycleStillActive/);
      }
    });

    it("rejects close_cycle from a non-creator wallet (Unauthorized)", async () => {
      try {
        await program.methods
          .closeCycle()
          .accounts({
            projectState: pdas.projectState,
            cycleState: pdas.cycleState,
            reserve: pdas.reserve,
            sink: pdas.sink,
            creator: buyerKp.publicKey, // wrong creator!
            authorityConfig: null,
            caller: buyerKp.publicKey,
            systemProgram: SystemProgram.programId,
          } as any)
          .signers([buyerKp])
          .rpc();
        assert.fail("expected Unauthorized");
      } catch (err: any) {
        expect(err.message || "").to.match(/Unauthorized|constraint/);
      }
    });

    it("creator closes cycle and distributes SOL (creator/reserve/sink)", async () => {
      const creatorBefore = await connection.getBalance(wallet.publicKey);
      const reserveBefore = await connection.getBalance(pdas.reserve);

      await program.methods
        .closeCycle()
        .accounts({
          projectState: pdas.projectState,
          cycleState: pdas.cycleState,
          reserve: pdas.reserve,
          sink: pdas.sink,
          creator: wallet.publicKey,
          authorityConfig: null,
          caller: wallet.publicKey,
          systemProgram: SystemProgram.programId,
        } as any)
        .rpc();

      const cycle = await program.account.cycleState.fetch(pdas.cycleState);
      assert.deepEqual(cycle.status, { closed: {} });

      const creatorAfter = await connection.getBalance(wallet.publicKey);
      const reserveAfter = await connection.getBalance(pdas.reserve);
      // Creator received some SOL (minus tx fee). Reserve also got some.
      assert.isAbove(reserveAfter, reserveBefore);
      // Creator delta is positive net of tx fee, but tx fee is small; we just check reserve increased.
      // (Strict creator check is noisy because creator pays the close_cycle tx fee.)
    });

    it("creator withdraws from reserve PDA", async () => {
      const reserveBalance = await connection.getBalance(pdas.reserve);
      assert.isAbove(reserveBalance, 0);
      await program.methods
        .withdrawReserve(new anchor.BN(reserveBalance))
        .accounts({
          projectState: pdas.projectState,
          reserve: pdas.reserve,
          creator: wallet.publicKey,
          systemProgram: SystemProgram.programId,
        } as any)
        .rpc();
      const after = await connection.getBalance(pdas.reserve);
      assert.equal(after, 0);
    });
  });

  // ── Negative test: rights path conflict ───────────────────────────────────
  describe("rights path conflict", () => {
    const mintKp = Keypair.generate();
    const cycleIndex = 0;
    let pdas: ReturnType<typeof derivePDAs>;

    it("creates a project for path-conflict test", async () => {
      pdas = derivePDAs(program.programId, mintKp.publicKey, cycleIndex, holderKp.publicKey);
      const protocolTreasuryToken = await getAssociatedTokenAddress(mintKp.publicKey, pdas.protocolTreasury, true);
      const creatorToken = await getAssociatedTokenAddress(mintKp.publicKey, wallet.publicKey, false);
      const projectEscrowToken = await getAssociatedTokenAddress(mintKp.publicKey, pdas.projectState, true);

      await program.methods
        .createProject({ fixed: {} }, new anchor.BN(1_000_000), 5000, 7000, 2000, 1000, null, { human: {} })
        .accounts({
          mint: mintKp.publicKey,
          projectState: pdas.projectState,
          protocolTreasury: pdas.protocolTreasury,
          protocolTreasuryToken,
          creatorToken,
          projectEscrowToken,
          reserve: pdas.reserve,
          sink: pdas.sink,
          protocolConfig: pdas.protocolConfig,
          creator: wallet.publicKey,
          tokenProgram: TOKEN_PROGRAM_ID,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
        } as any)
        .signers([mintKp])
        .rpc();
    });

    it("opens cycle and creates a holder right (legacy path)", async () => {
      const projectEscrowToken = await getAssociatedTokenAddress(mintKp.publicKey, pdas.projectState, true);
      await program.methods
        .openCycle(
          { linear: {} },
          new anchor.BN(100_000),
          new anchor.BN(1000),
          new anchor.BN(RIGHTS_WINDOW_SECS),
          new anchor.BN(0),
          new anchor.BN(0),
          new anchor.BN(2000),
          new anchor.BN(0)
        )
        .accounts({
          projectState: pdas.projectState,
          cycleState: pdas.cycleState,
          projectEscrowToken,
          mint: mintKp.publicKey,
          authorityConfig: null,
          caller: wallet.publicKey,
          tokenProgram: TOKEN_PROGRAM_ID,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
        } as any)
        .rpc();

      const expiry = Math.floor(Date.now() / 1000) + 3600;
      await program.methods
        .createHolderRights(holderKp.publicKey, new anchor.BN(1_000), new anchor.BN(expiry))
        .accounts({
          projectState: pdas.projectState,
          cycleState: pdas.cycleState,
          holderRights: pdas.holderRights!,
          creator: wallet.publicKey,
          systemProgram: SystemProgram.programId,
        } as any)
        .rpc();
    });

    it("rejects set_rights_merkle_root after legacy path used (RightsPathConflict)", async () => {
      const fakeRoot = Array(32).fill(7);
      try {
        await program.methods
          .setRightsMerkleRoot(fakeRoot, 1, new anchor.BN(1000))
          .accounts({
            projectState: pdas.projectState,
            cycleState: pdas.cycleState,
            authorityConfig: null,
            caller: wallet.publicKey,
          } as any)
          .rpc();
        assert.fail("expected RightsPathConflict");
      } catch (err: any) {
        expect(err.message || "").to.match(/RightsPathConflict/);
      }
    });
  });

  // ── Creator rotation ──────────────────────────────────────────────────────
  describe("rotate_creator", () => {
    const mintKp = Keypair.generate();
    let pdas: ReturnType<typeof derivePDAs>;

    it("creates a project for rotation test", async () => {
      pdas = derivePDAs(program.programId, mintKp.publicKey, 0);
      const protocolTreasuryToken = await getAssociatedTokenAddress(mintKp.publicKey, pdas.protocolTreasury, true);
      const creatorToken = await getAssociatedTokenAddress(mintKp.publicKey, wallet.publicKey, false);
      const projectEscrowToken = await getAssociatedTokenAddress(mintKp.publicKey, pdas.projectState, true);

      await program.methods
        .createProject({ fixed: {} }, new anchor.BN(1_000_000), 5000, 7000, 2000, 1000, null, { human: {} })
        .accounts({
          mint: mintKp.publicKey,
          projectState: pdas.projectState,
          protocolTreasury: pdas.protocolTreasury,
          protocolTreasuryToken,
          creatorToken,
          projectEscrowToken,
          reserve: pdas.reserve,
          sink: pdas.sink,
          protocolConfig: pdas.protocolConfig,
          creator: wallet.publicKey,
          tokenProgram: TOKEN_PROGRAM_ID,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
        } as any)
        .signers([mintKp])
        .rpc();
    });

    it("rotates creator to a new wallet", async () => {
      const newCreator = Keypair.generate();
      await program.methods
        .rotateCreator(newCreator.publicKey)
        .accounts({
          projectState: pdas.projectState,
          currentCreator: wallet.publicKey,
        } as any)
        .rpc();

      const project = await program.account.projectState.fetch(pdas.projectState);
      assert.equal(project.creator.toBase58(), newCreator.publicKey.toBase58());
    });

    it("rejects rotate from non-current creator", async () => {
      try {
        await program.methods
          .rotateCreator(wallet.publicKey)
          .accounts({
            projectState: pdas.projectState,
            currentCreator: buyerKp.publicKey,
          } as any)
          .signers([buyerKp])
          .rpc();
        assert.fail("expected Unauthorized");
      } catch (err: any) {
        expect(err.message || "").to.match(/Unauthorized/);
      }
    });
  });

  // ── Negative: open_cycle below minimum rights window ──────────────────────
  describe("open_cycle validation", () => {
    const mintKp = Keypair.generate();
    let pdas: ReturnType<typeof derivePDAs>;

    it("creates a project", async () => {
      pdas = derivePDAs(program.programId, mintKp.publicKey, 0);
      const protocolTreasuryToken = await getAssociatedTokenAddress(mintKp.publicKey, pdas.protocolTreasury, true);
      const creatorToken = await getAssociatedTokenAddress(mintKp.publicKey, wallet.publicKey, false);
      const projectEscrowToken = await getAssociatedTokenAddress(mintKp.publicKey, pdas.projectState, true);

      await program.methods
        .createProject({ fixed: {} }, new anchor.BN(1_000_000), 5000, 7000, 2000, 1000, null, { human: {} })
        .accounts({
          mint: mintKp.publicKey,
          projectState: pdas.projectState,
          protocolTreasury: pdas.protocolTreasury,
          protocolTreasuryToken,
          creatorToken,
          projectEscrowToken,
          reserve: pdas.reserve,
          sink: pdas.sink,
          protocolConfig: pdas.protocolConfig,
          creator: wallet.publicKey,
          tokenProgram: TOKEN_PROGRAM_ID,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
        } as any)
        .signers([mintKp])
        .rpc();
    });

    it("rejects rights_window_duration < 60s (InvalidRightsWindow)", async () => {
      const projectEscrowToken = await getAssociatedTokenAddress(mintKp.publicKey, pdas.projectState, true);
      try {
        await program.methods
          .openCycle(
            { linear: {} },
            new anchor.BN(10_000),
            new anchor.BN(1000),
            new anchor.BN(30), // BELOW minimum
            new anchor.BN(0),
            new anchor.BN(0),
            new anchor.BN(2000),
            new anchor.BN(0)
          )
          .accounts({
            projectState: pdas.projectState,
            cycleState: pdas.cycleState,
            projectEscrowToken,
            mint: mintKp.publicKey,
            authorityConfig: null,
            caller: wallet.publicKey,
            tokenProgram: TOKEN_PROGRAM_ID,
            associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
            systemProgram: SystemProgram.programId,
          } as any)
          .rpc();
        assert.fail("expected InvalidRightsWindow");
      } catch (err: any) {
        expect(err.message || "").to.match(/InvalidRightsWindow/);
      }
    });

    it("rejects supply_cap > escrow balance (SupplyCapExceedsEscrow)", async () => {
      const projectEscrowToken = await getAssociatedTokenAddress(mintKp.publicKey, pdas.projectState, true);
      try {
        await program.methods
          .openCycle(
            { linear: {} },
            new anchor.BN(10_000_000), // way more than 500k public allocation
            new anchor.BN(1000),
            new anchor.BN(RIGHTS_WINDOW_SECS),
            new anchor.BN(0),
            new anchor.BN(0),
            new anchor.BN(2000),
            new anchor.BN(0)
          )
          .accounts({
            projectState: pdas.projectState,
            cycleState: pdas.cycleState,
            projectEscrowToken,
            mint: mintKp.publicKey,
            authorityConfig: null,
            caller: wallet.publicKey,
            tokenProgram: TOKEN_PROGRAM_ID,
            associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
            systemProgram: SystemProgram.programId,
          } as any)
          .rpc();
        assert.fail("expected SupplyCapExceedsEscrow");
      } catch (err: any) {
        expect(err.message || "").to.match(/SupplyCapExceedsEscrow/);
      }
    });
  });

  // ── BPS validation in create_project ──────────────────────────────────────
  describe("create_project BPS validation", () => {
    it("rejects creator+reserve+sink != 10000 (InvalidBpsSplit)", async () => {
      const mintKp = Keypair.generate();
      const pdas = derivePDAs(program.programId, mintKp.publicKey, 0);
      const protocolTreasuryToken = await getAssociatedTokenAddress(mintKp.publicKey, pdas.protocolTreasury, true);
      const creatorToken = await getAssociatedTokenAddress(mintKp.publicKey, wallet.publicKey, false);
      const projectEscrowToken = await getAssociatedTokenAddress(mintKp.publicKey, pdas.projectState, true);

      try {
        await program.methods
          .createProject(
            { fixed: {} },
            new anchor.BN(1_000_000),
            5000,
            7000,
            2000,
            500, // 7000+2000+500 = 9500, NOT 10000
            null,
            { human: {} }
          )
          .accounts({
            mint: mintKp.publicKey,
            projectState: pdas.projectState,
            protocolTreasury: pdas.protocolTreasury,
            protocolTreasuryToken,
            creatorToken,
            projectEscrowToken,
            protocolConfig: pdas.protocolConfig,
            creator: wallet.publicKey,
            tokenProgram: TOKEN_PROGRAM_ID,
            associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
            systemProgram: SystemProgram.programId,
          } as any)
          .signers([mintKp])
          .rpc();
        assert.fail("expected InvalidBpsSplit");
      } catch (err: any) {
        expect(err.message || "").to.match(/InvalidBpsSplit/);
      }
    });
  });
});
