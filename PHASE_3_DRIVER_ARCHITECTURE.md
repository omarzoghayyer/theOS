# Phase 3: Hardware Driver Architecture for theOS

**Target Device:** Google Pixel 7 Pro (codename: cheetah, Tensor G2)  
**Kernel:** theOS aarch64 no_std (Phase 2 complete)  
**Status:** Architecture design (implementation awaits hardware)

---

## Overview

Phase 3 introduces three critical drivers to boot the OS on real hardware:

1. **Audio Codec Driver (WCD codec)** — VoIP audio capture/playback
2. **WiFi Driver (ath11k port)** — Satellite fallback networking
3. **Touch Driver (I2C)** — Call surface input

Each runs as an **isolated process** with capability-based access to MMIO registers (Phase 4 isolation model).

---

## Design Principles

### 1. Isolation First
- Each driver = isolated process spawned by init
- Only capability: `Mmio` for its specific register range
- No shared memory; communication via IPC channels
- Privilege escalation impossible by construction

### 2. Minimal Kernel Changes
- Phase 2 kernel (memory, process, IPC, scheduler) is stable
- Drivers are **userspace daemons**, not kernel modules
- DTB (device tree blob) already parsed in Phase 1
- HAL expands only with device enumeration; no driver-specific code in kernel

### 3. Incremental Testing
- Boot kernel + init on QEMU (Phase 2 already working)
- Flash PostmarketOS on Pixel 7 Pro
- Boot kernel on real hardware
- Spawn drivers one by one; test each in isolation
- Full demo only when all three are functional

---

## 1. Audio Codec Driver (WCD codec)

### Hardware
- **Chip:** Qualcomm WCD9385 (on Pixel 7 Pro)
- **Interface:** I2S (Inter-IC Sound) for PCM audio
- **Control:** I2C (SMBus) for codec configuration
- **Registers:** ~2KB MMIO range

### Kernel Integration
kernel_main()
→ init spawned
→ audio_driver spawned (process::spawn)
→ requests Mmio capability for WCD registers
→ initializes codec via I2C + I2S

### Driver Responsibilities

1. **Initialization** (once, on spawn)
   - Parse device tree for WCD node (already in DTB)
   - Request Mmio capability from kernel for register range
   - Initialize I2C/I2S interfaces
   - Load codec firmware (if needed)
   - Set default gain levels

2. **Audio Path Management**
   - Route microphone → ADC → I2S output (for VoIP RX)
   - Route I2S input → DAC → speaker (for VoIP TX)
   - Mute/unmute on compositor command

3. **RTP Integration**
   - Receive raw PCM from daemon (services/voip.rs)
   - Write to DAC (playback)
   - Read from ADC (capture)
   - Handle jitter buffer (already in voip.rs)

### File Structure
theos-kernel/src/drivers/
├── audio.rs          # WCD codec driver
├── mod.rs            # driver dispatcher
└── audio/
├── wcd.rs        # WCD9385 register definitions
├── i2s.rs        # I2S interface
└── i2c.rs        # I2C control

### Register Map (WCD9385)
```rust
// Partial register definitions
const WCD_CHIP_ID: u16 = 0x0000;      // read-only, expect 0x9385
const WCD_RESET: u16 = 0x0001;
const WCD_RX_PATH: u16 = 0x0700;      // routing control
const WCD_TX_PATH: u16 = 0x0800;
const WCD_GAIN_RX: u16 = 0x0900;      // RX gain (0-15, 0dB-30dB)
const WCD_GAIN_TX: u16 = 0x0910;      // TX gain (0-15)
```

### IPC Protocol (to daemon)
```rust
// From daemon (services/voip.rs)
AudioMessage {
    StartCapture { sample_rate: u32 },  // 16kHz for VoIP
    StopCapture,
    StartPlayback { sample_rate: u32 },
    StopPlayback,
    SetGain { tx: u8, rx: u8 },         // 0-15
    Mute { muted: bool },
}

// From driver (to daemon)
AudioEvent {
    CaptureReady { frame: [i16; 320] },  // 20ms @ 16kHz
    PlaybackRequest { frame: [i16; 320] },
    Error { code: u32 },
}
```

