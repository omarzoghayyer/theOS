//! CS35L41 register address constants.
//!
//! # Provenance
//!
//! These values are intended to come from the Cirrus Logic CS35L41
//! datasheet's register summary. They are **placeholders pending
//! verification** against the datasheet — the shape of the constants
//! (32-bit addresses, 32-bit values) matches the real device, but the
//! exact numeric values below should be confirmed before any real
//! hardware operation.
//!
//! Subsequent commits will replace each constant with the datasheet
//! value and cite the datasheet section.

/// Device ID register. Reading it returns the fixed chip identifier
/// (see [`EXPECTED_DEVID`]).
///
/// Placeholder: confirm address from datasheet.
pub const DEVID: u32 = 0x0000_0000;

/// Expected value of [`DEVID`] for a CS35L41.
///
/// Placeholder: confirm exact value from datasheet. Not yet verified.
pub const EXPECTED_DEVID: u32 = 0x0003_5A40;

/// Revision ID register. Hardware revision identifier.
///
/// Placeholder: confirm address from datasheet.
pub const REVID: u32 = 0x0000_0004;
