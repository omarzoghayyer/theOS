// audio.rs — theOS Audio Engine
// Microphone capture and speaker playback via ALSA
// Opus codec for compression over satellite links
// Designed for high-latency, lossy satellite conditions

use std::fs;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

// Sample rate — 48kHz is Opus native rate
const SAMPLE_RATE: u32 = 48000;
// Frame size — 20ms at 48kHz = 960 samples
// Longer frames = better compression, more latency
// 20ms is the sweet spot for satellite voice
const FRAME_SIZE: usize = 960;
// Channels — mono for voice calls
const CHANNELS: u16 = 1;

#[derive(Debug, Clone, PartialEq)]
pub enum AudioState {
    Idle,
    Capturing,   // microphone active
    Playing,     // speaker active
    CallActive,  // both mic and speaker
}

#[derive(Debug, Clone)]
pub struct AudioConfig {
    pub sample_rate:   u32,
    pub frame_size:    usize,
    pub channels:      u16,
    pub bitrate_kbps:  u32,   // Opus bitrate — lower for poor satellite links
    pub jitter_buf_ms: u32,   // jitter buffer — larger for high-latency satellite
}

impl AudioConfig {
    pub fn default_satellite() -> Self {
        Self {
            sample_rate:   SAMPLE_RATE,
            frame_size:    FRAME_SIZE,
            channels:      CHANNELS,
            bitrate_kbps:  24,    // 24kbps — good voice quality over satellite
            jitter_buf_ms: 200,   // 200ms jitter buffer for satellite latency
        }
    }

    pub fn low_bandwidth() -> Self {
        Self {
            sample_rate:   SAMPLE_RATE,
            frame_size:    FRAME_SIZE,
            channels:      CHANNELS,
            bitrate_kbps:  8,     // 8kbps — minimum viable voice
            jitter_buf_ms: 400,   // larger buffer for poor links
        }
    }
}

pub struct AudioEngine {
    pub state:   AudioState,
    pub config:  AudioConfig,
    device_name: String,
    running:     Arc<AtomicBool>,
}

impl AudioEngine {
    pub fn new() -> Self {
        let device = Self::detect_audio_device();
        println!("[audio] device: {}", device);
        Self {
            state:       AudioState::Idle,
            config:      AudioConfig::default_satellite(),
            device_name: device,
            running:     Arc::new(AtomicBool::new(false)),
        }
    }

    /// Detect the best audio device for calls
    fn detect_audio_device() -> String {
        // Check ALSA devices
        let alsa_devices = [
            "/dev/snd/pcmC0D0c",  // capture device 0
            "/dev/snd/pcmC1D0c",  // capture device 1
            "/dev/dsp",           // legacy OSS
        ];

        for device in &alsa_devices {
            if std::path::Path::new(device).exists() {
                return device.to_string();
            }
        }

        // Check /proc/asound for ALSA cards
        if let Ok(cards) = fs::read_to_string("/proc/asound/cards") {
            for line in cards.lines() {
                if line.contains("USB") || line.contains("Audio") {
                    println!("[audio] found ALSA card: {}", line.trim());
                }
            }
        }

        // Dev fallback
        "hw:0,0".to_string()
    }

    /// Start capturing from microphone
    pub fn start_capture(&mut self) -> Result<(), String> {
        if self.running.load(Ordering::Relaxed) {
            return Err("already capturing".to_string());
        }

        println!("[audio] starting microphone capture");
        println!("[audio] sample rate: {}Hz", self.config.sample_rate);
        println!("[audio] frame size: {} samples ({}ms)",
            self.config.frame_size,
            self.config.frame_size * 1000 / self.config.sample_rate as usize);
        println!("[audio] bitrate: {}kbps (Opus)", self.config.bitrate_kbps);

        self.running.store(true, Ordering::Relaxed);
        self.state = AudioState::Capturing;

        // Production: open ALSA PCM device, start capture loop
        // let pcm = alsa::PCM::new(&self.device_name, alsa::Direction::Capture, false)?;
        // configure hwparams, start capture thread

        println!("[audio] capture started — waiting for real hardware");
        Ok(())
    }

    /// Start playing to speaker
    pub fn start_playback(&mut self) -> Result<(), String> {
        println!("[audio] starting speaker playback");
        println!("[audio] jitter buffer: {}ms", self.config.jitter_buf_ms);
        self.state = AudioState::Playing;

        // Production: open ALSA PCM playback device, start playback loop
        println!("[audio] playback started — waiting for real hardware");
        Ok(())
    }

    /// Start a full duplex call — mic + speaker simultaneously
    pub fn start_call(&mut self) -> Result<(), String> {
        println!("[audio] starting full duplex call");
        self.start_capture()?;
        self.start_playback()?;
        self.state = AudioState::CallActive;
        println!("[audio] call audio active — {} bitrate, {}ms jitter buf",
            self.config.bitrate_kbps,
            self.config.jitter_buf_ms);
        Ok(())
    }

    /// Stop all audio
    pub fn stop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        self.state = AudioState::Idle;
        println!("[audio] stopped");
    }

    /// Adapt audio quality based on AI agent prediction
    /// Called by the AI executor when link quality degrades
    pub fn set_bitrate(&mut self, kbps: u32) {
        println!("[audio] bitrate adjusted: {} → {}kbps", self.config.bitrate_kbps, kbps);
        self.config.bitrate_kbps = kbps;
        // Production: update Opus encoder bitrate in real time
    }

    /// Increase jitter buffer for high-latency satellite conditions
    pub fn set_jitter_buffer(&mut self, ms: u32) {
        println!("[audio] jitter buffer: {} → {}ms", self.config.jitter_buf_ms, ms);
        self.config.jitter_buf_ms = ms;
    }

    /// Process an incoming RTP audio packet
    /// Decode Opus, add to jitter buffer, play out
    pub fn receive_rtp(&mut self, payload: &[u8], sequence: u16, timestamp: u32) {
        println!("[audio] RTP seq:{} ts:{} len:{} bytes",
            sequence, timestamp, payload.len());
        // Production:
        // 1. Add to jitter buffer (reorder out-of-sequence packets)
        // 2. Decode Opus: opus_decoder.decode(payload) -> PCM samples
        // 3. Write PCM to ALSA playback device
    }

    /// Capture a frame from microphone, encode to Opus, return RTP payload
    pub fn capture_rtp(&mut self) -> Option<Vec<u8>> {
        // Production:
        // 1. Read FRAME_SIZE PCM samples from ALSA capture device
        // 2. Encode with Opus: opus_encoder.encode(pcm) -> compressed bytes
        // 3. Wrap in RTP packet with sequence number and timestamp
        // 4. Return for transmission over satellite link
        None // placeholder until real hardware
    }

    pub fn is_active(&self) -> bool {
        self.state != AudioState::Idle
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        self.stop();
    }
}