### Testing Checklist
- [ ] Kernel boots, spawns audio driver
- [ ] Driver initializes WCD codec
- [ ] I2C communication verified (read CHIP_ID)
- [ ] I2S clocking enabled
- [ ] Microphone capture produces non-zero samples
- [ ] Speaker playback audible (sine wave test)
- [ ] RTP integration: daemon sends Opus, driver decodes to PCM, playback works

---

## 2. WiFi Driver (ath11k port)

### Hardware
- **Chip:** Qualcomm WCN6855 (Pixel 7 Pro)
- **Interface:** PCIe or SDIO (likely PCIe on cheetah)
- **Firmware:** ath11k firmware (mainline Linux has this)
- **Registers:** ~64KB MMIO range

### Kernel Integration
kernel_main()
→ init spawned
→ wifi_driver spawned
→ requests Mmio capability for WCN registers
→ loads firmware
→ connects to WiFi

### Driver Responsibilities

1. **Initialization**
   - Parse device tree for WiFi node
   - Request Mmio capability for PCIe BAR
   - Load ath11k firmware from filesystem
   - Scan for WiFi networks

2. **Network Stack**
   - UDP socket for RTP voice packets
   - DHCP to get IP address
   - Keep-alive (ping) to detect beam switches

3. **DHT Resolver Integration**
   - Respond to compositor's StartCall intent
   - Query bootstrap server (via UDP)
   - Resolve @handle → peer address
   - Return peer address to daemon

### File Structure
theos-kernel/src/drivers/
├── wifi.rs           # ath11k driver
└── wifi/
├── ath11k.rs     # register definitions
├── firmware.rs   # firmware loading
├── mac80211.rs   # wireless stack
└── udp.rs        # UDP socket layer

### UDP Integration
```rust
// Driver exposes UDP socket to daemon
pub async fn send_udp(dest_addr: SocketAddr, payload: &[u8]) -> Result<(), Error>;
pub async fn recv_udp() -> Result<(SocketAddr, Vec<u8>), Error>;
```

### IPC Protocol (to daemon)
```rust
WiFiMessage {
    Connect { ssid: String, password: String },
    Disconnect,
    Scan,
    GetStats,
}

WiFiEvent {
    Connected { ip: String, gateway: String },
    Disconnected,
    ScanResults { networks: Vec<NetworkInfo> },
    Stats { rssi: i8, link_speed: u16 },  // dBm, Mbps
    Error { code: u32 },
}
```

### Testing Checklist
- [ ] Kernel boots, spawns WiFi driver
- [ ] Driver initializes WCN6855 (verify PCIe enumeration)
- [ ] Firmware loads successfully
- [ ] WiFi scan returns visible networks
- [ ] Connects to test network
- [ ] DHCP acquires IP address
- [ ] Ping works (connectivity verified)
- [ ] UDP socket works (send/recv packets)
- [ ] DHT bootstrap resolution works

---

## 3. Touch Driver (I2C)

### Hardware
- **Chip:** Touchscreen controller (exact model TBD from Pixel 7 Pro teardown)
- **Interface:** I2C @ standard address (0x4a or 0x48)
- **Interrupt:** GPIO pin (likely GPIO 9)
- **Resolution:** 1080×2400 (Pixel 7 Pro screen)

### Kernel Integration
kernel_main()
→ init spawned
→ touch_driver spawned
→ requests Mmio capability for I2C + GPIO
→ initializes touch controller
→ listens for touch IRQ
→ sends events to compositor

### Driver Responsibilities

1. **Initialization**
   - Parse device tree for touchscreen node
   - Initialize I2C master for touch controller
   - Set up GPIO interrupt for touch IRQ
   - Calibrate if needed

