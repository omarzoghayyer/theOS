// enclave.rs — Secure Enclave Detection
// Detects hardware security chip for keystore backing
// Titan M2 (Pixel), Samsung Knox, generic TPM, or software fallback

use std::fs;
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub enum EnclaveType {
    TitanM2,       // Google Pixel 6+ — strongest
    TitanM1,       // Google Pixel 4/5
    SamsungKnox,   // Samsung devices
    QualcommSPU,   // Qualcomm Secure Processing Unit
    GenericTpm,    // Generic TPM 2.0
    Software,      // No hardware enclave — software keystore (dev only)
}

impl EnclaveType {
    pub fn detect() -> Self {
        let cpuinfo = fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
        let combined = cpuinfo.to_lowercase();

        // Check for Titan M2 (Pixel 6+, GS101/GS201)
        if combined.contains("gs201") || combined.contains("gs101") {
            // Pixel 6 = GS101 = Titan M2
            if Path::new("/dev/titan-m2").exists()
                || combined.contains("tensor") {
                return Self::TitanM2;
            }
        }

        // Check for Titan M1 (Pixel 4/5)
        if combined.contains("pixel 4") || combined.contains("pixel 5") {
            return Self::TitanM1;
        }

        // Check for Samsung Knox
        if combined.contains("exynos") || combined.contains("samsung") {
            if Path::new("/dev/kms").exists()
                || Path::new("/sys/class/knox").exists() {
                return Self::SamsungKnox;
            }
        }

        // Check for Qualcomm SPU
        if combined.contains("qualcomm") || combined.contains("snapdragon") {
            return Self::QualcommSPU;
        }

        // Check for generic TPM
        if Path::new("/dev/tpm0").exists() || Path::new("/dev/tpmrm0").exists() {
            return Self::GenericTpm;
        }

        // Software fallback — dev machine or unsupported hardware
        println!("[hal] WARNING: no hardware secure enclave detected — using software keystore");
        Self::Software
    }

    /// Is this a hardware-backed enclave?
    pub fn is_hardware(&self) -> bool {
        !matches!(self, Self::Software)
    }

    /// Security level description
    pub fn security_level(&self) -> &str {
        match self {
            Self::TitanM2     => "Hardware — Titan M2 (highest)",
            Self::TitanM1     => "Hardware — Titan M1",
            Self::SamsungKnox => "Hardware — Samsung Knox",
            Self::QualcommSPU => "Hardware — Qualcomm SPU",
            Self::GenericTpm  => "Hardware — Generic TPM 2.0",
            Self::Software    => "Software — DEV ONLY, not secure",
        }
    }

    /// Keystore storage path based on enclave type
    pub fn keystore_path(&self) -> &str {
        match self {
            Self::TitanM2     => "/dev/titan-m2",
            Self::TitanM1     => "/dev/titan-m",
            Self::SamsungKnox => "/dev/kms",
            Self::QualcommSPU => "/dev/qseecom",
            Self::GenericTpm  => "/dev/tpm0",
            Self::Software    => "/tmp/theos-keystore-sw",
        }
    }
}
