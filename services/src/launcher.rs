// launcher.rs — theOS App Launcher
// Manages app lifecycle and kiosk mode startup
// Bare OS requirement: every OS needs a process/app manager

use std::error::Error;
use std::process::Command;

pub struct Launcher;

impl Launcher {
    pub fn new() -> Self { Self }

    /// Launch the UI in kiosk mode (runs on real hardware at boot)
    pub fn launch_kiosk(&self) -> Result<(), Box<dyn Error>> {
        println!("Launching theOS UI in kiosk mode...");

        // Try Chromium first (postmarketOS default)
        let chromium = Command::new("chromium-browser")
            .args(&[
                "--kiosk",
                "--no-sandbox",
                "--disable-infobars",
                "--disable-session-crashed-bubble",
                "--disable-restore-session-state",
                "--autoplay-policy=no-user-gesture-required",
                "--check-for-update-interval=31536000",
                "http://localhost:8080",
            ])
            .spawn();

        if chromium.is_err() {
            // Fallback to Firefox
            let _ = Command::new("firefox")
                .args(&["--kiosk", "http://localhost:8080"])
                .spawn()?;
        }

        println!("UI launched at http://localhost:8080");
        Ok(())
    }

    /// Check if we're running on real hardware vs emulator
    pub fn is_real_hardware(&self) -> bool {
        // Check for Pixel 6 specific hardware markers
        std::fs::read_to_string("/proc/cpuinfo")
            .map(|s| s.contains("Google Tensor") || s.contains("GS101"))
            .unwrap_or(false)
    }

    /// Get device info
    pub fn device_info(&self) -> DeviceInfo {
        let model = std::fs::read_to_string("/proc/device-tree/model")
            .unwrap_or_else(|_| "Development Machine".to_string());
        DeviceInfo {
            model: model.trim_end_matches('\0').to_string(),
            is_pixel: model.contains("Pixel"),
            kernel: self.read_kernel_version(),
        }
    }

    fn read_kernel_version(&self) -> String {
        std::fs::read_to_string("/proc/version")
            .map(|s| s.split_whitespace().take(3).collect::<Vec<_>>().join(" "))
            .unwrap_or_else(|_| "Unknown kernel".to_string())
    }
}

#[derive(Debug)]
pub struct DeviceInfo {
    pub model: String,
    pub is_pixel: bool,
    pub kernel: String,
}