2. **Touch Event Handling**
   - Read touch coordinates from I2C
   - Normalize to screen coordinates (1080×2400)
   - Classify gesture (tap, swipe, long-press)
   - Send to compositor over IPC

### File Structure
theos-kernel/src/drivers/
├── touch.rs          # touch driver
└── touch/
├── i2c.rs        # I2C master
├── gpio.rs       # GPIO interrupt handling
└── gestures.rs   # gesture classification

### Touch Protocol (to compositor)
```rust
TouchEvent {
    Down { x: u16, y: u16 },
    Up { x: u16, y: u16 },
    Move { x: u16, y: u16 },
    LongPress { x: u16, y: u16 },
    Swipe { direction: SwipeDir, velocity: f32 },
}

enum SwipeDir { Up, Down, Left, Right }
```

### Testing Checklist
- [ ] Kernel boots, spawns touch driver
- [ ] I2C communication verified (read touch controller ID)
- [ ] GPIO interrupt fires on touch
- [ ] Touch events arrive at compositor
- [ ] Tap on call surface button triggers action
- [ ] Swipe gestures work (back/forward)

---

## Integration with Phase 2 Kernel

### No Kernel Changes Needed
- `process::spawn()` already works
- `capability::CapabilitySet` already supports Mmio
- IPC channels already working
- Scheduler handles driver processes

### Device Tree (DTB)
- Already parsed in Phase 1 (`hal::dtb::parse()`)
- Drivers read MMIO addresses from DTB
- Firmware paths read from DTB or hardcoded

### Init Process
Current `idle_loop()` in `main.rs` will become:

```rust
fn idle_loop() -> ! {
    // Spawn drivers
    spawn_driver("audio", audio_main, Mmio(0x62_000_000..0x62_100_000));
    spawn_driver("wifi", wifi_main, Mmio(0x01_c00_000..0x01_d00_000));
    spawn_driver("touch", touch_main, Mmio(0x970_000..0x980_000, 0x110_000..0x120_000));
    
    // Wait for drivers to be ready
    loop {
        unsafe { core::arch::asm!("wfe") }
    }
}
```

---

## Firmware & Binary Blobs

### Where to Get Them
- **WCD firmware:** `firmware-qcom-adreno` package (mainline Linux)
- **ath11k firmware:** `linux-firmware/ath11k` directory
- **Touch firmware:** Likely bundled with touch controller or in Pixel bootloader

### Loading Strategy
1. Store firmware in `/lib/firmware/` on Pixel 7 Pro (after flashing kernel)
2. Drivers read from filesystem at boot
3. Load via DMA from userspace (no need for kernel loader)

---

## Phase 3 Timeline (Estimate)

| Task | Days | Notes |
|------|------|-------|
| Get Pixel 7 Pro hardware | — | Purchased |
| Flash PostmarketOS kernel | 1 | Verify aarch64 boots |
| Audio driver (WCD codec) | 3-4 | Hardest; I2S complexity |
| WiFi driver (ath11k) | 2-3 | Firmware integration |
| Touch driver (I2C) | 1-2 | Simplest; interrupt handling |
| Integration testing | 2-3 | End-to-end with compositor |
| **Total** | **9-13 days** | — |

---

## Success Criteria

**Phase 3 Complete When:**

1. ✅ Kernel boots on Pixel 7 Pro
2. ✅ Audio capture/playback works (test tone audible)
3. ✅ WiFi connects to network + DHCP works
4. ✅ Touch input responds to taps/swipes
5. ✅ Compositor receives touch events
6. ✅ All three drivers run simultaneously without crashes
7. ✅ **60-second demo call:** "call alice" → DHT lookup → audio flows encrypted

---

## References

- Qualcomm WCD9385 Audio Codec
- ath11k WiFi Driver (mainline Linux)
- Pixel 7 Pro Hardware Teardown
- ARM AMBA Primecell I2C (PL022)

