// display_backend.rs -- theOS DisplayBackend Trait
//
// Decouples the compositor from any specific display implementation.
// The compositor calls DisplayBackend methods -- it doesn't care whether
// it's talking to real DRM/KMS hardware or a test stub.
//
// Implementations:
//   DrmBackend   -- DRM/KMS/GBM/EGL on PostmarketOS (OnePlus 6 SDM845)
//                   Lives in drm_backend.rs. Wraps via DrmDisplayBackend.
//   DemoBackend  -- Zero-dep stub. Runs on Windows/CI/any platform.
//                   Used for compositor logic tests without a display.
//
// Future implementations (not built yet):
//   AospBackend  -- Android gralloc + SurfaceFlinger for AOSP port.
//                   Adding it requires only a new impl, no compositor changes.
//   WaylandBackend -- Nested Wayland for desktop testing.
//
// Architecture:
//   Compositor owns a Box<dyn DisplayBackend>.
//   On PostmarketOS: Box::new(DrmDisplayBackend::new())
//   In tests:        Box::new(DemoBackend::new())
//   Future AOSP:     Box::new(AospBackend::new())
//
// Security assumptions (flag for audit):
//   None -- this is pure display routing, no crypto or auth.

// -- DisplayBackend trait -----------------------------------------------------

/// The interface every display backend must implement.
/// The compositor only calls these methods -- never touches hardware directly.
pub trait DisplayBackend: Send {
    /// Called at the start of each frame.
    /// Returns true if rendering should proceed.
    /// Returns false if a flip is pending (caller should skip this frame).
    fn begin_frame(&mut self) -> bool;

    /// Called after all GLES draw calls for this frame are complete.
    /// Triggers eglSwapBuffers -> page flip on hardware backends.
    /// Returns Err if the flip fails (unrecoverable -- backend enters Error state).
    fn end_frame(&mut self) -> Result<(), DisplayError>;

    /// Called by the DRM vblank event handler after a page flip completes.
    /// Transitions FlipPending -> Active. No-op on DemoBackend.
    fn on_vblank(&mut self);

    /// Current display resolution in pixels (width, height).
    fn resolution(&self) -> (u32, u32);

    /// Duration of one frame in milliseconds at the display refresh rate.
    /// 60Hz -> 16ms, 90Hz -> 11ms, 120Hz -> 8ms.
    fn frame_duration_ms(&self) -> u64;

    /// True if the backend is ready to render (Active or Demo state).
    fn is_active(&self) -> bool;

    /// True if running without real hardware (DemoBackend or drm demo mode).
    fn is_demo(&self) -> bool;

    /// Human-readable backend name for logging.
    fn name(&self) -> &str;

    /// Total frames rendered since backend init.
    fn frame_count(&self) -> u64;
}

// -- DisplayError -------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum DisplayError {
    /// Page flip failed -- display pipeline stalled.
    PageFlipFailed,
    /// Backend is in an unrecoverable error state.
    BackendError(String),
    /// Frame skipped -- flip still pending from previous frame.
    FlipPending,
}

impl std::fmt::Display for DisplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            DisplayError::PageFlipFailed     => write!(f, "page flip failed"),
            DisplayError::BackendError(e)    => write!(f, "backend error: {}", e),
            DisplayError::FlipPending        => write!(f, "flip pending -- frame skipped"),
        }
    }
}

// -- DemoBackend --------------------------------------------------------------

/// Zero-dependency display backend for testing and CI.
/// Runs on Windows, macOS, Linux -- no DRM device needed.
/// Simulates 60Hz frame timing with no actual display output.
pub struct DemoBackend {
    frame_count:  u64,
    width:        u32,
    height:       u32,
    refresh_hz:   u32,
    flip_pending: bool,
}

impl DemoBackend {
    /// Create a demo backend with OnePlus 6 resolution (1080x2280 @ 60Hz).
    pub fn new() -> Self {
        println!("[display] DemoBackend initialized -- 1080x2280@60Hz (no hardware)");
        Self {
            frame_count:  0,
            width:        1080,
            height:       2280,
            refresh_hz:   60,
            flip_pending: false,
        }
    }

    /// Create a demo backend with custom resolution (for desktop testing).
    pub fn with_resolution(width: u32, height: u32, refresh_hz: u32) -> Self {
        println!("[display] DemoBackend initialized -- {}x{}@{}Hz (no hardware)", width, height, refresh_hz);
        Self { frame_count: 0, width, height, refresh_hz, flip_pending: false }
    }
}

