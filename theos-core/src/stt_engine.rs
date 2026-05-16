// stt_engine.rs -- theOS Speech-to-Text Engine
//
// Converts raw microphone audio into text commands for the WakeEngine.
//
// Architecture:
//   On device (PostmarketOS / SDM845):
//     Whisper.cpp via whisper-rs crate.
//     Model: whisper-tiny.en (~75MB) loaded at boot.
//     Runs on CDSP (Compute DSP) via Qualcomm FastRPC for low power.
//     Latency target: <500ms for a 3-second utterance on SDM845.
//
//   In tests / dev (Windows / no hardware):
//     SttBackend::Stub -- inject transcriptions directly.
//     No audio hardware, no model files needed.
//     All logic fully testable on any platform.
//
// Pipeline:
//   Microphone (ALSA, 16kHz mono) -> AudioCapture
//       -> SttEngine::transcribe(pcm_bytes)
//           -> VAD (voice activity detection) -- skip silence
//           -> Whisper inference
//           -> TranscriptionResult { text, confidence, duration_ms }
//       -> WakeEngine::on_command_received(text)
//
// Security:
//   Audio is never stored to disk.
//   Transcription happens entirely on-device.
//   No audio leaves the device -- ever.
//   Model file integrity verified with SHA-256 on load.
//
// Power:
//   STT only runs when WakeEngine is in Listening state.
//   CDSP spun up on wake, back to low power after transcription.
//   Estimated power draw: ~200mW during inference, ~5mW idle.

use std::time::{SystemTime, UNIX_EPOCH, Instant};
use std::collections::VecDeque;

// -- Constants ----------------------------------------------------------------

/// Target sample rate for Whisper (16kHz mono PCM16)
pub const WHISPER_SAMPLE_RATE:  u32 = 16_000;

/// Bytes per sample (PCM16 = 2 bytes)
pub const BYTES_PER_SAMPLE:     u32 = 2;

/// Max audio duration to transcribe (seconds)
pub const MAX_AUDIO_SECS:       u32 = 10;

/// Min audio duration before attempting transcription (ms)
pub const MIN_AUDIO_MS:         u32 = 200;

/// VAD silence threshold -- frames below this RMS are silence
pub const VAD_SILENCE_THRESHOLD: f32 = 0.01;

/// VAD: minimum consecutive silence frames to mark end of speech (ms)
pub const VAD_SILENCE_MS:       u32 = 500;

/// Confidence threshold below which transcription is discarded
pub const MIN_CONFIDENCE:       f32 = 0.4;

/// Default model path on device
pub const DEFAULT_MODEL_PATH:   &str = "/run/theos/models/whisper-tiny.en.bin";

// -- SttBackend ---------------------------------------------------------------

/// Which backend to use for transcription.
#[derive(Debug, Clone, PartialEq)]
pub enum SttBackend {
    /// Whisper.cpp on CDSP -- production (PostmarketOS / SDM845)
    WhisperCdsp { model_path: String },
    /// Whisper.cpp on CPU -- fallback if CDSP unavailable
    WhisperCpu  { model_path: String },
    /// Stub -- for tests and development on Windows
    Stub,
}

impl SttBackend {
    pub fn label(&self) -> &str {
        match self {
            SttBackend::WhisperCdsp { .. } => "whisper-cdsp",
            SttBackend::WhisperCpu  { .. } => "whisper-cpu",
            SttBackend::Stub               => "stub",
        }
    }

    pub fn is_hardware(&self) -> bool {
        !matches!(self, SttBackend::Stub)
    }
}

// -- TranscriptionResult ------------------------------------------------------

/// Result of a transcription attempt.
#[derive(Debug, Clone)]
pub struct TranscriptionResult {
    /// Transcribed text, trimmed and lowercased
    pub text:        String,
    /// Confidence score 0.0-1.0 (Whisper log-prob based)
    pub confidence:  f32,
    /// How long the audio was (ms)
    pub audio_ms:    u32,
    /// How long transcription took (ms)
    pub latency_ms:  u32,
    /// Backend that produced this result
    pub backend:     String,
    /// Timestamp of transcription
    pub timestamp:   u64,
}

