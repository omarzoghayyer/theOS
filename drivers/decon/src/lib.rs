//! theOS DECON/DSIM display-controller driver (Tensor G2 / gs201, Pixel 7 Pro "cheetah")
//!
//! Scaffold only. Base addresses below are VERIFIED against two independent
//! Google source trees (cross-checked, not guessed):
//!   1. arch/arm64/boot/dts/google/gs201-drm-dpu.dtsi
//!      (GrapheneOS-Archive/kernel_gs)
//!   2. samsung/cal_9855/regs-decon.h
//!      (android.googlesource.com/kernel/google-modules/display,
//!       commit b7910bcbbf5d83ad67cb926a056e2728f826781c,
//!       "drm: samsung: add initial updates for GS201")
//!
//! "cal_9855" is the GS201-specific chip-abstraction-layer directory in
//! Google's own tree (cal_9845 is GS101 / Tensor G1). DECON_MAIN and
//! MIPI_DSIM0/1 addresses agree exactly between the device tree and the
//! CAL header, which is why these are marked verified rather than assumed.
//!
//! UPDATE: the GS101 baseline (samsung/cal_9845/regs-decon.h + decon_reg.c)
//! has since been pulled from branch android-gs-pantah-5.10-android13-qpr3
//! ("pantah" = shared Pixel 7/7 Pro platform). GS201's own cal_9855/decon_reg.c
//! includes this GS101 file directly (`#include "../cal_9845/regs-decon.h"`)
//! rather than redefining these registers, so the GS101 baseline register
//! offsets and bit definitions below ARE the real GS201/cheetah values, not
//! guesses. `reset()`, `set_clkgate_disabled()`, `set_enable()`, and
//! `is_running()` are modeled directly on decon_reg_reset(),
//! decon_reg_set_clkgate_mode(), decon_reg_direct_on_off(), and the
//! GLOBAL_CON_RUN_STATUS check in the real driver.
//!
//! STILL NOT verified: whether DECON0_MAIN is reachable via plain MMIO from
//! non-secure (EL1) code, or whether it's gated by the D_TZPC_DISP/D_TZPC_DPU
//! TrustZone protection controllers noted in the base-address comment block
//! in regs-decon.h -- a related GS101 peripheral (power domains) was found to
//! require ARM SMC calls rather than plain reads/writes for some operations.
//! Also still not ported: DSIM (panel interface) and DQE (color/dither path),
//! which this crate doesn't touch yet.

#![cfg_attr(not(test), no_std)]

/// Verified physical base addresses, Tensor G2 (gs201) display subsystem.
pub mod base_addr {
    /// DECON0 (display controller, main register block). Size 0x6000.
    /// Verified: gs201-drm-dpu.dtsi AND cal_9855/regs-decon.h agree.
    pub const DECON0_MAIN: usize = 0x1C24_0000;
    /// DECON window configuration block.
    /// Verified: cal_9855/regs-decon.h (DISP BASE ADDRESS comment block).
    pub const DECON_WIN: usize = 0x1C25_0000;
    pub const DECON_SUB: usize = 0x1C26_0000;
    pub const DECON0_WINCON: usize = 0x1C27_0000;
    pub const DECON1_WINCON: usize = 0x1C28_0000;
    pub const DECON2_WINCON: usize = 0x1C29_0000;
    pub const DQE0: usize = 0x1C2A_0000;
    pub const DQE1: usize = 0x1C2B_0000;

    /// MIPI-DSI, panel interface 0. Verified: dtsi AND regs-decon.h agree.
    pub const MIPI_DSIM0: usize = 0x1C2C_0000;
    pub const MIPI_DSIM1: usize = 0x1C2D_0000;
    pub const MIPI_DCPHY: usize = 0x1C2E_0000;

    /// DPU DMA / layer engines (L0-L5, write-back). Verified: gs201-drm-dpu.dtsi.
    pub const DPU_DMA: usize = 0x1C0B_0000;

    /// Mali-G710 GPU. Verified: gs201-gpu.dtsi ("arm,malit6xx"), size 0x1000000.
    pub const MALI_GPU: usize = 0x2800_0000;
    pub const MALI_GPU_SIZE: usize = 0x0100_0000;
}

