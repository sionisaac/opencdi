// Host front-end state shared by every platform: key bindings, input modes and the GUI action queue.
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use log::info;
use winit::keyboard::KeyCode as Key;
use winit::event::MouseButton;

pub const ART_SIZE_MIN: u8 = 80;
pub const ART_SIZE_MAX: u8 = 150;
pub const ART_SIZE_DEFAULT: u8 = 100;

/// Per-user application data folder, or the app-root Portable folder when present.
pub fn app_data_dir() -> PathBuf {
    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(PathBuf::from));
    if let Some(portable) = exe_dir.as_ref().map(|p| p.join("Portable")).filter(|p| p.is_dir()) {
        let _ = std::fs::create_dir_all(&portable);
        return portable;
    }
    #[cfg(windows)]
    let root = std::env::var_os("APPDATA").map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join("AppData").join("Roaming")))
        .or(exe_dir.clone()).unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    #[cfg(not(windows))]
    let root = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))
        .or(exe_dir.clone()).unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let dir = root.join("OpenCDI");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Resolves a file in the selected data folder and migrates an older copy from
/// the working/program directory on first access.
pub fn app_data_file(name: &str) -> PathBuf {
    let dir = app_data_dir();
    let target = dir.join(name);
    if !target.exists() {
        let mut old_roots = Vec::new();
        let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(PathBuf::from));
        #[cfg(windows)]
        if exe_dir.as_ref().is_some_and(|p| p.join("Portable").is_dir()) {
            if let Some(appdata) = std::env::var_os("APPDATA") {
                let old = PathBuf::from(appdata).join("OpenCDI");
                if old != dir { old_roots.push(old); }
            }
        }
        if let Some(exe_dir) = exe_dir {
            old_roots.push(exe_dir);
        }
        if let Ok(cwd) = std::env::current_dir() { if !old_roots.contains(&cwd) { old_roots.push(cwd); } }
        #[cfg(windows)]
        if let Some(appdata) = std::env::var_os("APPDATA") {
            let old = PathBuf::from(appdata).join("OpenCDI");
            if old != dir && !old_roots.contains(&old) { old_roots.push(old); }
        }
        for root in old_roots {
            let old = root.join(name);
            if old != target && old.is_file() {
                if let Some(parent) = target.parent() { let _ = std::fs::create_dir_all(parent); }
                if std::fs::copy(&old, &target).is_ok() { break; }
            }
        }
    }
    if let Some(parent) = target.parent() { let _ = std::fs::create_dir_all(parent); }
    target
}

pub fn settings_file_path() -> PathBuf { app_data_file("opencdi_controls.ini") }

pub fn migrate_app_data_dir(name: &str) -> PathBuf {
    let data_dir = app_data_dir();
    let target = data_dir.join(name);
    if !target.exists() {
        let mut roots = Vec::new();
        let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(PathBuf::from));
        #[cfg(windows)]
        if exe_dir.as_ref().is_some_and(|p| p.join("Portable").is_dir()) {
            if let Some(appdata) = std::env::var_os("APPDATA") {
                let old = PathBuf::from(appdata).join("OpenCDI");
                if old != data_dir { roots.push(old); }
            }
        }
        if let Some(exe_dir) = exe_dir { roots.push(exe_dir); }
        if let Ok(cwd) = std::env::current_dir() { if !roots.contains(&cwd) { roots.push(cwd); } }
        #[cfg(windows)]
        if let Some(appdata) = std::env::var_os("APPDATA") {
            let old = PathBuf::from(appdata).join("OpenCDI");
            if old != data_dir && !roots.contains(&old) { roots.push(old); }
        }
        for root in roots {
            let source = root.join(name);
            if source.is_dir() { let _ = copy_tree(&source, &target); if target.exists() { break; } }
        }
    }
    target
}

