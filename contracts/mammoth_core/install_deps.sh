#!/bin/bash
source /home/kelvinsinferno/.nvm/nvm.sh
nvm use --lts
export PATH="$HOME/.cargo/bin:$PATH"
cd /mnt/c/Users/kelvi/Projects/mammoth/contracts/mammoth_core
yarn add @solana/spl-token 2>&1
