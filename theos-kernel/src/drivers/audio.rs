//! Audio driver wiring.
//!
//! The CS35L41 amplifier driver lives in the `theos-driver-cs35l41`
//! workspace crate. This module will become the kernel-side glue that
//! constructs a `Cs35l41` driver instance over the platform's real I2C
//! controller once that controller exists. Until then, this is
//! intentionally empty — kept only so `drivers/mod.rs` can keep
//! declaring `pub mod audio;` without a missing-file error.