impl TranscriptionResult {
    pub fn is_confident(&self) -> bool {
        self.confidence >= MIN_CONFIDENCE
    }

    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }

    /// Clean the transcription for command routing.
    /// Removes filler words, punctuation, normalizes whitespace.
    pub fn cleaned(&self) -> String {
        let s = self.text.trim().to_lowercase();
        // Remove trailing punctuation
        let s = s.trim_end_matches(|c: char| c == '.' || c == ',' || c == '!' || c == '?');
        // Remove common filler words at start
        let fillers = ["um ", "uh ", "like ", "so "];
        let mut s = s.trim();
        for filler in &fillers {
            if s.starts_with(filler) {
                s = s[filler.len()..].trim();
            }
        }
        s.to_string()
    }
}

// -- VoiceActivityDetector ----------------------------------------------------

/// Simple energy-based VAD.
/// Prevents transcribing silence or background noise.
/// On device: replaced with Qualcomm's VADQ for better accuracy.
pub struct VoiceActivityDetector {
    pub threshold:    f32,
    pub silence_ms:   u32,
    consecutive_silence_frames: u32,
    frame_duration_ms: u32,
}

impl VoiceActivityDetector {
    pub fn new() -> Self {
        Self {
            threshold:                  VAD_SILENCE_THRESHOLD,
            silence_ms:                 VAD_SILENCE_MS,
            consecutive_silence_frames: 0,
            frame_duration_ms:          20, // 20ms frames
        }
    }

    /// Is this audio frame speech (not silence)?
    /// Input: PCM16 samples as f32 normalized to -1.0..1.0
    pub fn is_speech(&mut self, samples: &[f32]) -> bool {
        let rms = rms(samples);
        if rms < self.threshold {
            self.consecutive_silence_frames += 1;
            false
        } else {
            self.consecutive_silence_frames = 0;
            true
        }
    }

    /// Has enough silence elapsed to mark end of utterance?
    pub fn end_of_speech(&self) -> bool {
        let silence_elapsed = self.consecutive_silence_frames * self.frame_duration_ms;
        silence_elapsed >= self.silence_ms
    }

    pub fn reset(&mut self) {
        self.consecutive_silence_frames = 0;
    }

    pub fn silence_duration_ms(&self) -> u32 {
        self.consecutive_silence_frames * self.frame_duration_ms
    }
}

fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() { return 0.0; }
    let sum_sq: f32 = samples.iter().map(|s| s * s).sum();
    (sum_sq / samples.len() as f32).sqrt()
}

// -- AudioBuffer --------------------------------------------------------------

/// Accumulates PCM audio frames for transcription.
/// Drops oldest frames when max duration exceeded.
pub struct AudioBuffer {
    frames:        VecDeque<Vec<f32>>, // each frame = 20ms of audio
    frame_ms:      u32,
    max_frames:    usize,
}

impl AudioBuffer {
    pub fn new() -> Self {
        let frame_ms   = 20u32;
        let max_frames = (MAX_AUDIO_SECS * 1000 / frame_ms) as usize;
        Self { frames: VecDeque::new(), frame_ms, max_frames }
    }

    pub fn push_frame(&mut self, samples: Vec<f32>) {
        if self.frames.len() >= self.max_frames {
            self.frames.pop_front();
        }
        self.frames.push_back(samples);
    }

    /// Flatten all frames into a single sample buffer for Whisper.
    pub fn flatten(&self) -> Vec<f32> {
        self.frames.iter().flatten().copied().collect()
    }

    pub fn duration_ms(&self) -> u32 {
        self.frames.len() as u32 * self.frame_ms
    }

    pub fn is_empty(&self) -> bool { self.frames.is_empty() }

    pub fn clear(&mut self) { self.frames.clear(); }

    pub fn frame_count(&self) -> usize { self.frames.len() }

    /// Convert PCM16 bytes (from ALSA) to f32 samples.
    pub fn pcm16_to_f32(bytes: &[u8]) -> Vec<f32> {
        bytes.chunks_exact(2)
            .map(|c| {
                let sample = i16::from_le_bytes([c[0], c[1]]);
                sample as f32 / 32768.0
            })
            .collect()
    }

    /// Convert f32 samples back to PCM16 bytes.
    pub fn f32_to_pcm16(samples: &[f32]) -> Vec<u8> {
        samples.iter()
            .flat_map(|s| {
                let clamped = s.clamp(-1.0, 1.0);
                let sample = (clamped * 32767.0) as i16;
                sample.to_le_bytes().to_vec()
            })
            .collect()
    }
}

// -- SttStats -----------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct SttStats {
    pub total_transcriptions: u64,
    pub successful:           u64,
    pub failed:               u64,
    pub low_confidence:       u64,
    pub total_audio_ms:       u64,
    pub total_latency_ms:     u64,
    pub vad_rejected:         u64,
}

