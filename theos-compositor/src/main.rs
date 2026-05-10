// main.rs -- theOS Wayland Compositor
// Real display server -- no web layer, no browser
// Runs directly on DRM/KMS hardware via Smithay + GLES

#[cfg(feature = "compositor")] mod render;
#[cfg(feature = "compositor")] mod ipc;
#[cfg(feature = "compositor")] mod shell;
#[cfg(feature = "compositor")] mod dialer;
#[cfg(feature = "compositor")] mod assistant;
#[cfg(feature = "compositor")] mod settings;
#[cfg(feature = "compositor")] mod keystore;
#[cfg(feature = "compositor")] mod input;
#[cfg(feature = "compositor")] mod ai_shell;
#[cfg(feature = "compositor")] mod messenger_ui;
mod crypto;
mod dht;
mod identity;
mod hal;

#[cfg(feature = "compositor")]
use render::{RenderPipeline, ActiveSurface, Surface, TouchState, TransitionState};
#[cfg(feature = "compositor")] use shell::Shell;
#[cfg(feature = "compositor")] use dialer::Dialer;
#[cfg(feature = "compositor")] use assistant::Assistant;
#[cfg(feature = "compositor")] use ai_shell::{AiShell, AiShellNav, AiShellState, InputMode};
#[cfg(feature = "compositor")] use input::{InputManager, InputEvent, Gesture, HardwareButton};
#[cfg(feature = "compositor")] use messenger_ui::{MessengerView, MessengerScreen, ConvPreview, BubbleData, DeliveryStateUi};