fn copy_tree(source: &std::path::Path, target: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(target)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let to = target.join(entry.file_name());
        if entry.file_type()?.is_dir() { copy_tree(&entry.path(), &to)?; }
        else { std::fs::copy(entry.path(), to)?; }
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct UiShortcuts {
    pub menu_binding_text: String,
    pub exit_binding_text: String,
    pub pause_binding_text: String,
    pub exit_key: Option<Key>,
    pub menu_key: Option<Key>,
    pub pause_key: Option<Key>,
    pub pause_pad: Vec<gilrs::Button>,
    pub menu_mouse: Option<MouseButton>,
    pub menu_pad: Vec<gilrs::Button>,
    pub exit_pad: Vec<gilrs::Button>,
    pub skip_shell: bool,
    pub nvram_mode: NvramMode,
    pub disc_paths: Vec<PathBuf>,
    pub covers_path: PathBuf,
    pub cd_art_path: PathBuf,
    pub bezels_path: PathBuf,
    pub artwork_size: u8,
    pub input_device: InputDeviceMode,
}

fn artwork_config_path(value: &str) -> PathBuf {
    let path = PathBuf::from(value.trim());
    if !path.is_absolute() && path.components().next().is_some_and(|c| c.as_os_str().to_string_lossy().eq_ignore_ascii_case("Artwork")) {
        return app_data_dir().join(path);
    }
    #[cfg(windows)]
    if let Some(appdata) = std::env::var_os("APPDATA") {
        let previous = PathBuf::from(appdata).join("OpenCDI");
        if previous != app_data_dir() {
            if let Ok(relative) = path.strip_prefix(&previous) { return app_data_dir().join(relative); }
        }
    }
    path
}

fn parse_pad_button(s: &str) -> Option<gilrs::Button> {
    use gilrs::Button::*;
    match s.trim().to_ascii_lowercase().as_str() {
        "south" | "a" | "cross" => Some(South), "east" | "b" | "circle" => Some(East),
        "west" | "x" | "square" => Some(West), "north" | "y" | "triangle" => Some(North),
        "start" => Some(Start), "select" | "back" => Some(Select), "mode" | "guide" | "home" | "guide/home" => Some(Mode),
        "l1" | "leftshoulder" => Some(LeftTrigger), "r1" | "rightshoulder" => Some(RightTrigger),
        "l2" => Some(LeftTrigger2), "r2" => Some(RightTrigger2),
        "l3" | "leftstick" => Some(LeftThumb), "r3" | "rightstick" => Some(RightThumb),
        "up" | "dpup" | "dpadup" => Some(DPadUp), "down" | "dpdown" | "dpaddown" => Some(DPadDown),
        "left" | "dpleft" | "dpadleft" => Some(DPadLeft), "right" | "dpright" | "dpadright" => Some(DPadRight), _ => None,
    }
}

pub fn pad_button_to_string(button: gilrs::Button) -> &'static str {
    use gilrs::Button::*;
    match button {
        South => "South", East => "East", West => "West", North => "North",
        Start => "Start", Select => "Select", Mode => "Guide/Home",
        LeftTrigger => "L1", RightTrigger => "R1", LeftTrigger2 => "L2", RightTrigger2 => "R2",
        LeftThumb => "L3", RightThumb => "R3", DPadUp => "DPadUp", DPadDown => "DPadDown",
        DPadLeft => "DPadLeft", DPadRight => "DPadRight", _ => "Unknown",
    }
}

pub fn string_to_pad_button(s: &str) -> Option<gilrs::Button> { parse_pad_button(s) }

impl Default for UiShortcuts {
    fn default() -> Self {
        let artwork = app_data_dir().join("Artwork");
        let _ = std::fs::create_dir_all(artwork.join("Covers"));
        let _ = std::fs::create_dir_all(artwork.join("CD"));
        let _ = std::fs::create_dir_all(artwork.join("Bezels"));
        Self { menu_binding_text: "F10".into(), exit_binding_text: "Start+Select".into(), pause_binding_text: "F1".into(), exit_key: None, menu_key: Some(Key::F10), pause_key: Some(Key::F1), pause_pad: vec![gilrs::Button::North], menu_mouse: None,
        menu_pad: vec![],
        exit_pad: vec![gilrs::Button::Start, gilrs::Button::Select], skip_shell: true, nvram_mode: NvramMode::Global,
        disc_paths: Vec::new(), covers_path: artwork.join("Covers"), cd_art_path: artwork.join("CD"), bezels_path: artwork.join("Bezels"), artwork_size: ART_SIZE_DEFAULT, input_device: InputDeviceMode::Gamepad }
    }
}