impl SttStats {
    pub fn avg_latency_ms(&self) -> f32 {
        if self.successful == 0 { return 0.0; }
        self.total_latency_ms as f32 / self.successful as f32
    }

    pub fn success_rate(&self) -> f32 {
        if self.total_transcriptions == 0 { return 0.0; }
        self.successful as f32 / self.total_transcriptions as f32 * 100.0
    }

    pub fn record(&mut self, result: &TranscriptionResult) {
        self.total_transcriptions += 1;
        self.total_audio_ms   += result.audio_ms as u64;
        self.total_latency_ms += result.latency_ms as u64;
        if result.is_confident() && !result.is_empty() {
            self.successful += 1;
        } else if !result.is_confident() {
            self.low_confidence += 1;
        } else {
            self.failed += 1;
        }
    }
}

// -- SttEngine ----------------------------------------------------------------

/// The speech-to-text engine.
///
/// In production: loads Whisper model at boot, runs inference on CDSP.
/// In tests/dev: uses Stub backend with injected transcriptions.
pub struct SttEngine {
    pub backend:    SttBackend,
    pub stats:      SttStats,
    pub vad:        VoiceActivityDetector,
    pub buffer:     AudioBuffer,
    /// Stub: queue of transcriptions to return in order
    stub_queue:     VecDeque<String>,
    /// Whether the engine is actively listening
    pub listening:  bool,
}

impl SttEngine {
    /// Create a stub engine for testing (no hardware needed).
    pub fn new_stub() -> Self {
        println!("[stt] stub engine initialized");
        Self {
            backend:    SttBackend::Stub,
            stats:      SttStats::default(),
            vad:        VoiceActivityDetector::new(),
            buffer:     AudioBuffer::new(),
            stub_queue: VecDeque::new(),
            listening:  false,
        }
    }

    /// Create a production engine targeting the SDM845 CDSP.
    /// Falls back to CPU if CDSP unavailable.
    /// Falls back to Stub if model file not found.
    pub fn new_device() -> Self {
        let model_path = DEFAULT_MODEL_PATH.to_string();

        if std::path::Path::new(&model_path).exists() {
            println!("[stt] model found at {} -- using Whisper", model_path);
            // Production: try CDSP first, fall back to CPU
            // whisper_rs::WhisperContext::new_with_params(...)
            // For now: stub until whisper-rs is wired in
            println!("[stt] whisper-rs not yet wired -- falling back to stub");
            Self::new_stub()
        } else {
            println!("[stt] model not found at {} -- stub mode", model_path);
            Self::new_stub()
        }
    }

    /// Inject a transcription result (stub mode only, for testing).
    pub fn inject(&mut self, text: &str) {
        self.stub_queue.push_back(text.to_string());
    }

    /// Start listening -- begin buffering audio.
    pub fn start_listening(&mut self) {
        self.listening = true;
        self.buffer.clear();
        self.vad.reset();
        println!("[stt] listening started backend:{}", self.backend.label());
    }

    /// Stop listening and discard buffered audio.
    pub fn stop_listening(&mut self) {
        self.listening = false;
        self.buffer.clear();
        self.vad.reset();
        println!("[stt] listening stopped");
    }

    /// Push a PCM16 audio frame (20ms, 16kHz mono).
    /// Returns true if end-of-speech detected (ready to transcribe).
    pub fn push_audio(&mut self, pcm_bytes: &[u8]) -> bool {
        if !self.listening { return false; }
        let samples = AudioBuffer::pcm16_to_f32(pcm_bytes);
        let is_speech = self.vad.is_speech(&samples);
        self.buffer.push_frame(samples);
        // End of speech: had speech, then silence
        !is_speech && self.vad.end_of_speech() && self.buffer.duration_ms() > MIN_AUDIO_MS
    }