/// Register offsets and bit definitions, VERIFIED against the real GS101
/// baseline source: samsung/cal_9845/regs-decon.h and decon_reg.c
/// (android.googlesource.com/kernel/google-modules/display,
///  branch android-gs-pantah-5.10-android13-qpr3 -- "pantah" is the shared
///  Pixel 7 / 7 Pro platform, cheetah is the specific device).
/// GS201's own cal_9855/decon_reg.c includes this GS101 file directly
/// (`#include "../cal_9845/regs-decon.h"`) rather than redefining these
/// registers, which is why the GS101 baseline is the correct source for
/// register-level behavior on the Pixel 7 Pro's Tensor G2.
pub mod decon_regs {
    pub const DECON_VERSION: u32 = 0x0000;
    pub const FRAME_COUNT: u32 = 0x0004;

    pub const GLOBAL_CON: u32 = 0x0020;
    /// Self-clearing soft-reset bit: write 1, hardware clears it back to 0
    /// once reset completes. Real driver polls for this with a 2000us bound
    /// (readl_poll_timeout_atomic) -- see `reset()` for why this crate uses
    /// a bounded iteration count instead, for now.
    pub const GLOBAL_CON_SRESET: u32 = 1 << 28;
    pub const GLOBAL_CON_OPERATION_MODE_CMD_F: u32 = 1 << 8;
    pub const GLOBAL_CON_OPERATION_MODE_VIDEO_F: u32 = 0 << 8;
    pub const GLOBAL_CON_OPERATION_MODE_MASK: u32 = 0x1 << 8;
    pub const GLOBAL_CON_RUN_STATUS: u32 = 1 << 4;
    pub const GLOBAL_CON_DECON_EN: u32 = 1 << 1;
    pub const GLOBAL_CON_DECON_EN_F: u32 = 1 << 0;

    pub const TRIG_CON: u32 = 0x0030;
    pub const SHD_REG_UP_REQ: u32 = 0x0050;
    pub const SHD_REG_UP_REQ_GLOBAL: u32 = 1 << 31;

    /// Clock gating control. Real driver's own bring-up comment: during
    /// initial bring-up, disable all clock gating (keep clocks free-running)
    /// rather than letting hardware dynamically gate them.
    pub const CLOCK_CON: u32 = 0x5000;
    pub const CLOCK_CON_AUTO_CG_MASK: u32 = 0x1_1111;
    pub const CLOCK_CON_QACTIVE_MASK: u32 = (0x1 << 24) | (0x1 << 28);
}

/// Abstraction over a memory-mapped register block, so the driver logic can
/// be exercised on the host (via `MockBus`) without real hardware.
/// Same pattern as theos-driver-cs35l41's `I2cBus` trait.
pub trait MmioBus {
    /// Read a 32-bit register at `offset` bytes from the block's base.
    fn read32(&self, offset: u32) -> u32;
    /// Write a 32-bit register at `offset` bytes from the block's base.
    fn write32(&mut self, offset: u32, value: u32);
}

#[derive(Debug, PartialEq, Eq)]
pub enum DeconError {
    /// Soft reset didn't clear within MAX_RESET_POLL_ITERS iterations.
    /// NOTE: this is an iteration bound, not a real time bound -- see
    /// `reset()` docs.
    ResetTimeout,
}

/// Bounded iteration count for the reset poll, standing in for the real
/// driver's 2000us hardware timeout until a timer abstraction exists.
/// Arbitrary and unverified against real hardware timing -- treat as a
/// placeholder, same as everything else marked NOT yet verified.
const MAX_RESET_POLL_ITERS: u32 = 100_000;

/// DECON0 driver, generic over any `MmioBus` implementation.
pub struct Decon<B: MmioBus> {
    bus: B,
}

impl<B: MmioBus> Decon<B> {
    pub fn new(bus: B) -> Self {
        Self { bus }
    }

