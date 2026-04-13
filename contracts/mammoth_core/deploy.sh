#!/bin/bash
source /home/kelvinsinferno/.nvm/nvm.sh
nvm use --lts
export PATH="$HOME/.cargo/bin:$PATH"
export PATH="$HOME/.local/share/solana/install/active_release/bin:$PATH"
echo "=== Solana Config ==="
solana config get
echo ""
echo "=== Devnet Balance ==="
solana balance --url devnet
echo ""
echo "=== Deploying to Devnet ==="
cd /mnt/c/Users/kelvi/Projects/mammoth/contracts/mammoth_core
anchor deploy --provider.cluster devnet 2>&1
