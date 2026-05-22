// main.rs -- theOS Wayland Compositor
// Voice-first. No buttons. No visible UI chrome.
// The AI orb is the only interface.

#[cfg(feature = "compositor")] mod render;
#[cfg(feature = "compositor")] mod ipc;
#[cfg(feature = "compositor")] mod shell;
#[cfg(feature = "compositor")] mod assistant;
#[cfg(feature = "compositor")] mod settings;
#[cfg(feature = "compositor")] mod keystore;
#[cfg(feature = "compositor")] mod input;
#[cfg(feature = "compositor")] mod ai_shell;
#[cfg(feature = "compositor")] mod messenger_ui;
#[cfg(feature = "compositor")] mod drm_backend;
mod crypto;
mod dht;
mod identity;
mod hal;

#[cfg(feature = "compositor")]
use render::{RenderPipeline, ActiveSurface, Surface, TouchState, TransitionState, OrbState};
#[cfg(feature = "compositor")] use shell::Shell;
#[cfg(feature = "compositor")] use assistant::Assistant;
#[cfg(feature = "compositor")] use ai_shell::{AiShell, AiShellNav, AiShellState, InputMode};
#[cfg(feature = "compositor")] use input::{InputManager, InputEvent, Gesture, HardwareButton};
#[cfg(feature = "compositor")] use messenger_ui::{MessengerView, MessengerScreen, ConvPreview, BubbleData, DeliveryStateUi};

// theos-core voice + power
use theos_core::wake_word::{WakeEngine, WakeState, strip_wake_word, contains_wake_word};
use theos_core::power::{PowerManager, PowerState};

fn main() {
    println!("theOS Wayland Compositor");
    println!("Satellite-First Mobile OS");
    println!("Voice-First Interface -- no buttons");

    #[cfg(not(feature = "compositor"))]
    {
        println!("[compositor] built without compositor feature");
        return;
    }

    #[cfg(feature = "compositor")]
    run_compositor();
}

#[cfg(feature = "compositor")]
fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

