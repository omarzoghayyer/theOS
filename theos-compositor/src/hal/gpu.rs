// gpu.rs — GPU Detection and Configuration
// Detects GPU type and configures OpenGL ES pipeline accordingly
// Mali (Google/Samsung), Adreno (Qualcomm), PowerVR (MediaTek)

use std::fs;

#[derive(Debug, Clone, PartialEq)]
pub enum GpuBackend {
    MaliG78,        // Pixel 6 — ARM Mali-G78 MP20
    MaliG710,       // Pixel 7 — ARM Mali-G710
    MaliGeneric,    // Other Mali
    AdrenoGeneric,  // Qualcomm Adreno (OnePlus, Samsung Snapdragon)
    PowerVR,        // MediaTek PowerVR
    SwiftShader,    // Software renderer — dev machine
    Unknown,
}

impl GpuBackend {
    pub fn detect() -> Self {
        // Try to read GPU info from DRM subsystem
        let drm_info = Self::read_drm_info();
        let cpuinfo  = fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
        let combined = format!("{} {}", drm_info, cpuinfo).to_lowercase();

        if combined.contains("mali-g78") || combined.contains("g78") {
            return Self::MaliG78;
        }
        if combined.contains("mali-g710") || combined.contains("g710") {
            return Self::MaliG710;
        }
        if combined.contains("mali") {
            return Self::MaliGeneric;
        }
        if combined.contains("adreno") {
            return Self::AdrenoGeneric;
        }
        if combined.contains("powervr") || combined.contains("imgtech") {
            return Self::PowerVR;
        }

        // Dev machine fallback
        Self::SwiftShader
    }

    fn read_drm_info() -> String {
        // Try various DRM paths
        let paths = [
            "/sys/class/drm/card0/device/uevent",
            "/sys/class/drm/renderD128/device/uevent",
            "/proc/driver/mali/version",
        ];
        for path in &paths {
            if let Ok(content) = fs::read_to_string(path) {
                return content;
            }
        }
        String::new()
    }

    /// OpenGL ES version supported by this GPU
    pub fn gles_version(&self) -> (u32, u32) {
        match self {
            Self::MaliG78      => (3, 2),
            Self::MaliG710     => (3, 2),
            Self::MaliGeneric  => (3, 1),
            Self::AdrenoGeneric => (3, 2),
            Self::PowerVR      => (3, 1),
            Self::SwiftShader  => (3, 1),
            Self::Unknown      => (3, 0),
        }
    }

    /// Optimal render resolution scale for this GPU
    pub fn render_scale(&self) -> f32 {
        match self {
            Self::MaliG78      => 1.0,  // Full res
            Self::MaliG710     => 1.0,  // Full res
            Self::MaliGeneric  => 0.9,  // Slight reduction
            Self::AdrenoGeneric => 1.0, // Full res
            Self::PowerVR      => 0.85, // More conservative
            Self::SwiftShader  => 0.5,  // Software — half res
            Self::Unknown      => 0.75,
        }
    }

    /// Max FPS target for this GPU
    pub fn target_fps(&self) -> u32 {
        match self {
            Self::MaliG78      => 90,
            Self::MaliG710     => 120,
            Self::MaliGeneric  => 60,
            Self::AdrenoGeneric => 90,
            Self::PowerVR      => 60,
            Self::SwiftShader  => 30,
            Self::Unknown      => 60,
        }
    }
}