    fn write_mask(&mut self, offset: u32, value: u32, mask: u32) {
        let cur = self.bus.read32(offset);
        let new = (cur & !mask) | (value & mask);
        self.bus.write32(offset, new);
    }

    /// Soft-reset DECON0. Mirrors decon_reg_reset() in the real driver:
    /// set GLOBAL_CON_SRESET, then poll until hardware clears it back to 0.
    ///
    /// Uses a bounded iteration count rather than a real elapsed-time bound
    /// -- theos-core's no_std bridge doesn't have a timing abstraction yet
    /// (Phase 3, currently paused). Treat MAX_RESET_POLL_ITERS as a
    /// placeholder count, not a verified real-hardware timeout.
    pub fn reset(&mut self) -> Result<(), DeconError> {
        use decon_regs::*;
        self.write_mask(GLOBAL_CON, !0, GLOBAL_CON_SRESET);
        for _ in 0..MAX_RESET_POLL_ITERS {
            if self.bus.read32(GLOBAL_CON) & GLOBAL_CON_SRESET == 0 {
                return Ok(());
            }
        }
        Err(DeconError::ResetTimeout)
    }

    /// Disable clock gating (keep clocks free-running). Matches the real
    /// driver's own bring-up guidance: do this during initial bring-up,
    /// before relying on dynamic clock gating.
    pub fn set_clkgate_disabled(&mut self) {
        use decon_regs::*;
        let mask = CLOCK_CON_AUTO_CG_MASK | CLOCK_CON_QACTIVE_MASK;
        self.write_mask(CLOCK_CON, 0, mask);
    }

    /// Turn DECON0 on or off. Mirrors decon_reg_direct_on_off() -- sets both
    /// DECON_EN and DECON_EN_F together, matching the real driver.
    pub fn set_enable(&mut self, en: bool) {
        use decon_regs::*;
        let val = if en { !0 } else { 0 };
        let mask = GLOBAL_CON_DECON_EN | GLOBAL_CON_DECON_EN_F;
        self.write_mask(GLOBAL_CON, val, mask);
    }

    /// Real hardware-status check, not a register echo: true once DECON is
    /// actually running (GLOBAL_CON_RUN_STATUS), matching how the real
    /// driver confirms the on/off transition completed.
    pub fn is_running(&self) -> bool {
        use decon_regs::*;
        self.bus.read32(GLOBAL_CON) & GLOBAL_CON_RUN_STATUS != 0
    }

    /// Placeholder read-back of DECON_VERSION. Kept from the original
    /// scaffold as a trivial plumbing check.
    pub fn probe(&self) -> u32 {
        self.bus.read32(decon_regs::DECON_VERSION)
    }

