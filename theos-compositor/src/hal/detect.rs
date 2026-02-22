// detect.rs — Device Detection
// Reads CPU info to identify the device and SoC
// Used to load the right drivers and configure hardware features

use std::fs;

#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub name:         String,
    pub manufacturer: String,
    pub soc:          String,
    pub arch:         Arch,
    pub ram_mb:       u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Arch {
    Aarch64,  // ARM64 — all modern Android phones
    X86_64,   // Dev machine / emulator
    Unknown,
}

pub fn detect_device() -> DeviceInfo {
    let cpuinfo = fs::read_to_string("/proc/cpuinfo")
        .unwrap_or_default();
    let hardware = fs::read_to_string("/proc/device-tree/compatible")
        .unwrap_or_default();
    let model = fs::read_to_string("/proc/device-tree/model")
        .unwrap_or_default();

    // Detect SoC from cpuinfo Hardware field
    let soc = detect_soc(&cpuinfo, &hardware);

    // Detect manufacturer and device name
    let (name, manufacturer) = detect_device_name(&cpuinfo, &model, &soc);

    // Detect architecture
    let arch = detect_arch(&cpuinfo);

    // Read RAM from /proc/meminfo
    let ram_mb = detect_ram();

    DeviceInfo { name, manufacturer, soc, arch, ram_mb }
}

fn detect_soc(cpuinfo: &str, hardware: &str) -> String {
    // Check Hardware field in cpuinfo
    for line in cpuinfo.lines() {
        if line.starts_with("Hardware") {
            let val = line.split(':').nth(1).unwrap_or("").trim().to_string();
            if !val.is_empty() { return val; }
        }
    }

    // Check device tree compatible string
    let combined = format!("{} {}", cpuinfo, hardware).to_lowercase();

    if combined.contains("gs101") || combined.contains("tensor") {
        return "Google Tensor GS101".to_string();
    }
    if combined.contains("gs201") {
        return "Google Tensor G2 GS201".to_string();
    }
    if combined.contains("sm8") || combined.contains("snapdragon 8") {
        return "Qualcomm Snapdragon 8".to_string();
    }
    if combined.contains("sm7") || combined.contains("snapdragon 7") {
        return "Qualcomm Snapdragon 7".to_string();
    }
    if combined.contains("exynos") {
        return "Samsung Exynos".to_string();
    }
    if combined.contains("dimensity") || combined.contains("mt68") {
        return "MediaTek Dimensity".to_string();
    }
    if combined.contains("kirin") {
        return "HiSilicon Kirin".to_string();
    }

    // Dev machine
    if combined.contains("x86") || combined.contains("intel") || combined.contains("amd") {
        return "x86_64 (Dev Machine)".to_string();
    }

    "Unknown SoC".to_string()
}

fn detect_device_name(cpuinfo: &str, model: &str, soc: &str) -> (String, String) {
    let combined = format!("{} {}", cpuinfo, model).to_lowercase();

    // Google Pixel devices
    if combined.contains("pixel 6 pro") {
        return ("Pixel 6 Pro".to_string(), "Google".to_string());
    }
    if combined.contains("pixel 6a") {
        return ("Pixel 6a".to_string(), "Google".to_string());
    }
    if combined.contains("pixel 6") {
        return ("Pixel 6".to_string(), "Google".to_string());
    }
    if combined.contains("pixel 7") {
        return ("Pixel 7".to_string(), "Google".to_string());
    }
    if combined.contains("pixel 8") {
        return ("Pixel 8".to_string(), "Google".to_string());
    }

    // OnePlus
    if combined.contains("oneplus") {
        let model_name = if combined.contains("oneplus 12") { "OnePlus 12" }
            else if combined.contains("oneplus 11") { "OnePlus 11" }
            else { "OnePlus Device" };
        return (model_name.to_string(), "OnePlus".to_string());
    }

    // Samsung
    if combined.contains("samsung") {
        let model_name = if combined.contains("s24") { "Galaxy S24" }
            else if combined.contains("s23") { "Galaxy S23" }
            else if combined.contains("s22") { "Galaxy S22" }
            else { "Samsung Galaxy" };
        return (model_name.to_string(), "Samsung".to_string());
    }

    // Huawei
    if combined.contains("huawei") || combined.contains("kirin") {
        return ("Huawei Device".to_string(), "Huawei".to_string());
    }

    // Xiaomi
    if combined.contains("xiaomi") || combined.contains("redmi") {
        return ("Xiaomi Device".to_string(), "Xiaomi".to_string());
    }

    // Dev machine
    if soc.contains("x86") || soc.contains("Dev Machine") {
        return ("Dev Machine".to_string(), "Generic".to_string());
    }

    ("Unknown ARM Device".to_string(), "Unknown".to_string())
}

fn detect_arch(cpuinfo: &str) -> Arch {
    let lower = cpuinfo.to_lowercase();
    if lower.contains("aarch64") || lower.contains("armv8") {
        return Arch::Aarch64;
    }
    if lower.contains("x86_64") || lower.contains("intel") || lower.contains("amd") {
        return Arch::X86_64;
    }
    // Check uname
    if let Ok(machine) = fs::read_to_string("/proc/sys/kernel/osrelease") {
        if machine.contains("aarch64") { return Arch::Aarch64; }
    }
    Arch::Unknown
}

fn detect_ram() -> u32 {
    if let Ok(meminfo) = fs::read_to_string("/proc/meminfo") {
        for line in meminfo.lines() {
            if line.starts_with("MemTotal:") {
                let kb: u32 = line.split_whitespace()
                    .nth(1)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                return kb / 1024;
            }
        }
    }
    0
}