impl Default for DemoBackend {
    fn default() -> Self { Self::new() }
}

impl DisplayBackend for DemoBackend {
    fn begin_frame(&mut self) -> bool {
        if self.flip_pending {
            // Simulate vblank completing immediately in demo mode
            self.flip_pending = false;
        }
        true
    }

    fn end_frame(&mut self) -> Result<(), DisplayError> {
        self.frame_count += 1;
        self.flip_pending = true;
        if self.frame_count % 300 == 0 {
            println!("[display] demo frame {} ({}fps sim)", self.frame_count, self.refresh_hz);
        }
        Ok(())
    }

    fn on_vblank(&mut self) {
        // In demo mode, vblank is simulated in begin_frame
        self.flip_pending = false;
    }

    fn resolution(&self) -> (u32, u32) { (self.width, self.height) }

    fn frame_duration_ms(&self) -> u64 { 1000 / self.refresh_hz as u64 }

    fn is_active(&self) -> bool { true }

    fn is_demo(&self) -> bool { true }

    fn name(&self) -> &str { "DemoBackend" }

    fn frame_count(&self) -> u64 { self.frame_count }
}

// -- DrmDisplayBackend --------------------------------------------------------
//
// Wraps DrmBackend from drm_backend.rs and implements DisplayBackend.
// Only available when the compositor feature is enabled (Linux build).
//
// On PostmarketOS (OnePlus 6):
//   DrmBackend::new() auto-detects /dev/dri/card0,
//   finds the connected display, initializes GBM + EGL,
//   and returns a hardware-ready backend.

#[cfg(feature = "compositor")]
pub mod drm {
    use super::*;
    use crate::drm_backend::DrmBackend;

    pub struct DrmDisplayBackend {
        inner: DrmBackend,
    }

    impl DrmDisplayBackend {
        /// Auto-detect DRM device and initialize hardware.
        /// Falls back to DrmBackend demo mode if no device found.
        pub fn new() -> Self {
            println!("[display] DrmDisplayBackend initializing...");
            Self { inner: DrmBackend::new() }
        }
    }

    impl DisplayBackend for DrmDisplayBackend {
        fn begin_frame(&mut self) -> bool {
            self.inner.begin_frame()
        }

        fn end_frame(&mut self) -> Result<(), DisplayError> {
            self.inner.end_frame()
                .map_err(|e| DisplayError::BackendError(e.to_string()))
        }

        fn on_vblank(&mut self) {
            self.inner.on_vblank()
        }

        fn resolution(&self) -> (u32, u32) {
            self.inner.resolution()
        }

        fn frame_duration_ms(&self) -> u64 {
            self.inner.frame_duration_ms()
        }

        fn is_active(&self) -> bool {
            self.inner.is_active()
        }

        fn is_demo(&self) -> bool {
            self.inner.is_demo()
        }

        fn name(&self) -> &str {
            if self.inner.is_demo() { "DrmBackend(demo)" } else { "DrmBackend(hw)" }
        }

        fn frame_count(&self) -> u64 {
            self.inner.frame_count
        }
    }
}

// -- BackendBuilder -----------------------------------------------------------

/// Select the best available backend at runtime.
/// Called once at compositor startup.
///
/// Priority:
///   1. DrmDisplayBackend -- if compositor feature enabled and /dev/dri/card0 exists
///   2. DemoBackend       -- fallback for CI, Windows dev, or missing hardware
pub fn build_backend() -> Box<dyn DisplayBackend> {
    #[cfg(feature = "compositor")]
    {
        println!("[display] compositor feature enabled -- trying DRM backend");
        return Box::new(drm::DrmDisplayBackend::new());
    }
    #[cfg(not(feature = "compositor"))]
    {
        println!("[display] compositor feature disabled -- using DemoBackend");
        Box::new(DemoBackend::new())
    }
}