    pub fn into_inner(self) -> B {
        self.bus
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// In-memory mock of a register block for host-side testing.
    struct MockBus {
        regs: HashMap<u32, u32>,
    }

    impl MockBus {
        fn new() -> Self {
            Self { regs: HashMap::new() }
        }
    }

    impl MmioBus for MockBus {
        fn read32(&self, offset: u32) -> u32 {
            *self.regs.get(&offset).unwrap_or(&0)
        }
        fn write32(&mut self, offset: u32, value: u32) {
            self.regs.insert(offset, value);
        }
    }

    #[test]
    fn base_addresses_are_distinct_and_ordered() {
        // Sanity check on the verified address map: DPU block comes before
        // DISP block, matching the "[ DPU BASE ADDRESS ]" /
        // "[ DISP BASE ADDRESS ]" ordering in the source comment block.
        assert!(base_addr::DPU_DMA < base_addr::DECON0_MAIN);
        assert_eq!(base_addr::DECON0_MAIN, 0x1C24_0000);
        assert_eq!(base_addr::MIPI_DSIM0, 0x1C2C_0000);
        assert_eq!(base_addr::MIPI_DSIM1, 0x1C2D_0000);
        assert_eq!(base_addr::MALI_GPU, 0x2800_0000);
    }

    #[test]
    fn mock_bus_round_trips() {
        let mut bus = MockBus::new();
        bus.write32(0x0000, 0xDEAD_BEEF);
        assert_eq!(bus.read32(0x0000), 0xDEAD_BEEF);
        assert_eq!(bus.read32(0x0004), 0); // unwritten register reads as 0
    }

    #[test]
    fn decon_probe_reads_through_mock_bus() {
        let mut bus = MockBus::new();
        bus.write32(0x0000, 0x1234_5678);
        let decon = Decon::new(bus);
        assert_eq!(decon.probe(), 0x1234_5678);
    }

    /// A MockBus variant that simulates real hardware behavior for the
    /// self-clearing SRESET bit: once set, it clears itself after a fixed
    /// number of reads, the way real DECON hardware clears it once reset
    /// completes internally.
    struct SelfClearingResetMock {
        regs: HashMap<u32, u32>,
        reads_until_clear: u32,
    }

    impl SelfClearingResetMock {
        fn new(reads_until_clear: u32) -> Self {
            Self { regs: HashMap::new(), reads_until_clear }
        }
    }

    impl MmioBus for SelfClearingResetMock {
        fn read32(&self, offset: u32) -> u32 {
            let val = *self.regs.get(&offset).unwrap_or(&0);
            if offset == decon_regs::GLOBAL_CON
                && val & decon_regs::GLOBAL_CON_SRESET != 0
                && self.reads_until_clear == 0
            {
                0
            } else {
                val
            }
        }
        fn write32(&mut self, offset: u32, value: u32) {
            self.regs.insert(offset, value);
        }
    }

    #[test]
    fn reset_succeeds_when_hardware_clears_sreset() {
        // Bus that clears SRESET immediately on the first post-write read --
        // simulates hardware finishing reset instantly.
        let bus = SelfClearingResetMock::new(0);
        let mut decon = Decon::new(bus);
        assert_eq!(decon.reset(), Ok(()));
    }

    #[test]
    fn reset_times_out_if_sreset_never_clears() {
        // Plain MockBus never clears any bit on its own, so SRESET stays
        // set forever -- this must hit the iteration bound and report
        // ResetTimeout rather than hang or silently succeed.
        let bus = MockBus::new();
        let mut decon = Decon::new(bus);
        assert_eq!(decon.reset(), Err(DeconError::ResetTimeout));
    }

    #[test]
    fn set_enable_sets_both_en_bits_together() {
        let bus = MockBus::new();
        let mut decon = Decon::new(bus);
        decon.set_enable(true);
        let bus = decon.into_inner();
        let val = bus.read32(decon_regs::GLOBAL_CON);
        assert_ne!(val & decon_regs::GLOBAL_CON_DECON_EN, 0);
        assert_ne!(val & decon_regs::GLOBAL_CON_DECON_EN_F, 0);

        let mut decon = Decon::new(bus);
        decon.set_enable(false);
        let bus = decon.into_inner();
        let val = bus.read32(decon_regs::GLOBAL_CON);
        assert_eq!(val & decon_regs::GLOBAL_CON_DECON_EN, 0);
        assert_eq!(val & decon_regs::GLOBAL_CON_DECON_EN_F, 0);
    }

    #[test]
    fn is_running_reflects_run_status_bit() {
        let bus = MockBus::new();
        let decon = Decon::new(bus);
        assert!(!decon.is_running());

        let mut bus = decon.into_inner();
        bus.write32(decon_regs::GLOBAL_CON, decon_regs::GLOBAL_CON_RUN_STATUS);
        let decon = Decon::new(bus);
        assert!(decon.is_running());
    }

    #[test]
    fn set_clkgate_disabled_clears_gating_bits() {
        let mut bus = MockBus::new();
        // Pre-set the gating bits, as if hardware reset them to some
        // gated-by-default state.
        bus.write32(
            decon_regs::CLOCK_CON,
            decon_regs::CLOCK_CON_AUTO_CG_MASK | decon_regs::CLOCK_CON_QACTIVE_MASK,
        );
        let mut decon = Decon::new(bus);
        decon.set_clkgate_disabled();
        let bus = decon.into_inner();
        assert_eq!(bus.read32(decon_regs::CLOCK_CON), 0);
    }
}
