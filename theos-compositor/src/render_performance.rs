// render_performance.rs — Frame timing, profiling, and optimization
//
// Measures frame render time, detects jank, implements frame skipping,
// and profiles which surfaces consume the most time.

use std::time::{Duration, Instant};
use std::collections::VecDeque;

/// Frame timing metrics (60 FPS = 16.7ms per frame)
pub const TARGET_FRAME_TIME_MS: f32 = 16.667; // 1000.0 / 60.0
pub const FRAME_TIME_BUDGET_MS: f32 = 16.667;

/// Performance metrics for a single frame
#[derive(Debug, Clone)]
pub struct FrameMetrics {
    pub frame_num: u64,
    pub start_time: Instant,
    pub render_time_ms: f32,
    pub is_jank: bool,  // true if render_time > budget
    pub surface_times: Vec<(String, f32)>, // surface name, render time
}

impl FrameMetrics {
    pub fn new(frame_num: u64) -> Self {
        Self {
            frame_num,
            start_time: Instant::now(),
            render_time_ms: 0.0,
            is_jank: false,
            surface_times: Vec::new(),
        }
    }

    pub fn finish(&mut self) {
        self.render_time_ms = self.start_time.elapsed().as_secs_f32() * 1000.0;
        self.is_jank = self.render_time_ms > FRAME_TIME_BUDGET_MS;
    }

    pub fn add_surface_time(&mut self, name: String, time_ms: f32) {
        self.surface_times.push((name, time_ms));
    }
}

/// Frame timing profiler (keeps last 60 frames for analysis)
pub struct FrameProfiler {
    frames: VecDeque<FrameMetrics>,
    max_frames: usize,
    jank_count: u64,
    frame_counter: u64,
}

impl FrameProfiler {
    pub fn new() -> Self {
        Self {
            frames: VecDeque::new(),
            max_frames: 60,
            jank_count: 0,
            frame_counter: 0,
        }
    }

    pub fn start_frame(&mut self) -> FrameMetrics {
        FrameMetrics::new(self.frame_counter)
    }

    pub fn end_frame(&mut self, mut metrics: FrameMetrics) {
        metrics.finish();

        if metrics.is_jank {
            self.jank_count += 1;
        }

        if self.frames.len() >= self.max_frames {
            self.frames.pop_front();
        }

        self.frames.push_back(metrics);
        self.frame_counter += 1;
    }

    /// Average frame time in ms (last 60 frames)
    pub fn avg_frame_time_ms(&self) -> f32 {
        if self.frames.is_empty() {
            return 0.0;
        }
        let sum: f32 = self.frames.iter().map(|f| f.render_time_ms).sum();
        sum / self.frames.len() as f32
    }

    /// Jank percentage (frames > 16.7ms)
    pub fn jank_percentage(&self) -> f32 {
        if self.frames.is_empty() {
            return 0.0;
        }
        let jank_count = self.frames.iter().filter(|f| f.is_jank).count();
        (jank_count as f32 / self.frames.len() as f32) * 100.0
    }

    /// Slowest surface (cumulative across 60 frames)
    pub fn slowest_surface(&self) -> Option<(String, f32)> {
        let mut surface_totals: std::collections::HashMap<String, f32> = std::collections::HashMap::new();

        for frame in self.frames.iter() {
            for (name, time) in &frame.surface_times {
                *surface_totals.entry(name.clone()).or_insert(0.0) += time;
            }
        }

        surface_totals
            .into_iter()
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
    }

    /// Check if FPS is stable (< 5% jank)
    pub fn is_60fps_stable(&self) -> bool {
        self.jank_percentage() < 5.0
    }
}

/// Adaptive frame skipping (drop frames if render time > 20ms)
pub struct FrameSkipper {
    skip_threshold_ms: f32,
    skip_count: u32,
}

impl FrameSkipper {
    pub fn new() -> Self {
        Self {
            skip_threshold_ms: 20.0, // skip if render takes > 20ms
            skip_count: 0,
        }
    }

    pub fn should_skip(&mut self, render_time_ms: f32) -> bool {
        if render_time_ms > self.skip_threshold_ms {
            self.skip_count += 1;
            true
        } else {
            false
        }
    }

    pub fn skip_count(&self) -> u32 {
        self.skip_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frame_metrics_creation() {
        let metrics = FrameMetrics::new(0);
        assert_eq!(metrics.frame_num, 0);
        assert!(!metrics.is_jank);
    }

    #[test]
    fn test_frame_metrics_jank_detection() {
        let mut metrics = FrameMetrics::new(0);
        std::thread::sleep(Duration::from_millis(20));
        metrics.finish();
        assert!(metrics.is_jank); // 20ms > 16.7ms budget
    }

    #[test]
    fn test_profiler_jank_percentage() {
        let mut profiler = FrameProfiler::new();

        for i in 0..10 {
            let mut metrics = FrameMetrics::new(i as u64);
            metrics.render_time_ms = if i % 2 == 0 { 10.0 } else { 20.0 };
            metrics.is_jank = metrics.render_time_ms > FRAME_TIME_BUDGET_MS;
            profiler.end_frame(metrics);
        }

        let jank_pct = profiler.jank_percentage();
        assert!(jank_pct > 0.0 && jank_pct <= 100.0);
    }

    #[test]
    fn test_profiler_avg_frame_time() {
        let mut profiler = FrameProfiler::new();

        for i in 0..5 {
            let mut metrics = FrameMetrics::new(i as u64);
            metrics.render_time_ms = 15.0;
            profiler.end_frame(metrics);
        }

        assert!((profiler.avg_frame_time_ms() - 15.0).abs() < 0.1);
    }

    #[test]
    fn test_frame_skipper() {
        let mut skipper = FrameSkipper::new();

        assert!(!skipper.should_skip(10.0)); // within budget
        assert!(skipper.should_skip(25.0)); // over threshold

        assert_eq!(skipper.skip_count(), 1);
    }

    #[test]
    fn test_is_60fps_stable() {
        let mut profiler = FrameProfiler::new();

        // Add frames with minimal jank
        for i in 0..20 {
            let mut metrics = FrameMetrics::new(i as u64);
            metrics.render_time_ms = 16.0; // all fast frames
            profiler.end_frame(metrics);
        }

        assert!(profiler.is_60fps_stable());
    }

    #[test]
    fn test_slowest_surface() {
        let mut profiler = FrameProfiler::new();

        let mut metrics = FrameMetrics::new(0);
        metrics.add_surface_time("call_ui".to_string(), 5.0);
        metrics.add_surface_time("compositor".to_string(), 8.0);
        profiler.end_frame(metrics);

        if let Some((name, _time)) = profiler.slowest_surface() {
            assert_eq!(name, "compositor");
        }
    }
}
