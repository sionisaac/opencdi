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
pub struct State { pub row: usize, pub target: Target, pub listening: bool, pub pending_combo: Vec<gilrs::Button> }

impl Default for State {
    fn default() -> Self { Self { row: 0, target: Target::Keyboard, listening: false, pending_combo: Vec::new() } }
}

impl State {
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
    if (27..171).contains(&x) && (124..151).contains(&y) { return Some(Hit::Device(InputDeviceMode::Gamepad)); }
    if (174..318).contains(&x) && (124..151).contains(&y) { return Some(Hit::Device(InputDeviceMode::Mouse)); }
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

pub fn render_native(dst:&mut [u32],width:usize,height:usize,view:(usize,usize,usize,usize),bindings:&KeyBindings,shortcuts:&UiShortcuts,mode:InputDeviceMode,state:&State,gamepad_art:Option<&Artwork>,mouse_art:Option<&Artwork>){
    let mut c=library::NativeCanvas::new(dst,width,height,view);
    for y in 0..library::H { let t=y as u32*8/library::H as u32;c.rect(0,y,library::W,1,0xFF000000|((11+t)<<16)|((20+t)<<8)|(34+t*2)); }
    c.rect(0,0,library::W,42,0xFF0C1726);c.rect(0,41,library::W,1,0xFF26374A);
    c.round_rect(12,8,112,26,7,0xFF1D2D40);c.text(21,15,"<  Back",0xFFD9E4EF,1);
    c.text(20,54,"Controller Settings",0xFFF1F5FA,2);
    c.round_rect(14,92,338,300,12,0xFF131F2E);c.round_rect(364,92,390,300,12,0xFF131F2E);
    c.text(28,106,if mode==InputDeviceMode::Mouse{"CD-i Mouse"}else{"CD-i Gamepad"},0xFFEAF0F6,1);
    c.round_rect(27,124,144,27,7,if mode==InputDeviceMode::Gamepad{0xFF3B9DE5}else{0xFF202D3E});
    c.round_rect(174,124,144,27,7,if mode==InputDeviceMode::Mouse{0xFF3B9DE5}else{0xFF202D3E});
    c.text(68,132,"Gamepad",0xFFF1F6FA,1);c.text(226,132,"Mouse",0xFFF1F6FA,1);
    let art=if mode==InputDeviceMode::Mouse{mouse_art}else{gamepad_art};
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
    c.round_rect(27,354,312,27,7,0xFF202D3E);c.text(41,362,"Philips CD-i input device",0xFFB7C6D5,1);

    c.text(382,106,"Button Mapping",0xFFEAF0F6,1);c.text(481,128,"Keyboard",0xFF91A9C1,1);c.text(631,128,"Controller",0xFF91A9C1,1);
    let names=["UP","DOWN","LEFT","RIGHT","BUTTON I","BUTTON II","BUTTON I/II","PAUSE MENU"];
    let keys=[bindings.up,bindings.down,bindings.left,bindings.right,bindings.btn1,bindings.btn2,bindings.btn3];
    let pads=[bindings.pad_up,bindings.pad_down,bindings.pad_left,bindings.pad_right,bindings.pad_btn1,bindings.pad_btn2,bindings.pad_btn3];
    for row in 0..ROWS{
        let y=145+row*29;let active=state.row==row;
        c.round_rect(374,y,370,26,6,if active{0xFF243B53}else{0xFF1B2939});
        c.text(384,y+7,names[row],if active{0xFF80C4FF}else{0xFFE4EBF2},1);
        let kv=if row==7{shortcuts.pause_key.map(key_to_string).unwrap_or("--")}else{key_to_string(keys[row])};
        let pv=if row==7{shortcuts.pause_pad.first().copied().map(pad_button_to_string).unwrap_or("--")}else{pad_button_to_string(pads[row])};
        native_field(&mut c,472,y+2,120,22,kv,active&&state.target==Target::Keyboard,state.listening&&active&&state.target==Target::Keyboard);
        native_field(&mut c,602,y+2,142,22,pv,active&&state.target==Target::Gamepad,state.listening&&active&&state.target==Target::Gamepad);
    }
    c.round_rect(14,404,338,128,12,0xFF131F2E);c.text(28,418,"Hotkeys",0xFFEAF0F6,1);
    c.text(28,447,"Pause",0xFFB1C0CF,1);native_field(&mut c,102,440,104,27,shortcuts.pause_key.map(key_to_string).unwrap_or("--"),state.listening&&state.target==Target::PauseKey,state.listening&&state.target==Target::PauseKey);
    c.text(222,447,"Exit",0xFFB1C0CF,1);native_field(&mut c,270,440,68,27,shortcuts.exit_key.map(key_to_string).unwrap_or("--"),state.listening&&state.target==Target::ExitKey,state.listening&&state.target==Target::ExitKey);
    c.text(28,486,"Active profile",0xFF839BB2,1);c.text(28,506,profile_name(bindings.profile),0xFFEAF0F6,1);
    c.round_rect(364,404,390,128,12,0xFF131F2E);c.text(380,418,"Button I/II",0xFFDAE9F8,1);
    c.text(380,447,"One press sends Buttons I and II together.",0xFFB7C6D5,1);c.text(380,469,"Assign a key or controller button to trigger both.",0xFF8198AF,1);
    let exit_pad=if state.listening&&state.target==Target::ExitPad{"PRESS COMBO".to_string()}else if shortcuts.exit_pad.is_empty(){"--".into()}else{shortcuts.exit_pad.iter().map(|b|pad_button_to_string(*b)).collect::<Vec<_>>().join("+")};
    c.text(380,497,"Exit controller combo",0xFFB1C0CF,1);native_field(&mut c,530,490,210,27,&exit_pad,state.listening&&state.target==Target::ExitPad,state.listening&&state.target==Target::ExitPad);
}

fn native_field(c:&mut library::NativeCanvas<'_>,x:usize,y:usize,w:usize,h:usize,value:&str,selected:bool,listening:bool){
    c.round_rect(x,y,w,h,5,if listening{0xFF294B68}else if selected{0xFF2B4158}else{0xFF202E3E});
    c.rect(x+6,y,w.saturating_sub(12),1,if listening{0xFF64B8FA}else if selected{0xFF59B2F5}else{0xFF32475D});
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
