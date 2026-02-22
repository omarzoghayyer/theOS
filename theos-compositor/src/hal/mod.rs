// hal/mod.rs — theOS Hardware Abstraction Layer
// Detects underlying hardware at boot and adapts automatically
// Same binary runs on Pixel 6, OnePlus, Samsung, any ARM device

pub mod detect;
pub mod gpu;
pub mod display;
pub mod enclave;

pub use detect::{DeviceInfo, detect_device};
pub use gpu::GpuBackend;
pub use display::DisplayInfo;
pub use enclave::EnclaveType;

/// Full hardware profile — populated at boot, used by all subsystems
#[derive(Debug, Clone)]
pub struct HardwareProfile {
    pub device:   DeviceInfo,
    pub gpu:      GpuBackend,
    pub display:  DisplayInfo,
    pub enclave:  EnclaveType,
}

impl HardwareProfile {
    /// Detect all hardware at boot — call this once, pass everywhere
    pub fn detect() -> Self {
        println!("[hal] detecting hardware...");
        let device  = detect_device();
        let gpu     = GpuBackend::detect();
        let display = DisplayInfo::detect();
        let enclave = EnclaveType::detect();

        let profile = Self { device, gpu, display, enclave };
        profile.print_summary();
        profile
    }

    fn print_summary(&self) {
        println!("[hal] ┌─ Hardware Profile ───────────────────");
        println!("[hal] │  Device:   {}", self.device.name);
        println!("[hal] │  SoC:      {}", self.device.soc);
        println!("[hal] │  GPU:      {:?}", self.gpu);
        println!("[hal] │  Display:  {}x{} {}Hz",
            self.display.width, self.display.height, self.display.refresh_hz);
        println!("[hal] │  Enclave:  {:?}", self.enclave);
        println!("[hal] └───────────────────────────────────────");
    }
}
