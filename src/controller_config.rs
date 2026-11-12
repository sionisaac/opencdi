use opencdi::frontend::{KeyBindings, InputProfile, InputDeviceMode, UiShortcuts, key_to_string, pad_button_to_string};
use crate::library::{self, Artwork};
use winit::keyboard::KeyCode as Key;

pub const ROWS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target { Keyboard, Gamepad, PauseKey, PausePad, ExitKey, ExitPad }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit { Binding(usize, Target), Hotkey(Target), Device(InputDeviceMode), Close }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action { Close, AssignedKey(usize, Key), AssignedPauseKey(Key), AssignedExitKey(Key), Cancelled, Handled }

#[derive(Debug, Clone)]
pub struct State { pub row: usize, pub target: Target, pub listening: bool, pub pending_combo: Vec<gilrs::Button>, gamepad_confirm_ready: bool }

impl Default for State {
    fn default() -> Self { Self { row: 0, target: Target::Keyboard, listening: false, pending_combo: Vec::new(), gamepad_confirm_ready: false } }
}

impl State {
    pub fn confirm_gamepad(&mut self, pressed: bool, previously_pressed: bool) {
        if !pressed { self.gamepad_confirm_ready = true; }
        if self.gamepad_confirm_ready && pressed && !previously_pressed {
            self.listening = true;
        }
    }


    pub fn click(&mut self, hit: Hit) -> Option<Hit> {
        match hit {
            Hit::Binding(row, target) => { self.row = row; self.target = target; self.listening = true; self.pending_combo.clear(); None }
            Hit::Hotkey(target) => { self.target = target; self.listening = true; self.pending_combo.clear(); None }
            Hit::Device(_) => None,
            Hit::Close => { self.listening = false; Some(Hit::Close) }
        }
    }

    pub fn key_pressed(&mut self, key: Key) -> Action {
        if self.listening {
            if key == Key::Escape { self.listening = false; return Action::Cancelled; }
            if self.target == Target::Keyboard { self.listening = false; return Action::AssignedKey(self.row, key); }
            if self.target == Target::PauseKey { self.listening = false; return Action::AssignedPauseKey(key); }
            if self.target == Target::ExitKey { self.listening = false; return Action::AssignedExitKey(key); }
            return Action::Handled;
        }
        match key {
            Key::Escape => Action::Close,
            Key::ArrowUp => { self.row = (self.row + ROWS - 1) % ROWS; Action::Handled }
            Key::ArrowDown => { self.row = (self.row + 1) % ROWS; Action::Handled }
            Key::ArrowLeft | Key::ArrowRight => {
                self.target = if self.target == Target::Keyboard { Target::Gamepad } else { Target::Keyboard };
                Action::Handled
            }
            Key::Enter | Key::NumpadEnter | Key::Space => { self.listening = true; Action::Handled }
            _ => Action::Handled,
        }
    }
}

pub fn hit_test(x: usize, y: usize) -> Option<Hit> {
    if (12..124).contains(&x) && (8..34).contains(&y) { return Some(Hit::Close); }
    if (27..127).contains(&x) && (124..151).contains(&y) { return Some(Hit::Device(InputDeviceMode::Gamepad)); }
    if (132..232).contains(&x) && (124..151).contains(&y) { return Some(Hit::Device(InputDeviceMode::Mouse)); }
    if (237..337).contains(&x) && (124..151).contains(&y) { return Some(Hit::Device(InputDeviceMode::Peacekeeper)); }
    if (472..592).contains(&x) || (602..744).contains(&x) {
        let row = y.checked_sub(145)? / 29;
        if row < ROWS && y < 145 + row * 29 + 24 {
            return Some(Hit::Binding(row, if x < 592 { Target::Keyboard } else { Target::Gamepad }));
        }
    }
    if (102..206).contains(&x) && (440..467).contains(&y) { return Some(Hit::Hotkey(Target::PauseKey)); }
    if (270..338).contains(&x) && (440..467).contains(&y) { return Some(Hit::Hotkey(Target::ExitKey)); }
    if (530..740).contains(&x) && (490..517).contains(&y) { return Some(Hit::Hotkey(Target::ExitPad)); }
    None
}

