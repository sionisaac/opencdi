//! Chooses who moves the single CD-i pointer without excluding other buttons.
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
