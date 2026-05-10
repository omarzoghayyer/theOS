# theOS

**A satellite-first mobile operating system. No phone numbers. No SIM cards. No carriers.**

Your identity is an Ed25519 cryptographic keypair. You are reachable only by people who have scanned your QR code. Calls and messages route over Starlink with no cellular infrastructure involved.

---

## What it is

theOS is an open-source mobile OS built on PostmarketOS, targeting the OnePlus 6. It replaces the phone number — and everything built on top of it — with a cryptographic identity layer and satellite connectivity.

The user interface is a single AI that listens and becomes whatever you need. No app grid. No home screen. Just say what you want.

```
"message Sarah"     → encrypted conversation, routed over Starlink
"open Instagram"    → privacy sandbox, tracking stripped
"show connection"   → satellite status, DHT peers, your identity
"call Marcus"       → E2E encrypted voice, no carrier involved
```

---

## Architecture

```
┌─────────────────────────────────────────┐
│            AI Shell (Wayland)           │  ← the only thing the user sees
├─────────────────────────────────────────┤
│         theos-compositor (Rust)         │  ← Wayland compositor, GLES renderer
├──────────────┬──────────────────────────┤
│  theos-core  │     services             │  ← crypto, DHT, messenger, VoIP
├──────────────┴──────────────────────────┤
│         PostmarketOS (Linux)            │  ← mainline kernel, no Android
├─────────────────────────────────────────┤
│         Starlink / satellite            │  ← no SIM, no carrier
└─────────────────────────────────────────┘
```

**Identity:** Ed25519 keypairs via `ed25519-dalek`. Your public key is your address. No central registry. You can only be reached by contacts who have scanned your QR code.

**Encryption:** ChaCha20-Poly1305 AEAD for all messages and calls. Session keys derived from both parties' Ed25519 keys and the session ID — no key exchange round-trip needed.

**Peer discovery:** Kademlia DHT — no central servers. Handles map to public keys, gossiped across the network and signed so nobody can squat your handle without your private key.

**Audio:** Opus codec with satellite-optimized jitter buffering. 20ms frames, 200ms jitter buffer, automatic bitrate adaptation on beam switches.

**Network:** Starlink-first. RE-INVITE on beam switches keeps calls alive across IP changes. RTP keepalive every 5 seconds.

---

## Status

Early development. The following modules are complete with passing tests:

| Module | Status | Tests |
|---|---|---|
| Ed25519 cryptographic identity | ✅ complete | passing |
| ChaCha20-Poly1305 encryption | ✅ complete | 20 tests |
| Kademlia DHT peer discovery | ✅ complete | passing |
| Handle registry (@handle → pubkey) | ✅ complete | 40 tests |
| Connection request flow | ✅ complete | 62 tests |
| Messenger data model | ✅ complete | 27 tests |
| AI shell + intent routing | ✅ complete | 29 tests |
| Wayland compositor | ✅ complete | — |
| GLES render pipeline + AI orb | ✅ complete | — |
| Audio engine (Opus + ALSA) | ✅ complete | — |
| Hardware Abstraction Layer | ✅ complete | — |
| PostmarketOS flashing | 🔄 in progress | — |
| First device-to-device call | 🔄 in progress | — |

**89 tests passing** across `theos-core` (pure logic, runs on any platform).

---

## Target hardware

**OnePlus 6** (Snapdragon 845) — mainline kernel support in PostmarketOS, $80–120 used. Previously targeted Pixel 6/7; switched for better mainline kernel support and lower cost.

---

## Repository structure

```
theOS/
├── theos-core/          # Pure Rust logic — crypto, DHT, identity, messenger
│   └── src/
│       ├── crypto.rs    # ChaCha20-Poly1305 encryption
│       ├── identity/    # Ed25519 keypairs, contacts, QR exchange, connection flow
│       ├── dht/         # Kademlia routing, handle registry
│       └── messenger.rs # Conversations, delivery states, animations
├── theos-compositor/    # Wayland compositor — Linux only
│   └── src/
│       ├── main.rs      # Compositor entry point
│       ├── render.rs    # GLES pipeline, AI orb, animations
│       ├── ai_shell.rs  # Conversational interface, intent routing
│       ├── shell.rs     # Traditional home screen (escape hatch)
│       ├── dialer.rs    # Call UI
│       └── ...
├── services/            # Background daemons — VoIP, network, audio
└── tools/               # capctl and other utilities
```

---

## Building

**theos-core** (pure logic, builds on any platform):

```bash
cargo test -p theos-core
```

**theos-compositor** (Linux only — requires Wayland, libinput, DRM):

```bash
cargo build -p theos-compositor --features compositor
```

**Requirements:** Rust stable, Linux (WSL2 works for theos-core tests). Full compositor build requires PostmarketOS or a Linux machine with Wayland stack.

---

## Design principles

**Cryptographic identity over phone numbers.** There is no phone number in this system. Your Ed25519 public key is your address. Nobody can reach you without an explicit invitation. No cold calls, no spam, no number porting.

**Satellite-first networking.** Every networking and audio decision is optimized for ~600ms latency, variable jitter, and frequent IP changes — not for low-latency LAN. This means the OS works anywhere Starlink reaches.

**AI as the OS.** The user never sees an app grid. The AI shell is the only interface. It understands intent and surfaces the right experience — messaging, calls, web apps — without the user navigating anywhere.

**Privacy by default.** Web apps run in a sandboxed webview with tracking scripts stripped, no fingerprinting, and encrypted sessions. The OS never phones home.

**Open source core, proprietary network infrastructure.** The OS is MIT licensed. Network infrastructure (bootstrap servers, relay nodes) is proprietary — the Red Hat model.

---

## Roadmap

**90-day sprint (demo target):**
- [ ] Flash PostmarketOS on OnePlus 6
- [ ] Wire AI orb → slide-up surface transitions
- [ ] First encrypted call between two devices over Starlink
- [ ] 60-second demo video

**Phase 2 (post-demo):**
- [ ] ChaCha20 upgrade in VoIP layer
- [ ] Messenger conversation screen on device
- [ ] DHT bootstrap server on Railway.app
- [ ] NLnet Foundation grant application
- [ ] Show HN launch

**Phase 3 (open source launch):**
- [ ] Public repository announcement
- [ ] Contributor documentation
- [ ] PostmarketOS device port submission

---

## Why

Every phone on earth requires a phone number. Phone numbers require carriers. Carriers require governments to assign spectrum. This creates a single point of control over who can communicate.

theOS removes that dependency entirely. Your identity is math. Your connectivity is satellites. Nobody can take either away . 

---

## License

MIT — see [LICENSE](LICENSE)

The network infrastructure (bootstrap servers, relay nodes, DHT coordination layer) is proprietary and operated separately.

---

## Contact

Built by [@omarzoghayyer](https://github.com/omarzoghayyer)

*theOS is in early development. The first demo is a 60-second video of two devices completing an encrypted voice call with no SIM cards or carrier involvement.*