// hal/mod.rs -- Hardware Abstraction Layer
//
// Isolates all hardware-specific code.
// Everything above this layer is hardware-independent.
//
// For Pixel 7 Pro (cheetah):
//   UART: ARM PL011 compatible at 0xFE800000 (placeholder)
//         Real address comes from DTB -- updated in Phase 2
//
// Security: all hardware access is in this module only.
// No other module accesses MMIO registers directly.

pub mod uart;