pub fn load_controller_art() -> Option<Artwork> {
    library::load_png_bytes(include_bytes!("../assets/cdi_gamepad.png"))
}

pub fn load_peacekeeper_art() -> Option<Artwork> {
    library::load_png_bytes(include_bytes!("../assets/cdi_peacekeeper.png"))
}

pub fn load_mouse_art() -> Option<Artwork> { library::load_png_bytes(include_bytes!("../assets/cdi_mouse.png")) }

pub fn render(dst: &mut [u32], bindings: &KeyBindings, shortcuts: &UiShortcuts, mode: InputDeviceMode, state: &State, gamepad_art: Option<&Artwork>, mouse_art: Option<&Artwork>) {
    let w = library::W; let h = library::H;
    dst[..w * h].fill(0xFF0D1521);
    library::rect(dst, 0, 0, w, 48, 0xFF111B29);
    library::text(dst, 20, 14, "CONTROLLER CONFIGURATION", 0xFFF1F4F8, 2);
    library::text(dst, 650, 19, "ESC  DONE", 0xFF9EB0C2, 1);

    library::rect(dst, 14, 62, 338, 330, 0xFF172333);
    library::rect(dst, 14, 62, 338, 32, 0xFF202E40);
    library::text(dst, 26, 74, "CD-I GAMEPAD", 0xFFEAF0F6, 1);
    let art = if mode == InputDeviceMode::Mouse { mouse_art } else { gamepad_art };
    library::text(dst, 27, 102, if mode == InputDeviceMode::Mouse { "CD-I MOUSE" } else { "CD-I GAMEPAD" }, 0xFFEAF0F6, 1);
    if let Some(art) = art { library::blit_fit(dst, art, 27, 122, 312, 198); }
    // Highlight the four way pad and the three face buttons from the supplied artwork.
    if mode == InputDeviceMode::Gamepad {
        ring(dst, 109, 213, 37, 0xFFFFC15B); ring(dst, 226, 232, 16, 0xFFFFC15B);
        ring(dst, 253, 207, 16, 0xFF7BC9F1); ring(dst, 278, 188, 16, 0xFFB49CF4);
    }
    library::rect(dst, 26, 337, 314, 40, 0xFF202E40);
    library::text(dst, 38, 351, "KEYS + GAMEPAD SUPPORTED", 0xFFB7C6D5, 1);

    library::rect(dst, 364, 62, 390, 330, 0xFF172333);
    library::rect(dst, 364, 62, 390, 32, 0xFF202E40);
    library::text(dst, 378, 74, "EMULATED CONTROL", 0xFFAFC0D1, 1);
    library::text(dst, 503, 101, "KEYBOARD", 0xFF8FA2B7, 1);
    library::text(dst, 638, 101, "GAMEPAD", 0xFF8FA2B7, 1);
    let names = ["UP", "DOWN", "LEFT", "RIGHT", "BUTTON I", "BUTTON II", "BUTTON I/II", "PAUSE MENU"];
    let keys = [bindings.up, bindings.down, bindings.left, bindings.right, bindings.btn1, bindings.btn2, bindings.btn3];
    let pads = [bindings.pad_up, bindings.pad_down, bindings.pad_left, bindings.pad_right, bindings.pad_btn1, bindings.pad_btn2, bindings.pad_btn3];
    for row in 0..ROWS {
        let y = 107 + row * 34;
        let active = state.row == row;
        let bg = if active { 0xFF26384D } else { 0xFF1B2939 };
        library::rect(dst, 374, y, 370, 30, bg);
        library::text(dst, 382, y + 10, names[row], if active { 0xFFFFD18A } else { 0xFFE4EBF2 }, 1);
        let key_value = if row == 7 { shortcuts.pause_key.map(key_to_string).unwrap_or("--") } else { key_to_string(keys[row]) };
        let pad_value = if row == 7 { shortcuts.pause_pad.first().copied().map(pad_button_to_string).unwrap_or("--") } else { pad_button_to_string(pads[row]) };
        field(dst, 476, y + 2, 116, 26, key_value, active && state.target == Target::Keyboard, state.listening && active && state.target == Target::Keyboard);
        field(dst, 604, y + 2, 140, 26, pad_value, active && state.target == Target::Gamepad, state.listening && active && state.target == Target::Gamepad);
    }

    library::rect(dst, 364, 402, 390, 78, 0xFF28283A);
    library::rect(dst, 364, 402, 4, 78, 0xFFB49CF4);
    library::text(dst, 378, 415, "BUTTON 3", 0xFFD9CDFA, 1);
    library::text(dst, 378, 433, "BUTTON I/II PRESSES I + II TOGETHER", 0xFFEAF0F6, 1);
    library::text(dst, 378, 453, "Its own key or gamepad button can trigger both.", 0xFFB3A9CB, 1);

    library::rect(dst, 14, 402, 338, 78, 0xFF172333);
    library::text(dst, 28, 416, "ACTIVE PROFILE", 0xFF8FA2B7, 1);
    library::text(dst, 28, 438, profile_name(bindings.profile), 0xFFEAF0F6, 2);
    library::text(dst, 28, 465, "Assignments save automatically", 0xFF91A4B8, 1);

    library::rect(dst, 14, 487, 742, 68, 0xFF111B29);
    library::text(dst, 20, 494, "INPUT", 0xFFB1C0CF, 1);
    field(dst, 90, 489, 88, 18, "GAMEPAD", mode == InputDeviceMode::Gamepad, false); field(dst, 184, 489, 76, 18, "MOUSE", mode == InputDeviceMode::Mouse, false);
    library::text(dst, 278, 519, "EXIT", 0xFFB1C0CF, 1);
    field(dst, 330, 512, 100, 18, shortcuts.exit_key.map(key_to_string).unwrap_or("--"), state.listening && state.target == Target::ExitKey, state.listening && state.target == Target::ExitKey);
    let exit_pad = if state.listening && state.target == Target::ExitPad { "PRESS COMBO".to_string() } else if shortcuts.exit_pad.is_empty() { "--".into() } else { shortcuts.exit_pad.iter().map(|b| pad_button_to_string(*b)).collect::<Vec<_>>().join("+") };
    field(dst, 434, 512, 190, 18, &exit_pad, state.listening && state.target == Target::ExitPad, state.listening && state.target == Target::ExitPad);
    let status = if state.listening { "PRESS INPUT / HOLD CHORD, RELEASE TO SAVE" } else { "DEVICE AND HOTKEYS SAVE AUTOMATICALLY" };
    library::text(dst, 20, 538, status, if state.listening { 0xFFFFD18A } else { 0xFF8397AC }, 1);
    library::text(dst, 650, 538, "ESC: DONE", 0xFFB1C0CF, 1);
}

