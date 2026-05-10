# theOS — Project Roadmap

## Vision
A Linux-based mobile OS where satellite connectivity (Starlink/NTN) is a first-class citizen.
Primary goal: Make and receive calls directly over satellite. No traditional cell towers required.

## Target Hardware
- Google Pixel 6 / 7 (best mainline Linux kernel support)
- Starlink direct-to-cell as primary network

## Tech Stack
- Language: Rust
- OS Base: Custom Linux (mainline kernel)
- UI: Wayland / Phosh (later phases)
- Calling: SIP/VoIP over satellite IP connection
- Architecture: Capability-based (cap-broker already started)

---

## Phase 1 — Core MVP: Make & Receive a Call via Satellite
**Goal: Prove a call works over Starlink. Nothing else.**

- [ ] Minimal Linux environment on Pixel hardware
- [ ] Network interface manager (detect and connect to Starlink IP)
- [ ] VoIP/SIP engine in Rust
- [ ] Audio input/output handling (microphone + speaker)
- [ ] CLI dialer — make and receive calls from terminal
- [ ] Basic call signaling (SIP INVITE, ACK, BYE)

---

## Phase 2 — Connectivity Intelligence Layer
**Goal: Smart network management with ML-based optimization**

- [ ] Multi-link manager (Starlink primary + LTE fallback)
- [ ] Link quality monitoring (latency, packet loss, signal strength)
- [ ] ML model for link quality prediction (orbital position + device motion)
- [ ] Seamless handoff between networks mid-call
- [ ] Telemetry and logging daemon

---

## Phase 3 — Mobile OS Shell
**Goal: A real usable phone experience**

- [ ] Wayland compositor (Phosh or custom)
- [ ] Dialer UI app
- [ ] Contacts manager
- [ ] Notifications system
- [ ] Power management optimization for mobile
- [ ] Camera and sensor support (Pixel-specific drivers)

---

## Phase 4 — Platform & Ecosystem
**Goal: Attract partners, developers, and potential acquirers**

- [ ] Developer connectivity API (abstract satellite/cellular complexity)
- [ ] App sandbox using capability-based security (cap-broker)
- [ ] OTA update system
- [ ] Support additional devices beyond Pixel
- [ ] Enterprise/fleet management features
- [ ] NTN 3GPP standard compliance layer

---

## Key Differentiator
Nobody has built a mobile OS where satellite is the PRIMARY network with intelligent
software managing connectivity. The infrastructure (Starlink, AST SpaceMobile) is maturing
fast — theOS is the software layer that makes it usable as a phone.

## Potential Partners / Acquirers
- SpaceX / Starlink (connectivity infrastructure)
- AST SpaceMobile (direct-to-device satellite)
- Apple (if NTN becomes mainstream on iPhone)
- Enterprise: maritime, remote construction, military
