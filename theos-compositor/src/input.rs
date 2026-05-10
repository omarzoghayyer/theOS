// input.rs -- theOS Input Handler
use input::{
    Libinput, LibinputInterface,
    event::{
        Event, TouchEvent, KeyboardEvent,
        touch::TouchEventPosition,
        keyboard::KeyboardEventTrait,
    },
};
use std::fs::{File, OpenOptions};
use std::os::unix::{fs::OpenOptionsExt, io::{FromRawFd, IntoRawFd, OwnedFd}};
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub enum Gesture {
    SwipeUp, SwipeDown, SwipeLeft, SwipeRight,
    Tap { x: f64, y: f64 },
}

#[derive(Debug, Clone, PartialEq)]
pub enum HardwareButton { Power, VolumeUp, VolumeDown }

#[derive(Debug, Clone)]
pub enum InputEvent {
    Gesture(Gesture),
    Button(HardwareButton),
    TouchDown { x: f64, y: f64 },
    TouchUp   { x: f64, y: f64 },
    TouchMove { x: f64, y: f64 },
    KeyPress  { ch: char },
    KeyBackspace,
    KeyEnter,
    KeyEscape,
}

pub struct TheOsInterface;

impl LibinputInterface for TheOsInterface {
    fn open_restricted(&mut self, path: &Path, flags: i32) -> Result<OwnedFd, i32> {
        OpenOptions::new()
            .custom_flags(flags)
            .read(true)
            .open(path)
            .map(|f| unsafe { OwnedFd::from_raw_fd(f.into_raw_fd()) })
            .map_err(|e| e.raw_os_error().unwrap_or(1))
    }
    fn close_restricted(&mut self, fd: OwnedFd) { drop(fd); }
}

pub struct GestureDetector {
    start_x: f64, start_y: f64,
    start_time: std::time::Instant,
    active: bool,
}

impl GestureDetector {
    pub fn new() -> Self {
        Self {
            start_x: 0.0, start_y: 0.0,
            start_time: std::time::Instant::now(),
            active: false,
        }
    }

    pub fn down(&mut self, x: f64, y: f64) {
        self.start_x = x; self.start_y = y;
        self.start_time = std::time::Instant::now();
        self.active = true;
    }

    pub fn up(&mut self, x: f64, y: f64) -> Option<Gesture> {
        if !self.active { return None; }
        self.active = false;
        let dx = x - self.start_x;
        let dy = y - self.start_y;
        let dist = (dx*dx + dy*dy).sqrt();
        let ms = self.start_time.elapsed().as_millis();
        if dist < 20.0 && ms < 300 {
            return Some(Gesture::Tap { x: self.start_x, y: self.start_y });
        }
        if dist > 50.0 {
            return Some(if dy.abs() > dx.abs() {
                if dy < 0.0 { Gesture::SwipeUp } else { Gesture::SwipeDown }
            } else {
                if dx < 0.0 { Gesture::SwipeLeft } else { Gesture::SwipeRight }
            });
        }
        None
    }
}

pub struct InputManager {
    pub libinput: Libinput,
    pub gesture:  GestureDetector,
    last_x: f64,
    last_y: f64,
}

impl InputManager {
    pub fn new() -> Result<Self, String> {
        let mut li = Libinput::new_with_udev(TheOsInterface);
        li.udev_assign_seat("seat0").map_err(|_| "seat error".to_string())?;
        Ok(Self {
            libinput: li,
            gesture:  GestureDetector::new(),
            last_x:   0.0,
            last_y:   0.0,
        })
    }

    pub fn poll(&mut self) -> Vec<InputEvent> {
        let mut events = Vec::new();
        self.libinput.dispatch().ok();
        for event in &mut self.libinput {
            match event {
                Event::Touch(t) => match t {
                    TouchEvent::Down(e) => {
                        let x = e.x() * 1080.0;
                        let y = e.y() * 2280.0;
                        self.last_x = x; self.last_y = y;
                        self.gesture.down(x, y);
                        events.push(InputEvent::TouchDown { x, y });
                    }
                    TouchEvent::Up(_) => {
                        if let Some(g) = self.gesture.up(self.last_x, self.last_y) {
                            events.push(InputEvent::Gesture(g));
                        }
                        events.push(InputEvent::TouchUp { x: self.last_x, y: self.last_y });
                    }
                    TouchEvent::Motion(e) => {
                        let x = e.x() * 1080.0;
                        let y = e.y() * 2280.0;
                        self.last_x = x; self.last_y = y;
                        events.push(InputEvent::TouchMove { x, y });
                    }
                    _ => {}
                },
                Event::Keyboard(k) => {
                    let code = k.key();
                    match code {
                        116 => events.push(InputEvent::Button(HardwareButton::Power)),
                        115 => events.push(InputEvent::Button(HardwareButton::VolumeUp)),
                        114 => events.push(InputEvent::Button(HardwareButton::VolumeDown)),
                        14  => events.push(InputEvent::KeyBackspace),
                        28  => events.push(InputEvent::KeyEnter),
                        96  => events.push(InputEvent::KeyEnter),
                        1   => events.push(InputEvent::KeyEscape),
                        _   => {
                            if let Some(ch) = keycode_to_char(code) {
                                events.push(InputEvent::KeyPress { ch });
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        events
    }
}

pub fn keycode_to_char(code: u32) -> Option<char> {
    let chars: &[(u32, char)] = &[
        (2,'1'),(3,'2'),(4,'3'),(5,'4'),(6,'5'),
        (7,'6'),(8,'7'),(9,'8'),(10,'9'),(11,'0'),
        (16,'q'),(17,'w'),(18,'e'),(19,'r'),(20,'t'),
        (21,'y'),(22,'u'),(23,'i'),(24,'o'),(25,'p'),
        (30,'a'),(31,'s'),(32,'d'),(33,'f'),(34,'g'),
        (35,'h'),(36,'j'),(37,'k'),(38,'l'),
        (44,'z'),(45,'x'),(46,'c'),(47,'v'),(48,'b'),
        (49,'n'),(50,'m'),(57,' '),(12,'-'),(13,'='),
        (26,'['),(27,']'),(39,';'),(51,','),(52,'.'),(53,'/'),
    ];
    chars.iter().find(|(k,_)| *k == code).map(|(_,c)| *c)
}