pub fn render_native(dst:&mut [u32],width:usize,height:usize,view:(usize,usize,usize,usize),bindings:&KeyBindings,shortcuts:&UiShortcuts,mode:InputDeviceMode,state:&State,gamepad_art:Option<&Artwork>,mouse_art:Option<&Artwork>,peacekeeper_art:Option<&Artwork>,wiimote:Option<crate::wiimote::Sample>,notice:&str,calibrating:Option<Option<(f64,f64)>>,automatic:crate::wiimote::ControlMode){
    let mut c=library::NativeCanvas::new(dst,width,height,view);
    for y in 0..library::H { let t=y as u32*8/library::H as u32;c.rect(0,y,library::W,1,0xFF000000|((11+t)<<16)|((20+t)<<8)|(34+t*2)); }
    if let Some(first) = calibrating {
        let (x,y) = if first.is_none() { (115,84) } else { (653,476) };
        c.text(180,28,"Peacekeeper calibration",0xFFF1F5FA,2);
        c.text(175,55,"Aim at the target and press B. Escape cancels.",0xFFB7C6D5,1);
        c.rect(x-12,y-1,25,3,0xFFFFFFFF);
        c.rect(x-1,y-12,3,25,0xFFFFFFFF);
        c.rect(x-3,y-3,7,7,0xFF3B9DE5);
        c.text(220,520,if wiimote.is_some_and(|s| s.aim.is_some()) {
            "Tracking ready - keep the Wiimote upright"
        } else { "Point towards the sensor bar until both lights are visible" },0xFF80C4FF,1);
        return;
    }
    c.rect(0,0,library::W,42,0xFF0C1726);c.rect(0,41,library::W,1,0xFF26374A);
    c.surface(12,8,112,26,7,0xFF263C53,0xFF1B2D42,0xFF3B526B);c.icon(16,22,14,13,0xFFBADAF1);c.text(43,15,"Back",0xFFD9E4EF,1);
    c.text(20,54,"Controller Settings",0xFFF1F5FA,2);
    c.text(382,60,"All controls enabled together",0xFF91B8D8,1);
    c.surface(14,92,338,300,12,0xFF182A3E,0xFF122033,0xFF30455E);c.surface(364,92,390,300,12,0xFF182A3E,0xFF122033,0xFF30455E);
    c.text(28,106,match mode { InputDeviceMode::Mouse => "CD-i Mouse", InputDeviceMode::Gamepad => "CD-i Gamepad", InputDeviceMode::Peacekeeper => "CD-i Peacekeeper" },0xFFEAF0F6,1);
    for (x,device,label) in [
        (27,InputDeviceMode::Gamepad,"Gamepad"),
        (132,InputDeviceMode::Mouse,"Mouse"),
        (237,InputDeviceMode::Peacekeeper,"Peacekeeper"),
    ] {
        let selected=mode==device;
        c.surface(x,124,100,27,7,if selected{0xFF2D506C}else{0xFF24394F},if selected{0xFF23415B}else{0xFF1B2D41},if selected{0xFF67C2FD}else{0xFF3A516A});
        c.icon(match device{InputDeviceMode::Gamepad=>1,InputDeviceMode::Mouse=>19,InputDeviceMode::Peacekeeper=>18},x+7,130,13,if selected{0xFFB5E4FF}else{0xFFA7BED6});
        c.text(x+25,132,label,0xFFF1F6FA,1);
    }
    let art=match mode { InputDeviceMode::Mouse=>mouse_art, InputDeviceMode::Gamepad=>gamepad_art, InputDeviceMode::Peacekeeper=>peacekeeper_art };
    let art_box=(20,158,326,190);
    if let Some(art)=art{c.art_fit(art,art_box.0,art_box.1,art_box.2,art_box.3);}
    if mode==InputDeviceMode::Gamepad{
        // Map ring positions directly from the controller image's source pixels.
        if let Some(art)=art{
            c.ring_art(art,art_box,(204.0,197.0),126.0,0xFFFFC15B);
            c.ring_art(art,art_box,(520.0,244.0),43.0,0xFFFFC15B);
            c.ring_art(art,art_box,(590.0,190.0),41.0,0xFF73C8F2);
            c.ring_art(art,art_box,(661.0,136.0),41.0,0xFFB49CF4);
        }
    }
    c.surface(27,354,312,27,7,0xFF21394F,0xFF1A2D41,0xFF354F69);c.icon(match mode{InputDeviceMode::Gamepad=>1,InputDeviceMode::Mouse=>19,InputDeviceMode::Peacekeeper=>18},37,361,13,0xFF86CAFA);c.text(57,362,if wiimote.is_some() {
        match automatic {
            crate::wiimote::ControlMode::Pointer => "Wiimote - pointing mouse",
            crate::wiimote::ControlMode::Sideways => "Wiimote - sideways controller",
        }
    } else { "Philips CD-i input device" },0xFFB7C6D5,1);

    if mode == InputDeviceMode::Peacekeeper {
        c.text(382,106,"DolphinBar Mode 4",0xFFEAF0F6,1);
        let status = match wiimote {
            None => "Waiting for a Wiimote. Press its SYNC button.",
            Some(sample) if sample.aim.is_none() => "Connected. Aim towards the sensor bar.",
            Some(_) => "Connected - infrared aim detected",
        };
        c.text(382,137,status,0xFF80C4FF,1);
        for (row,(label,value)) in [
            ("Aim","Point the Wiimote at the screen"),
            ("Mouse / Button I","A"),
            ("Button II","B"),
            ("Pause menu","Home"),
            ("Sideways Buttons I / II","1 / 2"),
            ("Library confirm / back","1 / 2 or A / B"),
        ].into_iter().enumerate() {
            let y=172+row*23;
            c.surface(374,y-5,370,21,5,0xFF20364D,0xFF192D42,0xFF2D455E);
            c.text(382,y,label,0xFFB7C6D5,1);
            c.text(535,y,value,0xFFF1F6FA,1);
        }
        c.surface(390,320,340,30,8,0xFF305774,0xFF24455F,0xFF64BFF9);
        c.icon(18,441,328,14,0xFFB9E6FF);c.text(463,330,"Calibrate aim (C)",0xFFEAF0F6,1);
        c.text(382,365,notice,0xFF80C4FF,1);
        c.surface(14,404,740,128,12,0xFF182A3F,0xFF122034,0xFF30465F);
        c.text(28,418,"Peacekeeper controls",0xFFEAF0F6,1);
        c.text(28,445,"Point for mouse control. Hold sideways with the D-pad on the left.",0xFFB7C6D5,1);
        c.text(28,466,"Calibrate from your playing position. A clicks in menus; B returns.",0xFFB7C6D5,1);
        c.text(28,487,"After half a second without tracking, sideways controls take over.",0xFFB7C6D5,1);
        c.text(28,508,"Press Plus and Minus together to exit OpenCDi.",0xFFB7C6D5,1);
        return;
    }

    c.text(382,106,"Button Mapping",0xFFEAF0F6,1);c.text(481,128,"Keyboard",0xFF91A9C1,1);c.text(631,128,"Controller",0xFF91A9C1,1);
    let names=["Up","Down","Left","Right","Button I","Button II","Button I/II","Pause menu"];
    let keys=[bindings.up,bindings.down,bindings.left,bindings.right,bindings.btn1,bindings.btn2,bindings.btn3];
    let pads=[bindings.pad_up,bindings.pad_down,bindings.pad_left,bindings.pad_right,bindings.pad_btn1,bindings.pad_btn2,bindings.pad_btn3];
    for row in 0..ROWS{
        let y=145+row*29;let active=state.row==row;
        c.surface(374,y,370,26,6,if active{0xFF27445E}else{0xFF1E3248},if active{0xFF203B54}else{0xFF172A3F},if active{0xFF426A8D}else{0xFF2C435C});
        c.text(384,y+7,names[row],if active{0xFF80C4FF}else{0xFFE4EBF2},1);
        let kv=if row==7{shortcuts.pause_key.map(key_to_string).unwrap_or("--")}else{key_to_string(keys[row])};
        let pv=if row==7{shortcuts.pause_pad.first().copied().map(pad_button_to_string).unwrap_or("--")}else{pad_button_to_string(pads[row])};
        native_field(&mut c,472,y+2,120,22,kv,active&&state.target==Target::Keyboard,state.listening&&active&&state.target==Target::Keyboard);
        native_field(&mut c,602,y+2,142,22,pv,active&&state.target==Target::Gamepad,state.listening&&active&&state.target==Target::Gamepad);
    }
    c.surface(14,404,338,128,12,0xFF182A3F,0xFF122034,0xFF30465F);c.text(28,418,"Hotkeys",0xFFEAF0F6,1);
    c.text(28,447,"Pause",0xFFB1C0CF,1);native_field(&mut c,102,440,104,27,shortcuts.pause_key.map(key_to_string).unwrap_or("--"),state.listening&&state.target==Target::PauseKey,state.listening&&state.target==Target::PauseKey);
    c.text(222,447,"Exit",0xFFB1C0CF,1);native_field(&mut c,270,440,68,27,shortcuts.exit_key.map(key_to_string).unwrap_or("--"),state.listening&&state.target==Target::ExitKey,state.listening&&state.target==Target::ExitKey);
    c.text(28,486,"Active profile",0xFF839BB2,1);c.text(28,506,profile_name(bindings.profile),0xFFEAF0F6,1);
    c.surface(364,404,390,128,12,0xFF182A3F,0xFF122034,0xFF30465F);c.text(380,418,"Button I/II",0xFFDAE9F8,1);
    c.text(380,447,"One press sends Buttons I and II together.",0xFFB7C6D5,1);c.text(380,469,"Assign a key or controller button to trigger both.",0xFF8198AF,1);
    let exit_pad=if state.listening&&state.target==Target::ExitPad{"PRESS COMBO".to_string()}else if shortcuts.exit_pad.is_empty(){"--".into()}else{shortcuts.exit_pad.iter().map(|b|pad_button_to_string(*b)).collect::<Vec<_>>().join("+")};
    c.text(380,497,"Exit controller combo",0xFFB1C0CF,1);native_field(&mut c,530,490,210,27,&exit_pad,state.listening&&state.target==Target::ExitPad,state.listening&&state.target==Target::ExitPad);
}