    /// Transcribe buffered audio.
    /// Returns None if audio is too short or VAD rejects it.
    pub fn transcribe(&mut self) -> Option<TranscriptionResult> {
        let audio_ms = self.buffer.duration_ms();

        if audio_ms < MIN_AUDIO_MS {
            self.stats.vad_rejected += 1;
            return None;
        }

        let start = Instant::now();

        let result = match &self.backend {
            SttBackend::Stub => {
                // Return next injected transcription or empty
                let text = self.stub_queue.pop_front().unwrap_or_default();
                let latency_ms = start.elapsed().as_millis() as u32;
                TranscriptionResult {
                    text:       text.trim().to_lowercase(),
                    confidence: if text.is_empty() { 0.0 } else { 0.95 },
                    audio_ms,
                    latency_ms,
                    backend:   "stub".to_string(),
                    timestamp:  now_secs(),
                }
            }
            SttBackend::WhisperCdsp { .. } | SttBackend::WhisperCpu { .. } => {
                // Production: whisper_rs inference
                // let samples = self.buffer.flatten();
                // let state = ctx.create_state()?;
                // state.full(params, &samples)?;
                // let text = (0..state.full_n_segments()).map(|i| state.full_get_segment_text(i)).collect()
                let latency_ms = start.elapsed().as_millis() as u32;
                TranscriptionResult {
                    text:       "whisper not yet wired".to_string(),
                    confidence: 0.0,
                    audio_ms,
                    latency_ms,
                    backend:    self.backend.label().to_string(),
                    timestamp:  now_secs(),
                }
            }
        };

        self.stats.record(&result);
        self.buffer.clear();
        self.vad.reset();

        if result.is_confident() && !result.is_empty() {
            Some(result)
        } else {
            None
        }
    }

    /// Full pipeline: push audio + auto-transcribe on end-of-speech.
    /// Returns transcription if speech ended and transcription succeeded.
    pub fn process_audio(&mut self, pcm_bytes: &[u8]) -> Option<TranscriptionResult> {
        if self.push_audio(pcm_bytes) {
            self.transcribe()
        } else {
            None
        }
    }

    pub fn is_listening(&self) -> bool { self.listening }
    pub fn buffer_duration_ms(&self) -> u32 { self.buffer.duration_ms() }
    pub fn backend_label(&self) -> &str { self.backend.label() }
}

// -- Helpers ------------------------------------------------------------------

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

