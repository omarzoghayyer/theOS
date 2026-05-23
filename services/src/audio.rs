// NOTE: The audio capture/playback/OPUS/RTP helpers below are staged for
// the demo call path -- implemented but not yet wired into the live call
// loop. Flagged as unused until then. Intentional; do not delete.
#![allow(dead_code)]
// audio.rs — Audio Engine
// Handles microphone capture and speaker playback
// Streams audio via RTP over satellite connection
// Uses OPUS codec — best for variable latency networks like satellite

use std::error::Error;
use tokio::net::UdpSocket;
use tokio::sync::watch;

const SAMPLE_RATE: u32 = 16000;    // 16kHz — good for voice
const FRAME_SIZE: usize = 320;     // 20ms frames at 16kHz
const RTP_PAYLOAD_TYPE: u8 = 111; // Dynamic payload type for OPUS

#[derive(Clone)]
pub struct AudioEngine {
    stop_tx: watch::Sender<bool>,
    stop_rx: watch::Receiver<bool>,
}

impl AudioEngine {
    pub fn new() -> Result<Self, Box<dyn Error>> {
        let (stop_tx, stop_rx) = watch::channel(false);
        println!("Audio engine initialized ({}Hz, OPUS codec)", SAMPLE_RATE);
        Ok(Self { stop_tx, stop_rx })
    }
    pub async fn start(&self) {
    println!("Audio stream started.");
}
    /// Start bidirectional audio — capture mic and play incoming audio
    pub async fn start_capture_and_playback(&self, rtp_port: u16) -> Result<(), Box<dyn Error>> {
        println!("Starting audio capture and playback on RTP port {}", rtp_port);

        let stop_rx_capture = self.stop_rx.clone();
        let stop_rx_playback = self.stop_rx.clone();

        // Outbound: capture microphone and send RTP packets
        tokio::spawn(async move {
            let socket = UdpSocket::bind("0.0.0.0:0").await.unwrap();
            let mut sequence: u16 = 0;
            let mut timestamp: u32 = 0;

            loop {
                if *stop_rx_capture.borrow() { break; }

                // Production: capture real PCM from ALSA/PulseAudio
                // For now generate silence frames to prove the pipeline
                let pcm_frame = capture_audio_frame();
                let encoded = encode_opus(&pcm_frame);
                let rtp_packet = build_rtp_packet(sequence, timestamp, &encoded);

                // Send to remote RTP endpoint
                // Production: remote address comes from SIP SDP negotiation
                let _ = socket.send_to(&rtp_packet, format!("0.0.0.0:{}", rtp_port)).await;

                sequence = sequence.wrapping_add(1);
                timestamp = timestamp.wrapping_add(FRAME_SIZE as u32);

                // 20ms frame interval
                tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
            }
        });

        // Inbound: receive RTP packets and play to speaker
        tokio::spawn(async move {
            let socket = UdpSocket::bind(format!("0.0.0.0:{}", rtp_port)).await.unwrap();
            let mut buf = [0u8; 2048];

            loop {
                if *stop_rx_playback.borrow() { break; }

                match socket.recv_from(&mut buf).await {
                    Ok((len, _addr)) => {
                        let rtp_payload = parse_rtp_payload(&buf[..len]);
                        let pcm = decode_opus(rtp_payload);
                        // Production: write PCM to ALSA playback buffer
                        play_audio_frame(&pcm);
                    }
                    Err(e) => eprintln!("RTP receive error: {}", e),
                }
            }
        });

        Ok(())
    }

    /// Stop audio streams
    pub async fn stop(&self) {
        let _ = self.stop_tx.send(true);
        println!("Audio streams stopped.");
    }
}

// ---- Audio I/O stubs (replace with ALSA/cpal bindings in production) ----

fn capture_audio_frame() -> Vec<i16> {
    // Production: read from ALSA capture device
    // alsa::PCM::new("default", alsa::Direction::Capture, false)
    vec![0i16; FRAME_SIZE] // Silence for now
}

fn play_audio_frame(pcm: &[i16]) {
    // Production: write to ALSA playback device
    // alsa::PCM::new("default", alsa::Direction::Playback, false)
    let _ = pcm; // Stub
}

// ---- OPUS codec stubs (replace with opus crate bindings) ----

fn encode_opus(pcm: &[i16]) -> Vec<u8> {
    // Production: use opus crate
    // let encoder = opus::Encoder::new(SAMPLE_RATE, opus::Channels::Mono, opus::Application::Voip)
    // encoder.encode(pcm, &mut output_buf)
    pcm.iter().flat_map(|s| s.to_le_bytes()).collect()
}

fn decode_opus(data: &[u8]) -> Vec<i16> {
    // Production: use opus crate decoder
    data.chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect()
}

// ---- RTP packet helpers ----

fn build_rtp_packet(sequence: u16, timestamp: u32, payload: &[u8]) -> Vec<u8> {
    let ssrc: u32 = 0xDEADBEEF; // Random SSRC — production generates this properly
    let mut packet = Vec::with_capacity(12 + payload.len());

    // RTP header (RFC 3550)
    packet.push(0x80);                                    // V=2, P=0, X=0, CC=0
    packet.push(RTP_PAYLOAD_TYPE);                        // Marker=0, PT
    packet.extend_from_slice(&sequence.to_be_bytes());   // Sequence number
    packet.extend_from_slice(&timestamp.to_be_bytes());  // Timestamp
    packet.extend_from_slice(&ssrc.to_be_bytes());       // SSRC
    packet.extend_from_slice(payload);                   // Encoded audio

    packet
}

fn parse_rtp_payload(packet: &[u8]) -> &[u8] {
    // Skip 12-byte RTP header
    if packet.len() > 12 { &packet[12..] } else { &[] }
}