// -- Tests --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn demo() -> DemoBackend { DemoBackend::new() }
    fn demo_hd() -> DemoBackend { DemoBackend::with_resolution(1920, 1080, 60) }

    // DemoBackend construction

    #[test]
    fn test_demo_default_resolution() {
        let b = demo();
        assert_eq!(b.resolution(), (1080, 2280));
    }

    #[test]
    fn test_demo_custom_resolution() {
        let b = demo_hd();
        assert_eq!(b.resolution(), (1920, 1080));
    }

    #[test]
    fn test_demo_refresh_rate() {
        let b = demo();
        assert_eq!(b.frame_duration_ms(), 16); // 1000/60
    }

    #[test]
    fn test_demo_90hz() {
        let b = DemoBackend::with_resolution(1080, 2280, 90);
        assert_eq!(b.frame_duration_ms(), 11); // 1000/90
    }

    #[test]
    fn test_demo_is_active() {
        assert!(demo().is_active());
    }

    #[test]
    fn test_demo_is_demo() {
        assert!(demo().is_demo());
    }

    #[test]
    fn test_demo_name() {
        assert_eq!(demo().name(), "DemoBackend");
    }

    // Frame lifecycle

    #[test]
    fn test_demo_begin_frame_returns_true() {
        let mut b = demo();
        assert!(b.begin_frame());
    }

    #[test]
    fn test_demo_end_frame_ok() {
        let mut b = demo();
        b.begin_frame();
        assert!(b.end_frame().is_ok());
    }

    #[test]
    fn test_demo_frame_count_increments() {
        let mut b = demo();
        for _ in 0..5 {
            b.begin_frame();
            b.end_frame().unwrap();
        }
        assert_eq!(b.frame_count(), 5);
    }

    #[test]
    fn test_demo_frame_count_starts_zero() {
        assert_eq!(demo().frame_count(), 0);
    }

    #[test]
    fn test_demo_begin_after_end_returns_true() {
        let mut b = demo();
        b.begin_frame();
        b.end_frame().unwrap();
        // After end_frame, flip_pending=true
        // begin_frame should clear it and return true
        assert!(b.begin_frame());
    }

    #[test]
    fn test_demo_on_vblank_clears_flip() {
        let mut b = demo();
        b.begin_frame();
        b.end_frame().unwrap();
        b.on_vblank();
        assert!(!b.flip_pending);
    }

    // trait object usage

    #[test]
    fn test_trait_object_demo() {
        let mut b: Box<dyn DisplayBackend> = Box::new(DemoBackend::new());
        assert!(b.begin_frame());
        assert!(b.end_frame().is_ok());
        assert_eq!(b.frame_count(), 1);
        assert!(b.is_demo());
        assert!(b.is_active());
    }

    #[test]
    fn test_trait_object_multiple_frames() {
        let mut b: Box<dyn DisplayBackend> = Box::new(DemoBackend::new());
        for _ in 0..10 {
            b.begin_frame();
            b.end_frame().unwrap();
        }
        assert_eq!(b.frame_count(), 10);
    }

    #[test]
    fn test_display_error_display() {
        assert!(!DisplayError::PageFlipFailed.to_string().is_empty());
        assert!(!DisplayError::FlipPending.to_string().is_empty());
        assert!(!DisplayError::BackendError("test".to_string()).to_string().is_empty());
    }

    // build_backend returns usable backend

    #[test]
    fn test_build_backend_returns_active() {
        let b = build_backend();
        assert!(b.is_active());
    }

    #[test]
    fn test_build_backend_demo_mode_without_compositor_feature() {
        // Without compositor feature, must be demo
        #[cfg(not(feature = "compositor"))]
        { assert!(build_backend().is_demo()); }
        #[cfg(feature = "compositor")]
        { let _ = build_backend(); } // hardware backend -- is_demo depends on device
    }

    #[test]
    fn test_build_backend_frame_lifecycle() {
        let mut b = build_backend();
        assert!(b.begin_frame());
        assert!(b.end_frame().is_ok());
        assert_eq!(b.frame_count(), 1);
    }

    // Resolution helpers

    #[test]
    fn test_resolution_width_height() {
        let b = demo();
        let (w, h) = b.resolution();
        assert_eq!(w, 1080);
        assert_eq!(h, 2280);
    }

    #[test]
    fn test_frame_duration_60hz() {
        assert_eq!(DemoBackend::with_resolution(1080,2280,60).frame_duration_ms(), 16);
    }

    #[test]
    fn test_frame_duration_120hz() {
        assert_eq!(DemoBackend::with_resolution(1080,2280,120).frame_duration_ms(), 8);
    }
}