#[cfg(feature = "compositor")]
fn run_compositor() {
    // -- Initialize core systems ----------------------------------------------
    let mut wake    = WakeEngine::new();
    let mut power   = PowerManager::new();

    // -- Initialize surfaces --------------------------------------------------
    let mut pipeline  = RenderPipeline::new(1080, 2280);
    let mut shell     = Shell::new();
    let mut assistant = Assistant::new();
    let mut ai_shell  = AiShell::new();
    let mut messenger = MessengerView::new();

    let demo_convs = vec![
        ConvPreview {
            contact_name: "Sarah Chen".to_string(),
            preview_text: "Sounds good, see you then!".to_string(),
            unread: 2, online: true, typing: false, timestamp: 0,
        },
        ConvPreview {
            contact_name: "Marcus Webb".to_string(),
            preview_text: "the call quality was perfect".to_string(),
            unread: 0, online: true, typing: true, timestamp: 0,
        },
    ];

    let demo_bubbles = vec![
        BubbleData {
            text: "Hey! Got your connection request".to_string(),
            from_me: false, state: DeliveryStateUi::Delivered,
            appeared_at: 0.0, timestamp: 0,
        },
        BubbleData {
            text: "First theOS contact! No SIM, no carrier".to_string(),
            from_me: true, state: DeliveryStateUi::Read,
            appeared_at: 0.1, timestamp: 0,
        },
    ];

    println!("[compositor] surfaces initialized");

    // -- Initialize input -----------------------------------------------------
    let mut input_mgr = InputManager::new().ok();
    if input_mgr.is_none() {
        println!("[compositor] input: demo mode (no hardware)");
    }

    // -- Boot: straight to orb ------------------------------------------------
    // No lock screen. Identity is the device. Wake word is the key.
    pipeline.navigate(ActiveSurface::AiShell);
    println!("[compositor] boot -> AI orb (voice-first)");

    // -- Push system state into AI shell --------------------------------------
    ai_shell.update_state(AiShellState {
        handle:         "omar".to_string(),
        pubkey_hex:     "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
        satellite_link: shell.satellite_link.clone(),
        latency_ms:     shell.latency_ms,
        signal_quality: shell.signal_quality,
        dht_peers:      0,
        contact_count:  0,
        battery_pct:    shell.battery_pct,
        charging:       shell.charging,
    });

    // -- Frame loop state -----------------------------------------------------
    let mut transition:   Option<TransitionState> = None;
    let mut show_text_input = false; // hidden by default, tap orb to show
    let frame_budget = std::time::Duration::from_millis(16);

    let demo_mode  = std::env::args().any(|a| a == "--demo");
    let max_frames = if demo_mode { 300u64 } else { u64::MAX };

    println!("[compositor] frame loop -- voice-first -- demo:{}", demo_mode);

    for frame in 0..max_frames {
        let t          = now_secs();
        let frame_start = std::time::Instant::now();

        // -- 1. Poll hardware input -------------------------------------------
        let events: Vec<InputEvent> = input_mgr
            .as_mut()
            .map(|im| im.poll())
            .unwrap_or_default();

        for event in &events {
            match event {

                // Power button: toggle sleep
                InputEvent::Button(HardwareButton::Power) => {
                    if power.is_sleeping() {
                        power.wake();
                        wake.force_sleep(); // reset wake engine on manual wake
                        pipeline.navigate(ActiveSurface::AiShell);
                        println!("[compositor] power button -> wake");
                    } else {
                        power.sleep();
                        wake.force_sleep();
                        println!("[compositor] power button -> sleep");
                    }
                }

                // Tap orb -> show text input (accessibility fallback)
                InputEvent::TouchDown { x, y } => {
                    power.on_interaction();
                    let cx = pipeline.width  / 2;
                    let cy = pipeline.height / 2;
                    let dx = (*x as i32 - cx).abs();
                    let dy = (*y as i32 - cy).abs();
                    if dx < 80 && dy < 80 {
                        // Tapped the orb
                        show_text_input = !show_text_input;
                        println!("[compositor] orb tap -> text input: {}", show_text_input);
                    } else {
                        // Tap anywhere else -> dismiss text input
                        show_text_input = false;
                    }
                }

                // Keyboard input feeds text input when shown
                InputEvent::KeyPress { ch } => {
                    if show_text_input {
                        ai_shell.input_mode = InputMode::Typing;
                        ai_shell.type_char(*ch);
                    }
                }
                InputEvent::KeyBackspace => {
                    if show_text_input { ai_shell.backspace(); }
                }
                InputEvent::KeyEnter => {
                    if show_text_input {
                        ai_shell.input_mode = InputMode::Typing;
                        let nav = ai_shell.submit_input();
                        show_text_input = false;
                        handle_nav(nav, &mut transition, &mut messenger, t);
                    }
                }
                InputEvent::KeyEscape => {
                    show_text_input = false;
                    // Escape from any surface -> back to orb
                    if pipeline.active_surface != ActiveSurface::AiShell {
                        let ts = TransitionState::new(
                            pipeline.active_surface,
                            ActiveSurface::AiShell,
                            t,
                        );
                        transition = Some(ts);
                    }
                }

                _ => {}
            }
        }

        // -- 2. Wake engine tick ----------------------------------------------
        // On real device: ADSP fires on_wake_detected() via fastrpc interrupt.
        // In demo mode: simulate wake at frame 60, command at frame 90.
        if demo_mode {
            if frame == 60 {
                println!("[compositor] [demo] simulating: Hey OS, message Sarah");
                wake.on_wake_detected();
            }
            if frame == 90 {
                wake.on_command_received("hey os, message sarah".to_string());
            }
            if frame == 180 {
                println!("[compositor] [demo] simulating: Hey OS, call Marcus");
                wake.on_wake_detected();
            }
            if frame == 210 {
                wake.on_command_received("hey os, call marcus".to_string());
            }
        }

        wake.tick();

        // Route wake engine commands to navigation
        if let WakeState::Processing { .. } = &wake.state.clone() {
            if let Some(command) = wake.current_command() {
                let command = command.to_string();
                ai_shell.input_buf  = command.clone();
                ai_shell.input_mode = InputMode::Typing;
                let nav = ai_shell.submit_input();
                wake.on_command_executed();

                // Wake the screen if sleeping
                if power.is_sleeping() {
                    power.wake();
                    pipeline.navigate(ActiveSurface::AiShell);
                }

                handle_nav(nav, &mut transition, &mut messenger, t);
            }
            wake.on_response_complete();
        }

        // -- 3. Power tick (once per second) ----------------------------------
        if frame % 60 == 0 {
            power.tick();

            // Screen off when sleeping
            if power.is_sleeping() && pipeline.active_surface != ActiveSurface::Lock {
                pipeline.navigate(ActiveSurface::Lock);
            }
        }

        // -- 4. Transition advance --------------------------------------------
        if let Some(ref ts) = transition {
            if ts.is_complete(t) {
                let to = ts.to;
                pipeline.navigate(to);
                transition = None;
                println!("[compositor] transition -> {:?}", to);
            }
        }

        // -- 5. Draw ----------------------------------------------------------
        // Orb state drives the visual
        let orb_state = match &wake.state {
            WakeState::Sleeping        => OrbState::Passive,
            WakeState::Listening {..}  => OrbState::Listening,
            WakeState::Processing {..} => OrbState::Processing,
            WakeState::Responding      => OrbState::Responding,
        };

        if power.is_sleeping() {
            // Screen off -- nothing to draw
        } else if let Some(ref ts) = transition {
            let incoming_y = ts.incoming_y(pipeline.height, t);
            println!(
                "[compositor] frame:{} transition {:?}->{:?} y:{} orb:{:?}",
                frame, ts.from, ts.to, incoming_y, orb_state
            );
        } else {
            match pipeline.active_surface {
                ActiveSurface::AiShell => {
                    println!(
                        "[compositor] frame:{} orb:{:?} wake:{} power:{} text_input:{}",
                        frame,
                        orb_state,
                        wake.state.label(),
                        power.state.label(),
                        show_text_input,
                    );
                }
                ActiveSurface::Messenger => {
                    println!("[compositor] frame:{} messenger", frame);
                }
                _ => {
                    println!("[compositor] frame:{} {:?}", frame, pipeline.active_surface);
                }
            }
        }

        // -- Frame timing -----------------------------------------------------
        let elapsed = frame_start.elapsed();
        if elapsed < frame_budget {
            std::thread::sleep(frame_budget - elapsed);
        }
    }

    println!("[compositor] frame loop complete");
    println!("[compositor] stats: wakes={} false_wakes={} power={}",
        wake.total_wakes, wake.false_wakes, power.state.label());
}

// -- Navigation handler -------------------------------------------------------

#[cfg(feature = "compositor")]
fn handle_nav(
    nav:        AiShellNav,
    transition: &mut Option<TransitionState>,
    messenger:  &mut MessengerView,
    t:          f64,
) {
    match nav {
        AiShellNav::GoToCall { ref contact } => {
            println!("[compositor] voice -> call: {}", contact);
            *transition = Some(TransitionState::new(
                ActiveSurface::AiShell, ActiveSurface::Call, t
            ));
        }
        AiShellNav::GoToMessages => {
            println!("[compositor] voice -> messenger");
            messenger.open_conversation("key".to_string());
            *transition = Some(TransitionState::new(
                ActiveSurface::AiShell, ActiveSurface::Messenger, t
            ));
        }
        AiShellNav::GoToSettings => {
            println!("[compositor] voice -> settings");
            *transition = Some(TransitionState::new(
                ActiveSurface::AiShell, ActiveSurface::Settings, t
            ));
        }
        AiShellNav::GoToTraditionalHome => {
            *transition = Some(TransitionState::new(
                ActiveSurface::AiShell, ActiveSurface::Home, t
            ));
        }
        AiShellNav::None => {}
    }
}