fn main() {
    println!("theOS Wayland Compositor");
    println!("Satellite-First Mobile OS");

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
    // -- Initialize surfaces --------------------------------------------------
    let mut pipeline  = RenderPipeline::new(1080, 2280);
    let mut shell     = Shell::new();
    let mut dialer    = Dialer::new();
    let mut assistant = Assistant::new();
    let mut ai_shell  = AiShell::new();
    let mut messenger  = MessengerView::new();

    // Seed demo conversations -- replaced by real ContactBook + MessageStore on device
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
        BubbleData {
            text: "The call quality over satellite is incredible".to_string(),
            from_me: false, state: DeliveryStateUi::Delivered,
            appeared_at: 0.2, timestamp: 0,
        },
        BubbleData {
            text: "Starlink beam switch happened mid-call and it stayed connected".to_string(),
            from_me: true, state: DeliveryStateUi::Read,
            appeared_at: 0.3, timestamp: 0,
        },
    ];

    println!("[compositor] surfaces initialized");
    println!("[compositor] ai_shell:  {}", ai_shell.name());
    println!("[compositor] shell:     {}", shell.name());
    println!("[compositor] dialer:    {}", dialer.name());
    println!("[compositor] assistant: {}", assistant.name());

    // -- Initialize input -----------------------------------------------------
    let mut input_mgr = InputManager::new().ok();
    if input_mgr.is_none() {
        println!("[compositor] input: running without hardware (demo mode)");
    }

    // -- Boot sequence --------------------------------------------------------
    println!("[compositor] boot -> lock screen");
    pipeline.navigate(ActiveSurface::Lock);

    // Simulate unlock for demo -- on real hardware this waits for power button
    std::thread::sleep(std::time::Duration::from_millis(500));
    shell.unlock();
    pipeline.navigate(ActiveSurface::AiShell);
    println!("[compositor] unlocked -> AI orb");

    // -- Push initial system state into AI shell ------------------------------
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

    // -- Transition state (slide-up animation between surfaces) ---------------
    let mut transition: Option<TransitionState> = None;

    // -- Frame loop -----------------------------------------------------------
    // Target: 60fps. Each frame:
    //   1. Poll input
    //   2. Process AI shell navigation events
    //   3. Advance transition animation
    //   4. Draw the active surface (or transition)
    //
    // On real hardware this is driven by DRM vblank. In demo mode we use
    // a simple sleep loop.

    let frame_budget = std::time::Duration::from_millis(16); // ~60fps

    // Demo: run for 300 frames then exit (5 seconds at 60fps)
    // On real hardware: loop forever until power-off
    let demo_mode = std::env::args().any(|a| a == "--demo");
    let max_frames: u64 = if demo_mode { 300 } else { u64::MAX };

    println!("[compositor] entering frame loop (demo_mode: {})", demo_mode);

    for frame in 0..max_frames {
        let t = now_secs();
        let frame_start = std::time::Instant::now();

        // -- 1. Poll input ----------------------------------------------------
        let events: Vec<InputEvent> = input_mgr
            .as_mut()
            .map(|im| im.poll())
            .unwrap_or_default();

        for event in &events {
            match event {
                InputEvent::Gesture(Gesture::SwipeDown) => {
                    // Escape hatch: swipe down from AI shell -> traditional home
                    if pipeline.active_surface == ActiveSurface::AiShell {
                        let ts = TransitionState::new(
                            ActiveSurface::AiShell,
                            ActiveSurface::Home,
                            t,
                        );
                        transition = Some(ts);
                        println!("[compositor] swipe down -> traditional home");
                    }
                }
                InputEvent::Gesture(Gesture::SwipeUp) => {
                    // Swipe up from home -> back to AI shell
                    if pipeline.active_surface == ActiveSurface::Home {
                        let ts = TransitionState::new(
                            ActiveSurface::Home,
                            ActiveSurface::AiShell,
                            t,
                        );
                        transition = Some(ts);
                        println!("[compositor] swipe up -> AI shell");
                    }
                }
                InputEvent::Button(HardwareButton::Power) => {
                    println!("[compositor] power button -> lock");
                    pipeline.navigate(ActiveSurface::Lock);
                    transition = None;
                }
                InputEvent::TouchDown { x, y } => {
                    match pipeline.active_surface {
                        ActiveSurface::AiShell => {
                            ai_shell.handle_touch(*x, *y, TouchState::Down);
                        }
                        ActiveSurface::Home => {
                            shell.handle_touch(*x, *y, TouchState::Down);
                        }
                        ActiveSurface::Dialer => {
                            dialer.handle_touch(*x, *y, TouchState::Down);
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        // -- 2. Process AI shell navigation -----------------------------------
        // In the real compositor this is driven by the keyboard/touch input
        // feeding text into ai_shell.input_buf. For the demo we simulate
        // a sequence of intents.
        let nav = if frame == 60 {
            // Frame 1s: simulate "show connection"
            ai_shell.input_buf  = "show connection".to_string();
            ai_shell.input_mode = InputMode::Typing;
            ai_shell.submit_input()
        } else if frame == 150 {
            // Frame 2.5s: simulate "message sarah"
            ai_shell.input_buf  = "message sarah".to_string();
            ai_shell.input_mode = InputMode::Typing;
            ai_shell.submit_input()
        } else if frame == 240 {
            // Frame 4s: simulate "call marcus"
            ai_shell.input_buf  = "call marcus".to_string();
            ai_shell.input_mode = InputMode::Typing;
            ai_shell.submit_input()
        } else {
            AiShellNav::None
        };

        // Route navigation to surface transitions
        match nav {
            AiShellNav::GoToDialer { ref contact } => {
                println!("[compositor] AI -> dialer for '{}'", contact);
                let ts = TransitionState::new(ActiveSurface::AiShell, ActiveSurface::Dialer, t);
                transition = Some(ts);
            }
            AiShellNav::GoToMessages => {
                println!("[compositor] AI -> messenger");
                messenger.open_conversation("sarah_chen_key".to_string());
                let ts = TransitionState::new(ActiveSurface::AiShell, ActiveSurface::Messenger, t);
                transition = Some(ts);
            }
            AiShellNav::GoToSettings => {
                println!("[compositor] AI -> settings");
                let ts = TransitionState::new(ActiveSurface::AiShell, ActiveSurface::Settings, t);
                transition = Some(ts);
            }
            AiShellNav::GoToTraditionalHome => {
                let ts = TransitionState::new(ActiveSurface::AiShell, ActiveSurface::Home, t);
                transition = Some(ts);
            }
            AiShellNav::None => {}
        }

        // -- 3. Advance transition --------------------------------------------
        if let Some(ref ts) = transition {
            if ts.is_complete(t) {
                // Transition done -- snap to final surface
                let to = ts.to;
                pipeline.navigate(to);
                transition = None;
                println!("[compositor] transition complete -> {:?}", to);
            }
        }

        // -- 4. Draw ----------------------------------------------------------
        // (In demo mode we just log -- real GLES draw calls happen here
        //  when running on PostmarketOS with a DRM/KMS backend)

        if let Some(ref ts) = transition {
            let incoming_y = ts.incoming_y(pipeline.height, t);
            println!(
                "[compositor] frame:{} transition {:?}->{:?} y:{} progress:{:.2}",
                frame, ts.from, ts.to, incoming_y,
                ts.progress(t)
            );
        } else {
            match pipeline.active_surface {
                ActiveSurface::AiShell => {
                    println!(
                        "[compositor] frame:{} orb t:{:.2} msgs:{}",
                        frame, t, ai_shell.messages.len()
                    );
                }
                ActiveSurface::Dialer => {
                    println!("[compositor] frame:{} dialer", frame);
                }
                ActiveSurface::Messenger => {
                    // Draw conversation list or open conversation
                    match &messenger.screen {
                        MessengerScreen::ConversationList => {
                            pipeline.draw_conversation_list(
                                &mut frame_placeholder,
                                &demo_convs,
                                t,
                            );
                        }
                        MessengerScreen::Conversation { .. } => {
                            pipeline.draw_conversation(
                                &mut frame_placeholder,
                                "Sarah Chen",
                                &demo_bubbles,
                                false,
                                messenger.scroll_y,
                                t,
                            );
                        }
                    }
                    println!("[compositor] frame:{} messenger screen:{:?}", frame, messenger.screen);
                }
                ActiveSurface::Home => {
                    println!("[compositor] frame:{} traditional home", frame);
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
}
