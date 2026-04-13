#!/bin/bash
set -e
. "$HOME/.nvm/nvm.sh"
. "$HOME/.cargo/env"
export PATH="$HOME/.avm/bin:$HOME/.cargo/bin:$HOME/.nvm/versions/node/v20.20.2/bin:$HOME/.local/share/solana/install/active_release/bin:$PATH"
cd /mnt/c/Users/kelvi/Projects/mammoth/contracts/mammoth_core
anchor test --skip-build 2>&1
