#!/bin/bash
KERNEL="target/aarch64-unknown-none/debug/theos-kernel"
DTB="qemu.dtb"
qemu-system-aarch64 \
    -M virt -m 2G -cpu cortex-a72 \
    -nographic -serial stdio \
    -kernel "$KERNEL" -dtb "$DTB" 2>&1 | head -100
