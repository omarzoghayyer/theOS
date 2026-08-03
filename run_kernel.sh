#!/bin/bash
set -e
KERNEL="target/aarch64-unknown-none/debug/theos-kernel"
DTB="qemu.dtb"
cargo build -p theos-kernel --features qemu
qemu-system-aarch64 \
    -M virt -m 2G -cpu cortex-a72 \
    -nographic \
    -kernel "$KERNEL" -dtb "$DTB" 2>&1 | stdbuf -oL head -100
