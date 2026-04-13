# Mammoth

> A Rights-Based, Cycle-Driven Token Issuance Framework for Solana

Mammoth enables builders to raise capital multiple times without destroying trust, price, or upside asymmetry. It occupies the gap between meme launchpads (single event, high excitement) and DAO funding platforms (repeatable but kills speculation).

---

## Architecture Overview

```
mammoth/
├── contracts/           # Anchor (Solana) smart contracts
│   ├── mammoth_token/   # Contract 1: SPL Token (fixed + elastic supply)
│   ├── cycle_manager/   # Contract 2: Cycle state, allocation, bonding curve
│   ├── rights/          # Contract 3: Snapshot + pro-rata rights issuance
│   └── treasury/        # Contract 4: Deterministic treasury routing
├── frontend/            # Next.js dApp
│   ├── src/
│   │   ├── app/         # Next.js App Router
│   │   ├── components/
│   │   └── lib/
│   └── package.json
└── docs/                # Architecture decisions and specs
    ├── contract-1-spl-token.md
    ├── contract-2-cycle-manager.md
    └── ui-map.md
```

---

## Build Order

1. **Contract 1: mammoth_token** — SPL token, fixed supply, genesis allocation, 2% protocol stake
2. **Contract 2: cycle_manager** — receives mint authority, manages cycle state and bonding curves
3. **Contract 3: rights** — snapshots holders, issues pro-rata cycle participation rights
4. **Contract 4: treasury** — routes cycle proceeds per deterministic split
5. **Frontend** — Next.js + Wallet Adapter + Jupiter SDK

---

## Prerequisites

### Install Rust
```bash
# Linux/macOS/WSL
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env
```

### Install Solana CLI
```bash
# Linux/macOS/WSL
sh -c "$(curl -sSfL https://release.anza.xyz/stable/install)"
# Add to PATH: export PATH="$HOME/.local/share/solana/install/active_release/bin:$PATH"
solana --version
```

### Install Anchor CLI
```bash
# Requires: Rust, Solana CLI, Node.js 18+, yarn
cargo install --git https://github.com/coral-xyz/anchor avm --locked --force
avm install latest
avm use latest
anchor --version
```

### Install Node.js dependencies
```bash
npm install -g yarn
```

---

## Initialize Anchor Project (run once toolchain is installed)

```bash
cd contracts/
anchor init mammoth-contracts --no-git
cd mammoth-contracts/
anchor build
anchor test
```

---

## Tech Stack

| Layer | Technology |
|-------|-----------|
| Blockchain | Solana |
| Smart Contracts | Anchor (Rust) |
| Token Standard | SPL Token |
| Metadata | Metaplex Token Metadata Program |
| Frontend | Next.js + React |
| Wallet | Solana Wallet Adapter |
| DEX | Jupiter SDK (secondary trading) |
| UI Scaffolding | Loveable AI |

---

## Design Constraints (Non-Negotiable)

- Elastic supply + rights-based issuance are **mandatory together**
- Cycle parameters are **immutable once open** — enforced at contract level
- Hard-cap transition must be **on-chain and provably irreversible**
- 2% protocol fee enforced at contract or interface level
- No governance, no admin keys that break immutability promises
- Freeze authority **null at genesis**, always

---

## Key Links

- Whitepaper: `docs/` directory
- UI Wireframes: `docs/ui-map.md`
- Contract specs: `docs/contract-*.md`

---

*Mammoth is a framework, not a curator. Bad projects fail cleanly and early.*
