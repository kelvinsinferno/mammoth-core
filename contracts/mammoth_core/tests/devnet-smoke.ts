/**
 * Devnet Smoke Test
 *
 * Minimal end-to-end run against devnet to validate the deployed program works.
 * Uses the operator's main wallet (must have SOL on devnet).
 *
 * Run with: ts-node tests/devnet-smoke.ts
 *
 * Steps:
 *   1. Try initialize_protocol (idempotent — skip if already done)
 *   2. Create a fresh project (new random mint)
 *   3. Open a cycle with min rights window (60s)
 *   4. Wait 65s for rights window
 *   5. Activate cycle
 *   6. Buy a few tokens
 *   7. Close cycle
 *   8. Withdraw reserve
 *
 * Total runtime ~90s. Costs ~0.05 SOL.
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
import fs from "fs";
import path from "path";

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function main() {
  console.log("=== Mammoth Devnet Smoke Test ===\n");

  const rpcUrl = "https://api.devnet.solana.com";
  const walletPath =
    process.env.ANCHOR_WALLET ||
    path.join(process.env.HOME || process.env.USERPROFILE || "", ".config", "solana", "id.json");
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

  console.log(`Wallet: ${wallet.publicKey.toBase58()}`);
  const balance = await connection.getBalance(wallet.publicKey);
  console.log(`Balance: ${(balance / 1e9).toFixed(4)} SOL\n`);
  if (balance < 0.1 * LAMPORTS_PER_SOL) {
    throw new Error("Need at least 0.1 SOL on devnet");
  }

  // ── Step 1: initialize_protocol (idempotent) ──
  const [protocolConfig] = PublicKey.findProgramAddressSync([Buffer.from("protocol_config")], program.programId);
  const [protocolTreasury] = PublicKey.findProgramAddressSync(
    [Buffer.from("protocol_treasury")],
    program.programId
  );

  console.log("Step 1: initialize_protocol");
  try {
    const sig = await program.methods
      .initializeProtocol(200, 7000, 2000, 1000)
      .accounts({
        protocolConfig,
        protocolTreasury,
        admin: wallet.publicKey,
        systemProgram: SystemProgram.programId,
      } as any)
      .rpc();
    console.log(`  ✓ initialized — sig: ${sig}`);
  } catch (e: any) {
    if (/already in use|already exists/i.test(e.message || "")) {
      console.log(`  → already initialized (skipping)`);
    } else {
      throw e;
    }
  }

  // ── Step 2: create_project ──
  const mintKp = Keypair.generate();
  const [projectState] = PublicKey.findProgramAddressSync(
    [Buffer.from("project"), mintKp.publicKey.toBuffer()],
    program.programId
  );
  const [reserve] = PublicKey.findProgramAddressSync(
    [Buffer.from("reserve"), projectState.toBuffer()],
    program.programId
  );
  const [sink] = PublicKey.findProgramAddressSync(
    [Buffer.from("sink"), projectState.toBuffer()],
    program.programId
  );
  const protocolTreasuryToken = await getAssociatedTokenAddress(mintKp.publicKey, protocolTreasury, true);
  const creatorToken = await getAssociatedTokenAddress(mintKp.publicKey, wallet.publicKey, false);
  const projectEscrowToken = await getAssociatedTokenAddress(mintKp.publicKey, projectState, true);

  console.log(`\nStep 2: create_project (mint: ${mintKp.publicKey.toBase58()})`);
  const sigCreate = await program.methods
    .createProject(
      { fixed: {} },
      new anchor.BN(1_000_000),
      5000,
      7000,
      2000,
      1000,
      null,
      { human: {} }
    )
    .accounts({
      mint: mintKp.publicKey,
      projectState,
      protocolTreasury,
      protocolTreasuryToken,
      creatorToken,
      projectEscrowToken,
      reserve,
      sink,
      protocolConfig,
      creator: wallet.publicKey,
      tokenProgram: TOKEN_PROGRAM_ID,
      associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    } as any)
    .signers([mintKp])
    .rpc();
  console.log(`  ✓ created — sig: ${sigCreate}`);

  // ── Step 3: open_cycle ──
  const cycleIndex = 0;
  const [cycleState] = PublicKey.findProgramAddressSync(
    [Buffer.from("cycle"), projectState.toBuffer(), Buffer.from([cycleIndex])],
    program.programId
  );

  console.log(`\nStep 3: open_cycle (linear, 60s rights window)`);
  const sigOpen = await program.methods
    .openCycle(
      { linear: {} },
      new anchor.BN(10_000),
      new anchor.BN(1000),
      new anchor.BN(60),
      new anchor.BN(0),
      new anchor.BN(0),
      new anchor.BN(2000),
      new anchor.BN(0)
    )
    .accounts({
      projectState,
      cycleState,
      projectEscrowToken,
      mint: mintKp.publicKey,
      authorityConfig: null,
      caller: wallet.publicKey,
      tokenProgram: TOKEN_PROGRAM_ID,
      associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    } as any)
    .rpc();
  console.log(`  ✓ opened — sig: ${sigOpen}`);

  // ── Step 4: wait for rights window ──
  console.log(`\nStep 4: waiting 65s for rights window to expire...`);
  await sleep(65_000);

  // ── Step 5: activate_cycle ──
  console.log(`\nStep 5: activate_cycle`);
  const sigActivate = await program.methods
    .activateCycle()
    .accounts({
      projectState,
      cycleState,
      systemProgram: SystemProgram.programId,
    } as any)
    .rpc();
  console.log(`  ✓ activated — sig: ${sigActivate}`);

  // ── Step 6: buy_tokens ──
  const buyerToken = await getAssociatedTokenAddress(mintKp.publicKey, wallet.publicKey);
  console.log(`\nStep 6: buy_tokens (100 tokens)`);
  const sigBuy = await program.methods
    .buyTokens(new anchor.BN(100), new anchor.BN(1_000_000))
    .accounts({
      projectState,
      cycleState,
      protocolConfig,
      protocolTreasury,
      projectEscrowToken,
      buyerToken,
      mint: mintKp.publicKey,
      buyer: wallet.publicKey,
      tokenProgram: TOKEN_PROGRAM_ID,
      associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    } as any)
    .rpc();
  console.log(`  ✓ bought — sig: ${sigBuy}`);

  // ── Step 7: close_cycle ──
  console.log(`\nStep 7: close_cycle`);
  const sigClose = await program.methods
    .closeCycle()
    .accounts({
      projectState,
      cycleState,
      reserve,
      sink,
      creator: wallet.publicKey,
      authorityConfig: null,
      caller: wallet.publicKey,
      systemProgram: SystemProgram.programId,
    } as any)
    .rpc();
  console.log(`  ✓ closed — sig: ${sigClose}`);

  // ── Step 8: withdraw_reserve ──
  const reserveBalance = await connection.getBalance(reserve);
  console.log(`\nStep 8: withdraw_reserve (${reserveBalance} lamports)`);
  if (reserveBalance > 1_000_000) {
    // Leave rent-exempt minimum
    const withdrawAmount = reserveBalance - 1_000_000;
    const sigWithdraw = await program.methods
      .withdrawReserve(new anchor.BN(withdrawAmount))
      .accounts({
        projectState,
        reserve,
        creator: wallet.publicKey,
        systemProgram: SystemProgram.programId,
      } as any)
      .rpc();
    console.log(`  ✓ withdrew ${withdrawAmount} lamports — sig: ${sigWithdraw}`);
  } else {
    console.log(`  → reserve too small to withdraw (skipped)`);
  }

  console.log(`\n=== ALL STEPS PASSED ===`);
  console.log(`Project mint: ${mintKp.publicKey.toBase58()}`);
  console.log(`View on explorer: https://explorer.solana.com/address/${mintKp.publicKey.toBase58()}?cluster=devnet`);
}

main().catch((e) => {
  console.error("\n=== FAILED ===");
  console.error(e);
  process.exit(1);
});
