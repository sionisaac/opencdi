//! Chooses who moves the single CD-i pointer without excluding other buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControllerOptions {
    pub gamepad: bool,
    pub mouse: bool,
    pub peacekeeper: bool,
    pub mouse_sensitivity: u16,
}
impl Default for ControllerOptions {
    fn default()->Self {Self{gamepad:true,mouse:true,peacekeeper:true,mouse_sensitivity:100}}
}
impl ControllerOptions {
    pub fn pointing_enabled(self,gun_profile:bool)->bool {if gun_profile{self.peacekeeper}else{self.mouse}}
    pub fn buttons(self,active:bool,mouse:u8,pad:u8,pointing:u8,gun_profile:bool)->u8 {
        combined_buttons(active,if self.mouse{mouse}else{0},if self.gamepad{pad}else{0},if self.pointing_enabled(gun_profile){pointing}else{0})
    }
    pub fn load_setting(&mut self,key:&str,value:&str) {
        let enabled=match value.to_ascii_lowercase().as_str(){"true"|"1"=>Some(true),"false"|"0"=>Some(false),_=>None};
        match key {
            "control_gamepad"=>if let Some(v)=enabled{self.gamepad=v},
            "control_mouse"=>if let Some(v)=enabled{self.mouse=v},
            "control_peacekeeper"=>if let Some(v)=enabled{self.peacekeeper=v},
            "mouse_sensitivity"=>if let Ok(v)=value.parse::<u16>(){self.mouse_sensitivity=v.clamp(25,150)},
            _=>{}
        }
    }
    pub fn settings(&self)->String {
        format!("control_gamepad={}\ncontrol_mouse={}\ncontrol_peacekeeper={}\nmouse_sensitivity={}\n",self.gamepad,self.mouse,self.peacekeeper,self.mouse_sensitivity.clamp(25,150))
    }
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum PointerSource { #[default] Mouse, Gamepad, Wiimote }

#[derive(Debug, Default)]
pub struct PointerArbiter {
    source: PointerSource,
    remote_anchor: Option<(i32,i32)>,
    was_pointing: bool,
}

impl PointerArbiter {
    pub fn source(&self) -> PointerSource { self.source }

    pub fn remote_mode(&mut self, pointing: bool) {
        if pointing && !self.was_pointing { self.remote_anchor = None; }
        self.was_pointing = pointing;
    }

    pub fn pointers(&mut self, mouse_activity: bool, remote: Option<(i32,i32)>, remote_press: bool) {
        if mouse_activity {
            self.source = PointerSource::Mouse;
            if remote.is_some() { self.remote_anchor = remote; }
            return;
        }
        if let Some(p) = remote {
            let deliberate = self.remote_anchor.is_none_or(|a|
                (p.0-a.0).abs().max((p.1-a.1).abs()) >= 16);
            if self.source == PointerSource::Wiimote || remote_press || deliberate {
                self.source = PointerSource::Wiimote;
                self.remote_anchor = Some(p);
            }
        }
    }

    pub fn gamepad_motion(&mut self, remote: Option<(i32,i32)>) {
        self.source = PointerSource::Gamepad;
        if remote.is_some() { self.remote_anchor = remote; }
    }
}

pub fn combined_buttons(active: bool, mouse: u8, pad: u8, remote: u8) -> u8 {
    if active { (mouse | pad | remote) & 3 } else { 0 }
}

/// Confirmed motion-driven games in the user's compatibility tests. Keep
/// unverified titles on the previously working absolute path, irrespective of
/// which physical controller is active. Call with the resolved catalogue key.
pub fn relative_controls(title_key:&str)->bool {
    matches!(title_key,"tetris"|"battleship")
}

/// The original upstream/gamepad path preserves guest PT_Pos recentering.
/// Peacekeeper profiles use the earlier working reporting path independently
/// of ordinary mouse aiming; the Tetris/Battleship pointer profile is retained.
pub fn relative_motion(title_key:&str,source:PointerSource,peacekeeper_profile:bool)->bool {
    peacekeeper_profile || source==PointerSource::Gamepad || relative_controls(title_key)
}

#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub enum PauseAction { None, PauseGame, ReturnToLibrary }
pub fn pause_action(library:bool,preview:bool,has_disc:bool,launched:bool)->PauseAction {
    if library||preview {PauseAction::None}
    else if !has_disc {PauseAction::ReturnToLibrary}
    else if launched {PauseAction::PauseGame}
    else {PauseAction::None}
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disabled_controls_cannot_contribute_buttons_in_any_combination() {
        for mask in 0..8 {
            let c=ControllerOptions{gamepad:mask&1!=0,mouse:mask&2!=0,peacekeeper:mask&4!=0,mouse_sensitivity:100};
            assert_eq!(c.buttons(true,1,0,0,false),if c.mouse{1}else{0});
            assert_eq!(c.buttons(true,0,2,0,false),if c.gamepad{2}else{0});
            assert_eq!(c.buttons(true,0,0,1,true),if c.peacekeeper{1}else{0});
            assert_eq!(c.buttons(true,0,0,1,false),if c.mouse{1}else{0});
            assert_eq!(c.buttons(false,3,3,3,true),0);
        }
    }
    #[test]
    fn controller_and_legacy_gun_motion_do_not_change_ordinary_mouse_aiming() {
        for title in ["alice in wonderland","3rd degree","asterix caesars challenge","burn cycle","7th guest"] {
            assert!(relative_motion(title,PointerSource::Gamepad,false));
            assert!(!relative_motion(title,PointerSource::Mouse,false));
            assert!(!relative_motion(title,PointerSource::Wiimote,false));
            assert!(relative_motion(title,PointerSource::Wiimote,true));
        }
        for source in [PointerSource::Mouse,PointerSource::Wiimote,PointerSource::Gamepad] {
            assert!(relative_motion("battleship",source,false));
        }
    }
    #[test] fn pause_shortcut_returns_from_bios_but_pauses_a_running_disc(){
        assert_eq!(pause_action(false,false,false,false),PauseAction::ReturnToLibrary);
        assert_eq!(pause_action(false,false,true,true),PauseAction::PauseGame);
        for (library,preview,disc,launched) in [(true,false,false,false),(false,true,true,false),(false,false,true,false)] {
            assert_eq!(pause_action(library,preview,disc,launched),PauseAction::None);
        }
    }
    #[test]
    fn returning_from_sideways_to_pointing_claims_the_pointer_again() {
        let mut a = PointerArbiter::default();
        a.remote_mode(true);
        a.pointers(false,Some((300,200)),false);
        a.remote_mode(false);
        a.gamepad_motion(Some((300,200)));
        a.remote_mode(true);
        a.pointers(false,Some((300,200)),false);
        assert_eq!(a.source(),PointerSource::Wiimote);
    }
    #[test]
    fn mouse_takes_over_from_pointing_remote_and_jitter_cannot_cancel_it() {
        let mut a = PointerArbiter::default();
        a.pointers(false,Some((300,200)),false);
        assert_eq!(a.source(),PointerSource::Wiimote);
        a.pointers(true,Some((300,200)),false);
        for p in [(301,200),(298,203),(303,202)] {
            a.pointers(false,Some(p),false);
            assert_eq!(a.source(),PointerSource::Mouse);
        }
        a.pointers(false,Some((330,220)),false);
        assert_eq!(a.source(),PointerSource::Wiimote);
    }
    #[test]
    fn gamepad_keeps_position_until_another_device_is_used() {
        let mut a = PointerArbiter::default();
        a.gamepad_motion(Some((300,200)));
        a.pointers(false,Some((302,202)),false);
        assert_eq!(a.source(),PointerSource::Gamepad);
        a.pointers(false,Some((302,202)),true);
        assert_eq!(a.source(),PointerSource::Wiimote);
        a.gamepad_motion(Some((302,202)));
        a.pointers(true,Some((302,202)),false);
        assert_eq!(a.source(),PointerSource::Mouse);
    }
    #[test]
    fn releasing_one_controller_does_not_release_another_controllers_button() {
        assert_eq!(combined_buttons(true,1,2,1),3);
        assert_eq!(combined_buttons(true,0,2,1),3);
        assert_eq!(combined_buttons(true,0,2,0),2);
        assert_eq!(combined_buttons(false,1,2,1),0);
    }
}
