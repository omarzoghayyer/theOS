//! Cirrus Logic CS35L41 smart audio amplifier driver.
//!
//! The CS35L41 is the speaker amplifier used on the Google Pixel 7 Pro
//! (codename: cheetah). This crate provides a bus-agnostic driver suitable
//! for use under theOS in `no_std` environments.
//!
//! # Layering
//!
//! - [`bus`] defines the [`I2cBus`] trait that abstracts the I2C transport.
//!   The driver depends only on this trait — never directly on a controller
//!   register map — so the same driver code runs on real hardware and
//!   against the host-side `MockBus` in tests.
//! - [`registers`] holds CS35L41 register address constants.
//!
//! # Status
//!
//! Early architecture-only scaffolding. The driver currently exposes
//! [`Cs35l41::probe`] which reads the DEVID register and verifies it
//! matches the expected CS35L41 chip identifier. Power-up sequencing,
//! ASP/I2S configuration, DSP firmware loading, and the rest of the
//! bring-up sequence will land in subsequent commits.

#![cfg_attr(not(test), no_std)]

pub mod bus;
pub mod registers;

use bus::{BusError, I2cBus};

/// Driver lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverState {
    /// Constructed but no hardware access has been performed.
    Uninit,
    /// `probe()` succeeded — the chip responded with the expected DEVID.
    Probed,
}

/// Errors that can occur during CS35L41 operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// The underlying I2C transport failed.
    Bus(BusError),
    /// The DEVID register read back a value that does not match
    /// [`registers::EXPECTED_DEVID`]. The unexpected value is included
    /// so callers can log it.
    UnexpectedDevId(u32),
}

impl From<BusError> for Error {
    fn from(e: BusError) -> Self {
        Error::Bus(e)
    }
}

/// The CS35L41 driver, parameterized over an [`I2cBus`] implementation.
pub struct Cs35l41<B: I2cBus> {
    bus: B,
    state: DriverState,
}

impl<B: I2cBus> Cs35l41<B> {
    /// Construct a new driver wrapping a bus handle. No hardware access
    /// is performed by this call.
    pub fn new(bus: B) -> Self {
        Self {
            bus,
            state: DriverState::Uninit,
        }
    }

    /// Read the DEVID register and verify it matches the expected CS35L41
    /// chip ID. On success, transitions the driver to
    /// [`DriverState::Probed`].
    ///
    /// This is the first step of any bring-up sequence: confirm we are
    /// actually talking to a CS35L41 before issuing further commands.
    pub fn probe(&mut self) -> Result<(), Error> {
        let id = self.bus.read_register(registers::DEVID)?;
        if id != registers::EXPECTED_DEVID {
            return Err(Error::UnexpectedDevId(id));
        }
        self.state = DriverState::Probed;
        Ok(())
    }

    /// Current lifecycle state.
    pub fn state(&self) -> DriverState {
        self.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::MockBus;

    #[test]
    fn new_driver_is_uninit() {
        let driver = Cs35l41::new(MockBus::new());
        assert_eq!(driver.state(), DriverState::Uninit);
    }

    #[test]
    fn probe_succeeds_when_devid_matches() {
        let mut bus = MockBus::new();
        bus.program_register(registers::DEVID, registers::EXPECTED_DEVID);
        let mut driver = Cs35l41::new(bus);

        assert!(driver.probe().is_ok());
        assert_eq!(driver.state(), DriverState::Probed);
    }

    #[test]
    fn probe_fails_when_devid_mismatches() {
        let mut bus = MockBus::new();
        bus.program_register(registers::DEVID, 0xDEAD_BEEF);
        let mut driver = Cs35l41::new(bus);

        match driver.probe() {
            Err(Error::UnexpectedDevId(0xDEAD_BEEF)) => {}
            other => panic!("expected UnexpectedDevId(0xDEAD_BEEF), got {:?}", other),
        }
        assert_eq!(driver.state(), DriverState::Uninit);
    }

    #[test]
    fn probe_propagates_bus_error() {
        // Empty mock — any read returns BusError::NotResponding.
        let mut driver = Cs35l41::new(MockBus::new());
        match driver.probe() {
            Err(Error::Bus(BusError::NotResponding)) => {}
            other => panic!("expected Bus(NotResponding), got {:?}", other),
        }
        assert_eq!(driver.state(), DriverState::Uninit);
    }
}
