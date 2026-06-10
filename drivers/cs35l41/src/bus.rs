//! I2C bus abstraction.
//!
//! The CS35L41 is addressed with 32-bit register offsets and 32-bit
//! register values. This module defines the trait the driver uses to
//! talk to the chip, along with a host-side mock for tests.
//!
//! The trait operates in register terms (address + value), not raw I2C
//! bytes — the platform's I2C controller is responsible for translating
//! each call into the appropriate transactions on the bus.

/// Errors that can occur on the I2C transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusError {
    /// The chip did not ACK its address or did not respond within the
    /// expected window. On a real bus this typically means the chip is
    /// powered down, the address is wrong, or wiring is broken.
    NotResponding,
    /// A transaction completed but the data was rejected (e.g. CRC,
    /// arbitration loss). Reserved for future bus implementations.
    TransferFailed,
}

/// Abstraction over the I2C transport to the CS35L41.
///
/// Implementations are expected to issue the platform's equivalent of a
/// 32-bit-address, 32-bit-value I2C transaction for each call.
pub trait I2cBus {
    /// Read a 32-bit register.
    fn read_register(&mut self, addr: u32) -> Result<u32, BusError>;

    /// Write a 32-bit register.
    fn write_register(&mut self, addr: u32, value: u32) -> Result<(), BusError>;
}

// ---------------------------------------------------------------------------
// Host-side mock used by unit tests.
// ---------------------------------------------------------------------------

/// A test-only in-memory I2C bus.
///
/// Use [`MockBus::program_register`] to pre-load values that subsequent
/// reads will return. Writes are accepted and recorded in
/// [`MockBus::write_log`], allowing tests to assert what the driver wrote
/// and in what order.
///
/// Reading from an un-programmed address returns
/// [`BusError::NotResponding`] — modelling a chip that is absent or
/// unpowered.
#[cfg(test)]
pub struct MockBus {
    storage: std::collections::HashMap<u32, u32>,
    pub write_log: std::vec::Vec<(u32, u32)>,
}

#[cfg(test)]
impl MockBus {
    /// Construct an empty mock — no registers programmed, no writes recorded.
    pub fn new() -> Self {
        Self {
            storage: std::collections::HashMap::new(),
            write_log: std::vec::Vec::new(),
        }
    }

    /// Pre-load `value` at `addr` so the next read of that address returns it.
    pub fn program_register(&mut self, addr: u32, value: u32) {
        self.storage.insert(addr, value);
    }
}

#[cfg(test)]
impl I2cBus for MockBus {
    fn read_register(&mut self, addr: u32) -> Result<u32, BusError> {
        self.storage.get(&addr).copied().ok_or(BusError::NotResponding)
    }

    fn write_register(&mut self, addr: u32, value: u32) -> Result<(), BusError> {
        self.write_log.push((addr, value));
        self.storage.insert(addr, value);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_unprogrammed_register_returns_not_responding() {
        let mut bus = MockBus::new();
        assert_eq!(bus.read_register(0x1234), Err(BusError::NotResponding));
    }

    #[test]
    fn programmed_register_reads_back_value() {
        let mut bus = MockBus::new();
        bus.program_register(0x1234, 0xCAFE_F00D);
        assert_eq!(bus.read_register(0x1234), Ok(0xCAFE_F00D));
    }

    #[test]
    fn write_is_recorded_in_log_and_visible_to_read() {
        let mut bus = MockBus::new();
        assert!(bus.write_register(0x0040, 0x0000_0001).is_ok());
        assert!(bus.write_register(0x0044, 0x0000_0002).is_ok());

        assert_eq!(
            bus.write_log,
            vec![(0x0040, 0x0000_0001), (0x0044, 0x0000_0002)]
        );
        assert_eq!(bus.read_register(0x0040), Ok(0x0000_0001));
        assert_eq!(bus.read_register(0x0044), Ok(0x0000_0002));
    }
}
