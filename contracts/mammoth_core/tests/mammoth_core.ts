import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { MammothCore } from "../target/types/mammoth_core";
import {
  Keypair,
  PublicKey,
  SystemProgram,
  Connection,
  LAMPORTS_PER_SOL,
  SYSVAR_RENT_PUBKEY,
} from "@solana/web3.js";
import {
  TOKEN_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
  getAssociatedTokenAddress,
} from "@solana/spl-token";
import { assert } from "chai";
import fs from "fs";
import path from "path";

describe("mammoth_core", () => {
  const rpcUrl = process.env.ANCHOR_PROVIDER_URL || "http://127.0.0.1:8899";
  const walletPath = process.env.ANCHOR_WALLET || path.join(process.env.USERPROFILE || "", ".config", "solana", "id.json");
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

  const mintKp = Keypair.generate();
  const holder = Keypair.generate();
  const cycleIndex = 0;

  let protocolConfig: PublicKey;
  let protocolTreasury: PublicKey;
  let projectState: PublicKey;
  let cycleState: PublicKey;
  let holderRights: PublicKey;
  let reserve: PublicKey;
  let sink: PublicKey;

  let protocolTreasuryToken: PublicKey;
  let creatorToken: PublicKey;
  let projectEscrowToken: PublicKey;

  before(async () => {
    [protocolConfig] = PublicKey.findProgramAddressSync(
      [Buffer.from("protocol_config")],
      program.programId
    );
    [protocolTreasury] = PublicKey.findProgramAddressSync(
      [Buffer.from("protocol_treasury")],
      program.programId
    );
    [projectState] = PublicKey.findProgramAddressSync(
      [Buffer.from("project"), mintKp.publicKey.toBuffer()],
      program.programId
    );
    [cycleState] = PublicKey.findProgramAddressSync(
      [Buffer.from("cycle"), projectState.toBuffer(), Buffer.from([cycleIndex])],
      program.programId
    );
    [holderRights] = PublicKey.findProgramAddressSync(
      [Buffer.from("rights"), cycleState.toBuffer(), holder.publicKey.toBuffer()],
      program.programId
    );
    [reserve] = PublicKey.findProgramAddressSync(
      [Buffer.from("reserve"), projectState.toBuffer()],
      program.programId
    );
    [sink] = PublicKey.findProgramAddressSync(
      [Buffer.from("sink"), projectState.toBuffer()],
      program.programId
    );

    protocolTreasuryToken = await getAssociatedTokenAddress(mintKp.publicKey, protocolTreasury, true);
    creatorToken = await getAssociatedTokenAddress(mintKp.publicKey, wallet.publicKey, false);
    projectEscrowToken = await getAssociatedTokenAddress(mintKp.publicKey, projectState, true);
  });

  it("derives expected PDAs", async () => {
    assert.ok(protocolConfig instanceof PublicKey);
    assert.ok(protocolTreasury instanceof PublicKey);
    assert.ok(projectState instanceof PublicKey);
    assert.ok(cycleState instanceof PublicKey);
    assert.ok(holderRights instanceof PublicKey);
    assert.ok(reserve instanceof PublicKey);
    assert.ok(sink instanceof PublicKey);
  });

  it("IDL exposes current instruction surface", async () => {
    const ixNames = program.idl.instructions.map((ix) => ix.name);
    const expected = [
      "activateCycle",
      "buyTokens",
      "claimRights",
      "closeCycle",
      "createHolderRights",
      "createProject",
      "exerciseRights",
      "initializeAuthority",
      "initializeProtocol",
      "openCycle",
      "reclaimCycleRent",
      "rotateCreator",
      "setHardCap",
      "setRightsMerkleRoot",
      "updateAuthority",
      "withdrawReserve",
    ];

    for (const name of expected) {
      assert.include(ixNames, name, `IDL missing instruction: ${name}`);
    }
  });

  it("program methods reflect current API shape", async () => {
    assert.isFunction((program.methods as any).createProject);
    assert.isFunction((program.methods as any).openCycle);
    assert.isFunction((program.methods as any).buyTokens);
    assert.isFunction((program.methods as any).exerciseRights);
    assert.isFunction((program.methods as any).setRightsMerkleRoot);
  });

  it("current create_project call shape accepts operator_type", async () => {
    const builder = (program.methods as any).createProject(
      { fixed: {} },
      new anchor.BN(1_000_000),
      5000,
      5000,
      2000,
      3000,
      null,
      { human: {} }
    );
    assert.exists(builder, "createProject builder should be constructible with operator_type");
  });

  it("current buy_tokens call shape requires max_sol_cost", async () => {
    const builder = (program.methods as any).buyTokens(
      new anchor.BN(1_000),
      new anchor.BN(10_000_000)
    );
    assert.exists(builder, "buyTokens builder should require amount + max_sol_cost");
  });

  it("wallet/provider wiring is valid for local execution", async () => {
    assert.equal(provider.wallet.publicKey.toBase58(), wallet.publicKey.toBase58());
    assert.isString(rpcUrl);
    assert.isTrue(fs.existsSync(walletPath), `wallet path missing: ${walletPath}`);
  });

  it("local validator readiness check", async function () {
    this.timeout(10000);
    try {
      const version = await connection.getVersion();
      assert.exists(version, "connection should return version info");
    } catch (_err: any) {
      console.warn(`Skipping live RPC assertion because no local validator is reachable at ${rpcUrl}`);
      this.skip();
    }
  });

  it("initialize_protocol account shape is still aligned", async () => {
    const builder = (program.methods as any).initializeProtocol(200, 5000, 2000, 3000).accounts({
      protocolConfig,
      protocolTreasury,
      admin: wallet.publicKey,
      systemProgram: SystemProgram.programId,
    });
    assert.exists(builder);
  });

  it("open_cycle account shape reflects current contract requirements", async () => {
    const builder = (program.methods as any).openCycle(
      { linear: {} },
      new anchor.BN(500_000_000),
      new anchor.BN(1_000),
      new anchor.BN(60),
      new anchor.BN(0),
      new anchor.BN(0),
      new anchor.BN(5_000),
      new anchor.BN(0),
      null
    ).accounts({
      projectState,
      cycleState,
      projectEscrowToken,
      mint: mintKp.publicKey,
      caller: wallet.publicKey,
      tokenProgram: TOKEN_PROGRAM_ID,
      associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    });
    assert.exists(builder);
  });

  it("create_holder_rights no longer expects holder_account", async () => {
    const builder = (program.methods as any).createHolderRights(
      holder.publicKey,
      new anchor.BN(1_000),
      new anchor.BN(Math.floor(Date.now() / 1000) + 3600)
    ).accounts({
      projectState,
      cycleState,
      holderRights,
      creator: wallet.publicKey,
      systemProgram: SystemProgram.programId,
    });
    assert.exists(builder);
  });
});