impl UiShortcuts {
    pub fn load() -> Self {
        let mut out = Self::default();
        if let Ok(text) = std::fs::read_to_string(settings_file_path()) {
            for line in text.lines().filter(|l| !l.trim_start().starts_with('#')) {
                let Some((k,v)) = line.split_once('=') else { continue };
                match k.trim().to_ascii_lowercase().as_str() {
                    "menu_toggle" => {
                        out.menu_binding_text = v.trim().to_string();
                        out.menu_key = None; out.menu_mouse = None; out.menu_pad.clear();
                        let value = v.trim();
                        if value.to_ascii_lowercase().starts_with("mouse") {
                            out.menu_mouse = match value.to_ascii_lowercase().as_str() { "mouse1"|"mouseleft" => Some(MouseButton::Left), "mouse2"|"mousemiddle" => Some(MouseButton::Middle), "mouse3"|"mouseright" => Some(MouseButton::Right), _ => None };
                        } else if value.contains('+') { out.menu_pad = value.split('+').filter_map(parse_pad_button).collect(); }
                        else { out.menu_key = string_to_key(value); }
                    }
                    "exit_gamepad_combo" => { out.exit_binding_text = v.trim().to_string(); out.exit_pad = v.split('+').filter_map(parse_pad_button).collect(); },
                    "exit_hotkey" => out.exit_key = string_to_key(v.trim()),
                    "pause_toggle" => {
                        out.pause_binding_text = v.trim().to_string();
                        if let Some(button) = parse_pad_button(v.trim()) { out.pause_pad = vec![button]; }
                        else if let Some(key) = string_to_key(v.trim()) { out.pause_key = Some(key); }
                    },
                    "pause_gamepad" => out.pause_pad = v.split('+').filter_map(parse_pad_button).collect(),
                    "skip_shell" => out.skip_shell = matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"),
                    "nvram_mode" => out.nvram_mode = if v.trim().eq_ignore_ascii_case("per_disc") || v.trim().eq_ignore_ascii_case("per-disc") { NvramMode::PerDisc } else { NvramMode::Global },
                    "disc_path" if !v.trim().is_empty() => out.disc_paths.push(PathBuf::from(v.trim())),
                    "covers_path" if !v.trim().is_empty() => out.covers_path = PathBuf::from(v.trim()),
                    "cd_art_path" if !v.trim().is_empty() => out.cd_art_path = PathBuf::from(v.trim()),
                    "bezels_path" if !v.trim().is_empty() => out.bezels_path = PathBuf::from(v.trim()),
                    "artwork_size" => out.artwork_size = v.trim().parse::<u8>().unwrap_or(ART_SIZE_DEFAULT).clamp(ART_SIZE_MIN, ART_SIZE_MAX),
                    "input_device" => out.input_device = if v.trim().eq_ignore_ascii_case("mouse") { InputDeviceMode::Mouse } else { InputDeviceMode::Gamepad },
                    _ => {}
                }
            }
        }
        out.covers_path = artwork_config_path(&out.covers_path.to_string_lossy());
        out.cd_art_path = artwork_config_path(&out.cd_art_path.to_string_lossy());
        out.bezels_path = artwork_config_path(&out.bezels_path.to_string_lossy());
        out
    }