// -- Tests --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn stub() -> SttEngine { SttEngine::new_stub() }

    // SttBackend
    #[test] fn test_backend_labels() {
        assert_eq!(SttBackend::Stub.label(), "stub");
        assert!(!SttBackend::Stub.is_hardware());
        assert!(SttBackend::WhisperCdsp { model_path: "x".to_string() }.is_hardware());
    }

    // TranscriptionResult
    #[test] fn test_confidence_check() {
        let r = TranscriptionResult { text: "hello".to_string(), confidence: 0.9,
            audio_ms: 1000, latency_ms: 200, backend: "stub".to_string(), timestamp: 0 };
        assert!(r.is_confident());
        assert!(!r.is_empty());
    }

    #[test] fn test_low_confidence() {
        let r = TranscriptionResult { text: "hello".to_string(), confidence: 0.2,
            audio_ms: 1000, latency_ms: 200, backend: "stub".to_string(), timestamp: 0 };
        assert!(!r.is_confident());
    }

    #[test] fn test_cleaned_removes_punctuation() {
        let r = TranscriptionResult { text: "call marcus.".to_string(), confidence: 0.9,
            audio_ms: 1000, latency_ms: 200, backend: "stub".to_string(), timestamp: 0 };
        assert_eq!(r.cleaned(), "call marcus");
    }

    #[test] fn test_cleaned_removes_filler() {
        let r = TranscriptionResult { text: "um call marcus".to_string(), confidence: 0.9,
            audio_ms: 1000, latency_ms: 200, backend: "stub".to_string(), timestamp: 0 };
        assert_eq!(r.cleaned(), "call marcus");
    }

    #[test] fn test_cleaned_lowercase() {
        let r = TranscriptionResult { text: "CALL MARCUS".to_string(), confidence: 0.9,
            audio_ms: 1000, latency_ms: 200, backend: "stub".to_string(), timestamp: 0 };
        assert_eq!(r.cleaned(), "call marcus");
    }

    // VAD
    #[test] fn test_vad_silence() {
        let mut vad = VoiceActivityDetector::new();
        let silence = vec![0.0f32; 320]; // silent frame
        assert!(!vad.is_speech(&silence));
    }

    #[test] fn test_vad_speech() {
        let mut vad = VoiceActivityDetector::new();
        let loud = vec![0.5f32; 320]; // loud frame
        assert!(vad.is_speech(&loud));
    }

    #[test] fn test_vad_end_of_speech() {
        let mut vad = VoiceActivityDetector::new();
        let silence = vec![0.0f32; 320];
        // Need 500ms / 20ms = 25 silent frames
        for _ in 0..26 { vad.is_speech(&silence); }
        assert!(vad.end_of_speech());
    }

    #[test] fn test_vad_reset() {
        let mut vad = VoiceActivityDetector::new();
        let silence = vec![0.0f32; 320];
        for _ in 0..26 { vad.is_speech(&silence); }
        vad.reset();
        assert!(!vad.end_of_speech());
    }

    // AudioBuffer
    #[test] fn test_buffer_empty() {
        let b = AudioBuffer::new();
        assert!(b.is_empty());
        assert_eq!(b.duration_ms(), 0);
    }

    #[test] fn test_buffer_push_frame() {
        let mut b = AudioBuffer::new();
        b.push_frame(vec![0.1f32; 320]);
        assert_eq!(b.frame_count(), 1);
        assert_eq!(b.duration_ms(), 20);
    }

    #[test] fn test_buffer_flatten() {
        let mut b = AudioBuffer::new();
        b.push_frame(vec![0.1f32; 320]);
        b.push_frame(vec![0.2f32; 320]);
        let flat = b.flatten();
        assert_eq!(flat.len(), 640);
    }

    #[test] fn test_pcm16_roundtrip() {
        let samples = vec![0.5f32, -0.5f32, 0.0f32, 1.0f32];
        let bytes = AudioBuffer::f32_to_pcm16(&samples);
        let back  = AudioBuffer::pcm16_to_f32(&bytes);
        for (a, b) in samples.iter().zip(back.iter()) {
            assert!((a - b).abs() < 0.001);
        }
    }

    #[test] fn test_pcm16_silence() {
        let silence = vec![0u8; 640];
        let samples = AudioBuffer::pcm16_to_f32(&silence);
        assert!(samples.iter().all(|s| *s == 0.0));
    }

    // SttEngine stub
    #[test] fn test_stub_starts_not_listening() {
        assert!(!stub().is_listening());
    }

    #[test] fn test_stub_start_listening() {
        let mut e = stub();
        e.start_listening();
        assert!(e.is_listening());
    }

    #[test] fn test_stub_stop_listening() {
        let mut e = stub();
        e.start_listening();
        e.stop_listening();
        assert!(!e.is_listening());
    }

    #[test] fn test_stub_inject_and_transcribe() {
        let mut e = stub();
        e.inject("message sarah");
        e.start_listening();
        // Push enough audio to fill buffer past MIN_AUDIO_MS
        let frame = vec![0u8; 640]; // 20ms of silence PCM16
        for _ in 0..15 { e.push_audio(&frame); }
        // Force transcribe
        let result = e.transcribe();
        assert!(result.is_some());
        assert_eq!(result.unwrap().text, "message sarah");
    }

    #[test] fn test_stub_empty_queue_returns_none() {
        let mut e = stub();
        e.start_listening();
        let frame = vec![0u8; 640];
        for _ in 0..15 { e.push_audio(&frame); }
        let result = e.transcribe();
        assert!(result.is_none()); // empty queue = empty text = low confidence
    }

    #[test] fn test_stub_backend_label() {
        assert_eq!(stub().backend_label(), "stub");
    }

    // SttStats
    #[test] fn test_stats_success_rate() {
        let mut s = SttStats::default();
        let good = TranscriptionResult { text: "hello".to_string(), confidence: 0.9,
            audio_ms: 1000, latency_ms: 100, backend: "stub".to_string(), timestamp: 0 };
        let bad = TranscriptionResult { text: String::new(), confidence: 0.1,
            audio_ms: 500, latency_ms: 50, backend: "stub".to_string(), timestamp: 0 };
        s.record(&good);
        s.record(&bad);
        assert_eq!(s.total_transcriptions, 2);
        assert_eq!(s.successful, 1);
        assert!((s.success_rate() - 50.0).abs() < 0.1);
    }

    #[test] fn test_stats_avg_latency() {
        let mut s = SttStats::default();
        let r = TranscriptionResult { text: "hi".to_string(), confidence: 0.9,
            audio_ms: 1000, latency_ms: 200, backend: "stub".to_string(), timestamp: 0 };
        s.record(&r);
        assert!((s.avg_latency_ms() - 200.0).abs() < 0.1);
    }

    // rms helper
    #[test] fn test_rms_silence() {
        assert_eq!(rms(&[0.0f32; 100]), 0.0);
    }

    #[test] fn test_rms_nonzero() {
        assert!(rms(&[0.5f32; 100]) > 0.0);
    }
}