fn native_field(c:&mut library::NativeCanvas<'_>,x:usize,y:usize,w:usize,h:usize,value:&str,selected:bool,listening:bool){
    c.surface(x,y,w,h,6,if listening || selected{0xFF30506C}else{0xFF24394F},if listening || selected{0xFF25415C}else{0xFF1B2E43},if listening || selected{0xFF68C4FF}else{0xFF3B526B});
    let label=if listening{"PRESS..."}else{value};c.text(x+5,y+11,&label.chars().take(w/6-1).collect::<String>(),0xFFE8EDF3,1);
}

fn field(dst: &mut [u32], x: usize, y: usize, w: usize, h: usize, value: &str, selected: bool, listening: bool) {
    let color = if listening { 0xFF594626 } else if selected { 0xFF30465D } else { 0xFF243446 };
    library::rect(dst, x, y, w, h, color);
    library::rect(dst, x, y, w, 1, if selected || listening { 0xFFFFC15B } else { 0xFF3A4D61 });
    let label = if listening { "PRESS..." } else { value };
    let max = w / 6 - 1;
    let label: String = label.chars().take(max).collect();
    library::text(dst, x + 5, y + 11, &label, 0xFFE8EDF3, 1);
}

fn ring(dst: &mut [u32], cx: i32, cy: i32, radius: i32, color: u32) {
    for y in -radius..=radius { for x in -radius..=radius {
        let d = x * x + y * y;
        if d <= radius * radius && d >= (radius - 2) * (radius - 2) {
            let px = cx + x; let py = cy + y;
            if px >= 0 && py >= 0 && px < library::W as i32 && py < library::H as i32 {
                dst[py as usize * library::W + px as usize] = color;
            }
        }
    }}
}