    pub fn set_pause_key(&mut self, key: Key) { self.pause_key = Some(key); self.pause_binding_text = key_to_string(key).into(); }
    pub fn set_pause_combo(&mut self, buttons: Vec<gilrs::Button>) {
        self.pause_pad = buttons;
    }
    pub fn set_exit_key(&mut self, key: Key) { self.exit_key = Some(key); }
    pub fn set_exit_combo(&mut self, buttons: Vec<gilrs::Button>) {
        self.exit_pad = buttons;
        self.exit_binding_text = self.exit_pad.iter().map(|b| pad_button_to_string(*b)).collect::<Vec<_>>().join("+");
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputProfile {
    Standard, // Frecce + Z / X
    Wasd,     // W, A, S, D + J / K
    Numpad,   // Numpad 8, 4, 6, 2 + 1 / 2
    Custom,   // Personalizzato da opencdi_controls.ini
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputDeviceMode {
    Gamepad, // Controller CD-i (Gamepad USB / Joystick / Tastiera centrate)
    Mouse,   // Mouse Philips CD-i (Cursore Fisico Host)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayMode { BorderlessFullscreen, DecoratedFullscreen, Windowed }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NvramMode { Global, PerDisc }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtworkPathKind { Covers, Cd, Bezels }

pub fn key_to_string(key: Key) -> &'static str {
    match key {
        Key::ArrowUp => "Up",
        Key::ArrowDown => "Down",
        Key::ArrowLeft => "Left",
        Key::ArrowRight => "Right",
        Key::KeyA => "A",
        Key::KeyB => "B",
        Key::KeyC => "C",
        Key::KeyD => "D",
        Key::KeyE => "E",
        Key::KeyF => "F",
        Key::KeyG => "G",
        Key::KeyH => "H",
        Key::KeyI => "I",
        Key::KeyJ => "J",
        Key::KeyK => "K",
        Key::KeyL => "L",
        Key::KeyM => "M",
        Key::KeyN => "N",
        Key::KeyO => "O",
        Key::KeyP => "P",
        Key::KeyQ => "Q",
        Key::KeyR => "R",
        Key::KeyS => "S",
        Key::KeyT => "T",
        Key::KeyU => "U",
        Key::KeyV => "V",
        Key::KeyW => "W",
        Key::KeyX => "X",
        Key::KeyY => "Y",
        Key::KeyZ => "Z",
        Key::Space => "Space",
        Key::Enter => "Enter",
        Key::ShiftLeft => "LeftShift",
        Key::ShiftRight => "RightShift",
        Key::ControlLeft => "LeftCtrl",
        Key::ControlRight => "RightCtrl",
        Key::AltLeft => "LeftAlt",
        Key::AltRight => "RightAlt",
        Key::Tab => "Tab",
        Key::Backspace => "Backspace",
        Key::F1 => "F1", Key::F2 => "F2", Key::F3 => "F3", Key::F4 => "F4", Key::F5 => "F5",
        Key::F6 => "F6", Key::F7 => "F7", Key::F8 => "F8", Key::F9 => "F9", Key::F10 => "F10",
        Key::F11 => "F11", Key::F12 => "F12",
        Key::Numpad0 => "NumPad0",
        Key::Numpad1 => "NumPad1",
        Key::Numpad2 => "NumPad2",
        Key::Numpad3 => "NumPad3",
        Key::Numpad4 => "NumPad4",
        Key::Numpad5 => "NumPad5",
        Key::Numpad6 => "NumPad6",
        Key::Numpad7 => "NumPad7",
        Key::Numpad8 => "NumPad8",
        Key::Numpad9 => "NumPad9",
        _ => "Z",
    }
}

pub fn string_to_key(s: &str) -> Option<Key> {
    let s_upper = s.trim().to_uppercase();
    match s_upper.as_str() {
        "UP" => Some(Key::ArrowUp),
        "DOWN" => Some(Key::ArrowDown),
        "LEFT" => Some(Key::ArrowLeft),
        "RIGHT" => Some(Key::ArrowRight),
        "A" => Some(Key::KeyA),
        "B" => Some(Key::KeyB),
        "C" => Some(Key::KeyC),
        "D" => Some(Key::KeyD),
        "E" => Some(Key::KeyE),
        "F" => Some(Key::KeyF),
        "G" => Some(Key::KeyG),
        "H" => Some(Key::KeyH),
        "I" => Some(Key::KeyI),
        "J" => Some(Key::KeyJ),
        "K" => Some(Key::KeyK),
        "L" => Some(Key::KeyL),
        "M" => Some(Key::KeyM),
        "N" => Some(Key::KeyN),
        "O" => Some(Key::KeyO),
        "P" => Some(Key::KeyP),
        "Q" => Some(Key::KeyQ),
        "R" => Some(Key::KeyR),
        "S" => Some(Key::KeyS),
        "T" => Some(Key::KeyT),
        "U" => Some(Key::KeyU),
        "V" => Some(Key::KeyV),
        "W" => Some(Key::KeyW),
        "X" => Some(Key::KeyX),
        "Y" => Some(Key::KeyY),
        "Z" => Some(Key::KeyZ),
        "SPACE" => Some(Key::Space),
        "ENTER" | "RETURN" => Some(Key::Enter),
        "LEFTSHIFT" | "SHIFT" => Some(Key::ShiftLeft),
        "RIGHTSHIFT" => Some(Key::ShiftRight),
        "LEFTCTRL" | "CTRL" => Some(Key::ControlLeft),
        "RIGHTCTRL" => Some(Key::ControlRight),
        "LEFTALT" | "ALT" => Some(Key::AltLeft),
        "RIGHTALT" => Some(Key::AltRight),
        "TAB" => Some(Key::Tab),
        "BACKSPACE" => Some(Key::Backspace),
        "F1" => Some(Key::F1), "F2" => Some(Key::F2), "F3" => Some(Key::F3), "F4" => Some(Key::F4),
        "F5" => Some(Key::F5), "F6" => Some(Key::F6), "F7" => Some(Key::F7), "F8" => Some(Key::F8),
        "F9" => Some(Key::F9), "F10" => Some(Key::F10), "F11" => Some(Key::F11), "F12" => Some(Key::F12),
        "NUMPAD0" | "KP0" | "0" => Some(Key::Numpad0),
        "NUMPAD1" | "KP1" | "1" => Some(Key::Numpad1),
        "NUMPAD2" | "KP2" | "2" => Some(Key::Numpad2),
        "NUMPAD3" | "KP3" | "3" => Some(Key::Numpad3),
        "NUMPAD4" | "KP4" | "4" => Some(Key::Numpad4),
        "NUMPAD5" | "KP5" | "5" => Some(Key::Numpad5),
        "NUMPAD6" | "KP6" | "6" => Some(Key::Numpad6),
        "NUMPAD7" | "KP7" | "7" => Some(Key::Numpad7),
        "NUMPAD8" | "KP8" | "8" => Some(Key::Numpad8),
        "NUMPAD9" | "KP9" | "9" => Some(Key::Numpad9),
        "ARROWUP" => Some(Key::ArrowUp),
        "ARROWDOWN" => Some(Key::ArrowDown),
        "ARROWLEFT" => Some(Key::ArrowLeft),
        "ARROWRIGHT" => Some(Key::ArrowRight),
        "SHIFTLEFT" => Some(Key::ShiftLeft),
        "SHIFTRIGHT" => Some(Key::ShiftRight),
        "CONTROLLEFT" => Some(Key::ControlLeft),
        "CONTROLRIGHT" => Some(Key::ControlRight),
        "ALTLEFT" => Some(Key::AltLeft),
        "ALTRIGHT" => Some(Key::AltRight),
        // winit-style letter names such as KeyA
        k if k.len() == 4 && k.starts_with("KEY") => string_to_key(&k[3..]),
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub struct KeyBindings {
    pub ui_shortcuts: UiShortcuts,
    pub profile: InputProfile,
    pub up: Key,
    pub down: Key,
    pub left: Key,
    pub right: Key,
    pub btn1: Key,
    pub btn2: Key,
    pub btn3: Key,
    pub pad_up: gilrs::Button,
    pub pad_down: gilrs::Button,
    pub pad_left: gilrs::Button,
    pub pad_right: gilrs::Button,
    pub pad_btn1: gilrs::Button,
    pub pad_btn2: gilrs::Button,
    pub pad_btn3: gilrs::Button,
}

impl KeyBindings {
    pub fn new() -> Self {
        let mut bindings = Self {
            ui_shortcuts: UiShortcuts::load(),
            profile: InputProfile::Standard,
            up: Key::ArrowUp,
            down: Key::ArrowDown,
            left: Key::ArrowLeft,
            right: Key::ArrowRight,
            btn1: Key::KeyZ,
            btn2: Key::KeyX,
            btn3: Key::KeyC,
            pad_up: gilrs::Button::DPadUp,
            pad_down: gilrs::Button::DPadDown,
            pad_left: gilrs::Button::DPadLeft,
            pad_right: gilrs::Button::DPadRight,
            pad_btn1: gilrs::Button::South,
            pad_btn2: gilrs::Button::East,
            pad_btn3: gilrs::Button::West,
        };
        bindings.load_from_ini();
        bindings
    }

    // User choice: apply and write the ini
    pub fn set_profile(&mut self, profile: InputProfile) {
        self.apply_profile(profile);
        self.save_to_ini();
    }

    pub fn set_key_binding(&mut self, row: usize, key: Key) {
        match row {
            0 => self.up = key, 1 => self.down = key, 2 => self.left = key, 3 => self.right = key,
            4 => self.btn1 = key, 5 => self.btn2 = key, 6 => self.btn3 = key, _ => return,
        }
        self.profile = InputProfile::Custom;
    }

    pub fn set_pad_binding(&mut self, row: usize, button: gilrs::Button) {
        match row {
            0 => self.pad_up = button, 1 => self.pad_down = button, 2 => self.pad_left = button, 3 => self.pad_right = button,
            4 => self.pad_btn1 = button, 5 => self.pad_btn2 = button, 6 => self.pad_btn3 = button, _ => return,
        }
        self.profile = InputProfile::Custom;
    }

    pub fn load_disc_config(&mut self, path: &Path) {
        let Ok(text) = std::fs::read_to_string(path) else { return; };
        for line in text.lines() {
            let Some((name, value)) = line.split_once('=') else { continue; };
            let name = name.trim().to_ascii_lowercase(); let value = value.trim();
            match name.as_str() {
                "profile" => self.apply_profile(match value.to_ascii_lowercase().as_str() { "wasd" => InputProfile::Wasd, "numpad" => InputProfile::Numpad, "custom" => InputProfile::Custom, _ => InputProfile::Standard }),
                "up" => if let Some(k)=string_to_key(value){self.up=k}, "down"=>if let Some(k)=string_to_key(value){self.down=k},
                "left"=>if let Some(k)=string_to_key(value){self.left=k}, "right"=>if let Some(k)=string_to_key(value){self.right=k},
                "btn1"=>if let Some(k)=string_to_key(value){self.btn1=k}, "btn2"=>if let Some(k)=string_to_key(value){self.btn2=k}, "btn3"=>if let Some(k)=string_to_key(value){self.btn3=k},
                "pad_up"=>if let Some(b)=string_to_pad_button(value){self.pad_up=b}, "pad_down"=>if let Some(b)=string_to_pad_button(value){self.pad_down=b},
                "pad_left"=>if let Some(b)=string_to_pad_button(value){self.pad_left=b}, "pad_right"=>if let Some(b)=string_to_pad_button(value){self.pad_right=b},
                "pad_btn1"=>if let Some(b)=string_to_pad_button(value){self.pad_btn1=b}, "pad_btn2"=>if let Some(b)=string_to_pad_button(value){self.pad_btn2=b}, "pad_btn3"=>if let Some(b)=string_to_pad_button(value){self.pad_btn3=b},
                "pause_toggle" => { self.ui_shortcuts.pause_binding_text=value.into(); if let Some(button)=parse_pad_button(value){self.ui_shortcuts.pause_pad=vec![button];}else if let Some(key)=string_to_key(value){self.ui_shortcuts.pause_key=Some(key);} },
                "pause_gamepad"=>self.ui_shortcuts.pause_pad=value.split('+').filter_map(parse_pad_button).collect(),
                "exit_hotkey"=>self.ui_shortcuts.exit_key=string_to_key(value),
                "exit_gamepad_combo"=>{self.ui_shortcuts.exit_binding_text=value.into();self.ui_shortcuts.exit_pad=value.split('+').filter_map(parse_pad_button).collect();},
                "input_device"=>self.ui_shortcuts.input_device=if value.eq_ignore_ascii_case("mouse"){InputDeviceMode::Mouse}else{InputDeviceMode::Gamepad},
                _=>{}
            }
        }
    }

    pub fn save_disc_config(&self, path: &Path) {
        if let Some(parent)=path.parent(){let _=std::fs::create_dir_all(parent);}
        let profile=match self.profile{InputProfile::Standard=>"Standard",InputProfile::Wasd=>"WASD",InputProfile::Numpad=>"Numpad",InputProfile::Custom=>"Custom"};
        let pause_gamepad=self.ui_shortcuts.pause_pad.iter().map(|b|pad_button_to_string(*b)).collect::<Vec<_>>().join("+");
        let content=format!("profile={profile}\nup={}\ndown={}\nleft={}\nright={}\nbtn1={}\nbtn2={}\nbtn3={}\npad_up={}\npad_down={}\npad_left={}\npad_right={}\npad_btn1={}\npad_btn2={}\npad_btn3={}\npause_toggle={}\npause_gamepad={}\nexit_hotkey={}\nexit_gamepad_combo={}\ninput_device={}\n",
            key_to_string(self.up),key_to_string(self.down),key_to_string(self.left),key_to_string(self.right),key_to_string(self.btn1),key_to_string(self.btn2),key_to_string(self.btn3),
            pad_button_to_string(self.pad_up),pad_button_to_string(self.pad_down),pad_button_to_string(self.pad_left),pad_button_to_string(self.pad_right),pad_button_to_string(self.pad_btn1),pad_button_to_string(self.pad_btn2),pad_button_to_string(self.pad_btn3),
            self.ui_shortcuts.pause_binding_text,pause_gamepad,self.ui_shortcuts.exit_key.map(key_to_string).unwrap_or("None"),self.ui_shortcuts.exit_binding_text,
            if self.ui_shortcuts.input_device==InputDeviceMode::Mouse{"mouse"}else{"gamepad"});
        let _=std::fs::write(path,content);
    }

    fn apply_profile(&mut self, profile: InputProfile) {
        self.profile = profile;
        match profile {
            InputProfile::Standard => {
                self.up = Key::ArrowUp;
                self.down = Key::ArrowDown;
                self.left = Key::ArrowLeft;
                self.right = Key::ArrowRight;
                self.btn1 = Key::KeyZ;
                self.btn2 = Key::KeyX;
                self.btn3 = Key::KeyC;
            }
            InputProfile::Wasd => {
                self.up = Key::KeyW;
                self.down = Key::KeyS;
                self.left = Key::KeyA;
                self.right = Key::KeyD;
                self.btn1 = Key::KeyJ;
                self.btn2 = Key::KeyK;
                self.btn3 = Key::KeyL;
            }
            InputProfile::Numpad => {
                self.up = Key::Numpad8;
                self.down = Key::Numpad2;
                self.left = Key::Numpad4;
                self.right = Key::Numpad6;
                self.btn1 = Key::Numpad1;
                self.btn2 = Key::Numpad0;
                self.btn3 = Key::Numpad3;
            }
            InputProfile::Custom => {
                // Keep custom key configuration intact
            }
        }
    }

    pub fn save_to_ini(&self) {
        let prof_str = match self.profile {
            InputProfile::Standard => "Standard",
            InputProfile::Wasd => "WASD",
            InputProfile::Numpad => "Numpad",
            InputProfile::Custom => "Custom",
        };
        let disc_paths_ini = self.ui_shortcuts.disc_paths.iter().map(|p| format!("disc_path={}", p.to_string_lossy())).collect::<Vec<_>>().join("\n             ");
        let content = format!(
            "# OpenCDi Controls Configuration\n\
             # Profile: Standard | WASD | Numpad | Custom\n\
             profile={}\n\n\
             # Custom key bindings (active with profile=Custom or as overrides):\n\
             up={}\n\
             down={}\n\
             left={}\n\
             right={}\n\
             btn1={}\n\
             btn2={}\n\
             btn3={}\n\n\
             pad_up={}\n\
             pad_down={}\n\
             pad_left={}\n\
             pad_right={}\n\
             pad_btn1={}\n\
             pad_btn2={}\n\
             pad_btn3={}\n\n\
             # Menu toggle: one keyboard key (F10), mouse button (Mouse1/Mouse2/Mouse3), or gamepad chord (Start+Select)\n\
             menu_toggle={}\n\
             # Gamepad chord that exits the emulator (example: Start+Select+L1+R1)\n\
             exit_gamepad_combo={}\n\
             exit_hotkey={}\n\
             # Pause and resume emulation: one keyboard key or gamepad button (Guide/Home by default)\n\
             pause_toggle={}\n\
             pause_gamepad={}\n\
             # Hide the BIOS shell and automatically start inserted discs (true/false)\n\
             skip_shell={}\n\
             # NVRAM persistence: global or per_disc\n\
             nvram_mode={}\n\
             # Repeat disc_path=... to add scan folders\n\
             {}\n\
             covers_path={}\n\
             cd_art_path={}\n\
             bezels_path={}\n\
             artwork_size={}\n\
             input_device={}\n",
            prof_str,
            key_to_string(self.up),
            key_to_string(self.down),
            key_to_string(self.left),
            key_to_string(self.right),
            key_to_string(self.btn1),
            key_to_string(self.btn2),
            key_to_string(self.btn3),
            pad_button_to_string(self.pad_up),
            pad_button_to_string(self.pad_down),
            pad_button_to_string(self.pad_left),
            pad_button_to_string(self.pad_right),
            pad_button_to_string(self.pad_btn1),
            pad_button_to_string(self.pad_btn2),
            pad_button_to_string(self.pad_btn3),
            self.ui_shortcuts.menu_binding_text,
            self.ui_shortcuts.exit_binding_text,
            self.ui_shortcuts.exit_key.map(key_to_string).unwrap_or("None"),
            self.ui_shortcuts.pause_binding_text,
            self.ui_shortcuts.pause_pad.iter().map(|b|pad_button_to_string(*b)).collect::<Vec<_>>().join("+"),
            self.ui_shortcuts.skip_shell,
            match self.ui_shortcuts.nvram_mode { NvramMode::Global => "global", NvramMode::PerDisc => "per_disc" },
            disc_paths_ini,
            self.ui_shortcuts.covers_path.to_string_lossy(),
            self.ui_shortcuts.cd_art_path.to_string_lossy(),
            self.ui_shortcuts.bezels_path.to_string_lossy(),
            self.ui_shortcuts.artwork_size,
            if self.ui_shortcuts.input_device == InputDeviceMode::Mouse { "mouse" } else { "gamepad" },
        );
        let _ = std::fs::write(settings_file_path(), content);
    }

    pub fn load_from_ini(&mut self) {
        if let Ok(content) = std::fs::read_to_string(settings_file_path()) {
            let mut detected_custom = false;
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
                    continue;
                }
                let parts: Vec<&str> = trimmed.split('=').map(|s| s.trim()).collect();
                if parts.len() == 2 {
                    match parts[0].to_lowercase().as_str() {
                        "profile" => match parts[1].to_lowercase().as_str() {
                            "wasd" => self.apply_profile(InputProfile::Wasd),
                            "numpad" => self.apply_profile(InputProfile::Numpad),
                            "custom" => {
                                self.profile = InputProfile::Custom;
                                detected_custom = true;
                            }
                            _ => self.apply_profile(InputProfile::Standard),
                        },
                        "up" => if let Some(k) = string_to_key(parts[1]) { self.up = k; detected_custom = true; }
                        "down" => if let Some(k) = string_to_key(parts[1]) { self.down = k; detected_custom = true; }
                        "left" => if let Some(k) = string_to_key(parts[1]) { self.left = k; detected_custom = true; }
                        "right" => if let Some(k) = string_to_key(parts[1]) { self.right = k; detected_custom = true; }
                        "btn1" => if let Some(k) = string_to_key(parts[1]) { self.btn1 = k; detected_custom = true; }
                        "btn2" => if let Some(k) = string_to_key(parts[1]) { self.btn2 = k; detected_custom = true; }
                        "btn3" => if let Some(k) = string_to_key(parts[1]) { self.btn3 = k; detected_custom = true; }
                        "pad_up" => if let Some(b) = string_to_pad_button(parts[1]) { self.pad_up = b; }
                        "pad_down" => if let Some(b) = string_to_pad_button(parts[1]) { self.pad_down = b; }
                        "pad_left" => if let Some(b) = string_to_pad_button(parts[1]) { self.pad_left = b; }
                        "pad_right" => if let Some(b) = string_to_pad_button(parts[1]) { self.pad_right = b; }
                        "pad_btn1" => if let Some(b) = string_to_pad_button(parts[1]) { self.pad_btn1 = b; }
                        "pad_btn2" => if let Some(b) = string_to_pad_button(parts[1]) { self.pad_btn2 = b; }
                        "pad_btn3" => if let Some(b) = string_to_pad_button(parts[1]) { self.pad_btn3 = b; }
                        _ => {}
                    }
                }
            }
            if detected_custom && self.profile == InputProfile::Custom {
                info!("Comandi personalizzati caricati da opencdi_controls.ini: up={:?}, down={:?}, left={:?}, right={:?}, btn1={:?}, btn2={:?}",
                    self.up, self.down, self.left, self.right, self.btn1, self.btn2);
            }
        } else {
            self.save_to_ini();
        }
    }
}

pub enum GuiAction {
    InsertDisc(PathBuf),
    OpenDisc(PathBuf),
    EjectDisc,
    Reset,
    Exit,
    SetProfile(InputProfile),
    SetInputDevice(InputDeviceMode),
    ReloadControls,
    OpenControllerConfig,
    ToggleReleaseMouse,
    ToggleMute,
    VolumeUp,
    VolumeDown,
    ToggleFullscreen,
    SetDisplayMode(DisplayMode),
    ToggleSkipShell,
    SetNvramMode(NvramMode),
    AddDiscPath(PathBuf),
    SetArtworkPath(ArtworkPathKind, PathBuf),
    RescanLibrary,
    StopToLibrary,
    TakeScreenshot,
    SetVideoMode(crate::mcd212::VideoMode),
    ToggleWaitStates,
    ToggleDvc,
    ShowHelp,
    ShowAbout,
    QuickSaveState,
    QuickLoadState,
    SaveState(usize),
    LoadState(usize),
    ClearNvram,
    OpenFeedback,
}

static PENDING_ACTIONS: Mutex<Vec<GuiAction>> = Mutex::new(Vec::new());

pub fn pop_action() -> Option<GuiAction> {
    let mut actions = PENDING_ACTIONS.lock().unwrap();
    if actions.is_empty() {
        None
    } else {
        Some(actions.remove(0))
    }
}

pub fn push_action(action: GuiAction) {
    PENDING_ACTIONS.lock().unwrap().push(action);
}

pub const MAX_RECENT_IMAGES: usize = 10;
const RECENT_FILE: &str = "opencdi_recent.txt";

static RECENT_IMAGES: Mutex<Option<Vec<PathBuf>>> = Mutex::new(None);

// Reads opencdi_recent.txt: one path per line, newest first, missing files skipped
fn load_recent_images() -> Vec<PathBuf> {
    let mut list: Vec<PathBuf> = Vec::new();
    if let Ok(content) = std::fs::read_to_string(app_data_file(RECENT_FILE)) {
        for line in content.lines().map(str::trim).filter(|l| !l.is_empty()) {
            let p = PathBuf::from(line);
            if (p.exists() || crate::cdrom::parse_drive_letter(&p).is_some()) && !list.contains(&p) && list.len() < MAX_RECENT_IMAGES {
                list.push(p);
            }
        }
    }
    list
}

fn save_recent_images(list: &[PathBuf]) {
    let content: String = list.iter().take(MAX_RECENT_IMAGES).map(|p| format!("{}\n", p.to_string_lossy())).collect();
    let _ = std::fs::write(app_data_file(RECENT_FILE), content);
}

/// The recent disc images, newest first (at most `MAX_RECENT_IMAGES`).
pub fn recent_images() -> Vec<PathBuf> {
    RECENT_IMAGES.lock().unwrap().get_or_insert_with(load_recent_images).clone()
}

/// Moves `path` to the front of the recent list and saves it.
pub fn add_recent_image(path: &std::path::Path) {
    let mut guard = RECENT_IMAGES.lock().unwrap();
    let list = guard.get_or_insert_with(load_recent_images);
    // Stored absolute so the list works from any working directory; drive letters stay as given
    let p = if crate::cdrom::parse_drive_letter(path).is_some() { path.to_path_buf() } else { std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()) };
    list.retain(|x| x != &p);
    list.insert(0, p);
    list.truncate(MAX_RECENT_IMAGES);
    save_recent_images(list);
}

pub fn clear_recent_images() {
    *RECENT_IMAGES.lock().unwrap() = Some(Vec::new());
    save_recent_images(&[]);
}

/// Window title for a loaded disc: the image name, or the drive for a physical disc.
pub fn disc_display_name(path: &std::path::Path) -> String {
    match crate::cdrom::parse_drive_letter(path) {
        Some(letter) => format!("Real Optical CD Drive ({}:)", letter),
        None => path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "CD-i Disc".to_string()),
    }
}
