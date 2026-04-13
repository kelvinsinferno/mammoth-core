#!/usr/bin/env bash
export PATH="/home/kelvinsinferno/.cargo/bin:/home/kelvinsinferno/.local/share/solana/install/active_release/bin:$PATH"
cd /mnt/c/Users/kelvi/Projects/mammoth/contracts/mammoth_core/programs/mammoth_core
cargo check 2>&1 > /tmp/check_full.txt
echo "EXITCODE:$?" >> /tmp/check_full.txt