fn profile_name(profile: InputProfile) -> &'static str {
    match profile {
        InputProfile::Standard => "STANDARD",
        InputProfile::Wasd => "WASD",
        InputProfile::Numpad => "NUMPAD",
        InputProfile::Custom => "CUSTOM",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peacekeeper_has_its_own_device_target_and_embedded_picture() {
        assert_eq!(hit_test(280,135), Some(Hit::Device(InputDeviceMode::Peacekeeper)));
        assert_eq!(hit_test(180,135), Some(Hit::Device(InputDeviceMode::Mouse)));
        assert_eq!(hit_test(75,135), Some(Hit::Device(InputDeviceMode::Gamepad)));
        let picture = load_peacekeeper_art().expect("embedded Peacekeeper photograph");
        assert!(picture.w > 100 && picture.h > 100);
        if let Ok(path) = std::env::var("OPENCDI_CONTROLLER_PREVIEW") {
            let mut pixels = vec![0u32;1920*1080];
            render_native(&mut pixels,1920,1080,(0,0,1920,1080),
                &KeyBindings::new(),&UiShortcuts::default(),InputDeviceMode::Peacekeeper,&State::default(),
                None,None,Some(&picture),Some(crate::wiimote::Sample { buttons: 0, aim: Some((500.0,400.0)) }),"",None,crate::wiimote::ControlMode::Pointer);
            let bytes: Vec<u8> = pixels.iter().flat_map(|p| [((p>>16)&255) as u8,((p>>8)&255) as u8,(p&255) as u8]).collect();
            let mut encoder = png::Encoder::new(std::fs::File::create(path).unwrap(),1920,1080);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.write_header().unwrap().write_image_data(&bytes).unwrap();
        }
    }

    #[test]
    fn opening_press_requires_release_before_a_new_binding_can_start() {
        let mut state = State::default();
        state.confirm_gamepad(true, false);
        assert!(!state.listening);
        state.confirm_gamepad(true, true);
        assert!(!state.listening);
        state.confirm_gamepad(false, true);
        assert!(!state.listening);
        state.confirm_gamepad(true, false);
        assert!(state.listening);
    }
}
