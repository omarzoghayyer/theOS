# theOS

**A from-scratch, bare-metal Rust operating system for mobile. No phone numbers. No SIM cards. No carriers. No legacy kernel.**

Your identity is an Ed25519 cryptographic keypair. You are reachable only by people who have your public key. Calls and messages are meant to route over Starlink, with no cellular infrastructure involved. There is no Linux, no Android, and no third-party OS underneath — theOS is its own kernel, drivers, and UI, written from scratch in Rust.

---

## What it is

theOS is **not** built on PostmarketOS, Android, or any existing Linux distribution. `theos-kernel` is a `no_std` Rust kernel targeting `aarch64-unknown-none` directly. The goal is to power on the device and see theOS's own UI, top to bottom, with no third-party userspace underneath it.

**Target hardware:** Google Pixel 7 Pro ("cheetah", Tensor G2 / GS201).

The intended interface is a single AI orb that listens and becomes whatever you need, instead of an app grid or home screen:

```
"message Sarah"     → encrypted conversation, routed over Starlink
"call Marcus"       → identity-based encrypted voice call, no carrier involved
"show connection"   → satellite status, DHT peers, your identity
```

**Note on scope:** the AI-shell interaction model above describes the design intent for `theos-compositor`. The call surface (`call_ui.rs`) and intent routing (`ai_shell.rs`) are implemented and tested on the host, but the compositor does not yet run on `theos-kernel` on real hardware — see [Status](#status) below.

---

## Architecture

```
+-------------------------------------------+
|            AI Shell (orb UI)               |  <- intent routing, call surface (host-tested)
+---------------------------------------------+
|      theos-compositor (Rust/smithay)       |  <- Wayland compositor, GLES renderer (x86_64 only)
+---------------+-----------------------------+
|  theos-core    |       services             |  <- crypto, DHT, identity, messenger, VoIP, NTP
+---------------+-----------------------------+
|           theos-kernel (Rust)              |  <- no_std, aarch64-unknown-none, from scratch
+---------------------------------------------+
|        Starlink / satellite link           |  <- no SIM, no carrier
+---------------------------------------------+
```

**Identity:** Ed25519 keypairs via `ed25519-dalek`. Your public key is your address. No central registry, no phone number anywhere in the system.

**Encryption:** Signal-grade stack — X3DH for initial key agreement, Double Ratchet for per-message forward secrecy (including skipped-message-key handling), ChaCha20-Poly1305 AEAD for the actual payload. Ratchet state only commits after a message decrypts successfully, so a forged or corrupted packet can't mutate live session state — deliberately hardened for lossy satellite links.

**Peer discovery:** Kademlia DHT. Handles resolve to Ed25519 public keys, which resolve to peer addresses, backed by a UDP bootstrap server.

**Kernel:** capability-based process isolation (unforgeable capability tokens — no privilege escalation by construction), seL4-inspired synchronous rendezvous IPC, a strict-priority scheduler tuned for audio latency, and a buddy allocator — all `no_std`, no heap in the kernel.

---

## Status

theOS is in active early development. This section is meant to be read literally — items marked as a gap are known-missing, not "in progress."

| Layer | Status |
|---|---|
| Ed25519 identity, X3DH, Double Ratchet, ChaCha20-Poly1305 | Done — implemented and host-tested |
| Kademlia DHT + handle -> key -> peer resolution | Done — verified live against a running bootstrap server |
| Kernel: memory (buddy allocator), process/capabilities, IPC, scheduler, GICv2 timer | Done — boots cleanly in QEMU (`aarch64-unknown-none`) |
| Kernel on real Pixel 7 Pro hardware | In progress — first `fastboot boot` succeeded (bootloader accepted the image and jumped to the kernel), but the kernel currently resets almost immediately. It was still using a QEMU-specific UART address rather than the Tensor G2 (Exynos-derived) UART. Retargeting the UART, memory map, and interrupt controller to real cheetah/gs201 addresses is in progress. |
| Display / GPU driver (DECON, Mali-G710) | Gap — no display or GPU driver exists yet at the kernel level. Register-level work against Google's public GS101/GS201 display CAL source has started (`theos-driver-decon`), but nothing draws to the screen yet. |
| `theos-compositor` (Wayland/smithay UI) | Partial — compiles cleanly and its call-surface/intent-routing logic is host-tested, but it's still an x86_64, `std`-targeting binary. Running it on-device needs a GPU driver, an EGL/GLES stack, a userspace process/syscall model, and input handling — none of which theos-kernel provides yet. This is the largest remaining gap between "kernel boots" and "UI on screen." |
| Audio (Opus codec, VoIP pipeline) | Partial — implemented in `services`, not yet wired to real hardware audio (Tensor audio subsystem + Cirrus CS35L41 amp driver still in progress) |
| DHT bootstrap server | Done — built, tested, deployed live on Railway |

**Host-side test suite:** 559 tests passing in `theos-core` (crypto, identity, DHT, messenger, NTP, etc.), plus dedicated suites for kernel logic mirrored to the host (memory/IPC/scheduler), the call surface, and intent routing. Driver crates (`theos-driver-cs35l41`, `theos-driver-decon`) have their own host-side test suites against mocked register buses, since register constants haven't all been verified against hardware yet.

**Bottom line:** the kernel is real and boots (fully in QEMU, partially on real hardware), the crypto/identity/DHT logic is real and tested, and the compositor UI logic is real and tested — but "theOS running on a Pixel 7 Pro screen" requires display and GPU drivers and a much larger userspace layer that don't exist yet. This is stated here deliberately rather than glossed over.

---

## Repository structure

```
theOS/
├── theos-kernel/         # no_std kernel: boot, memory, process, IPC, scheduler, HAL (UART/GIC/DTB), drivers
├── theos-core/           # std, cross-platform: crypto (X3DH/Double Ratchet/ChaCha20-Poly1305), identity,
│                         # DHT, handle registry, messenger, NTP, power
├── theos-compositor/     # Wayland compositor (smithay) - Linux/x86_64 only for now; AI shell, call surface
├── drivers/
│   ├── cs35l41/          # Cirrus Logic CS35L41 audio amp driver (workspace crate, host-testable)
│   └── decon/            # Samsung/Exynos DECON display controller driver (workspace crate, host-testable)
├── services/              # Background daemons - bootstrap/DHT server, VoIP, audio, NTP task
└── tools/                 # capctl and other utilities
```

---

## Building

**theos-core** (pure logic, builds and tests on any host platform):

```bash
cargo test -p theos-core --target x86_64-unknown-linux-gnu
```

**theos-kernel** (bare-metal target):

```bash
cargo build -p theos-kernel --target aarch64-unknown-none
```

**theos-compositor** (Linux/x86_64 only — requires Wayland, libinput, DRM):

```bash
cargo build -p theos-compositor --features compositor
```

**Driver crates** (host-testable against mocked register buses):

```bash
cargo test -p theos-driver-cs35l41 -p theos-driver-decon --target x86_64-unknown-linux-gnu
```

The workspace defaults to `aarch64-unknown-none`, so host-side tests always need `--target x86_64-unknown-linux-gnu` explicitly.

---

## Design principles

**Cryptographic identity over phone numbers.** There is no phone number anywhere in this system. Your Ed25519 public key is your address.

**Satellite-first networking.** Networking and audio decisions are made for satellite-link characteristics (higher latency, variable jitter, frequent IP changes from beam switches), not for LAN.

**No legacy kernel.** theos-kernel is written from scratch rather than adapted from Linux or Android, specifically to avoid inheriting decades of legacy attack surface and driver cruft.

**AI as the OS, not an app on top of it.** There is no app store and no third-party SDK planned. The intent is for the AI shell to be the interface, with the open web (not native apps) as the application layer.

---

## Roadmap

| Phase | Name | Status |
|---|---|---|
| 1 | Boot | Done — ARM64 ELF, UART, DTB parser, confirmed booting in QEMU |
| 2 | Core kernel | Done — memory, process/capabilities, IPC, scheduler, GIC/timer, all active in QEMU |
| — | Compositor build + call UI | In progress — compiles clean; call surface and DHT contact resolution done; GLES text rendering and daemon socket wiring remain |
| 3 | Hardware drivers | In progress — UART retargeted to a real hardware address (verification pending); display, WiFi, and audio drivers still being developed against Pixel 7 Pro / Tensor G2 silicon |
| 4 | Driver isolation | Not started — each driver as an isolated process with only its own MMIO capability |
| 5 | GPU driver | Not started — Panfrost-class driver for the Mali-G710 |
| 6 | Production | Not started — OTA delta updates, verified boot, A/B partitions, Ed25519 update signing |
| 7 | Formal verification | Not started — seL4-inspired proof of memory isolation and IPC correctness |

---

## Why

Every phone requires a phone number. Phone numbers require carriers. Carriers require governments to assign spectrum. That's a chain of dependencies and a point of control over who can communicate.

theOS is an attempt to remove that dependency: identity as math, connectivity as satellites, and a kernel with no inherited legacy surface underneath either one.

---

## License

Not yet finalized — this repository does not currently include a LICENSE file. Until one is added, don't assume MIT or any other permissive terms.

---

## Contact

Built by [@omarzoghayyer](https://github.com/omarzoghayyer)