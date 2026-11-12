//! Shared frontend/headless boot handover check. This is a player-shell heuristic,
//! not an operating-system or game-readiness conformance test.
/// The shell's Play sequence needs relative button-only reports after its
/// cursor positioning. Keep this distinct from absolute host gameplay input.
pub fn send_shell_pointer(bus: &mut crate::bus::CdiBus, x: i32, y: i32, buttons: u8) {
    match bus.mono4.as_deref_mut() {
        Some(board) => board.ikat.update_pointer(x,y,buttons),
        None => bus.slave.update_mouse(x,y,buttons),
    }
}

/// Host positions refer to the displayed picture. During gameplay a guest
/// PT_Pos call must not relocate the next host click away from its target.
pub fn send_game_pointer(bus: &mut crate::bus::CdiBus, x: i32, y: i32, buttons: u8) {
    match bus.mono4.as_deref_mut() {
        Some(board) => board.ikat.update_absolute_pointer(x,y,buttons),
        None => bus.slave.update_mouse(x,y,buttons),
    }
}

/// Retry the known player menu's Play button. Mono-IV becomes eligible after
/// initial disc reads finish and the BIOS enables pointer reports. Once started,
/// finish the press/release sequence even when launching makes the drive busy.
pub fn shell_play_click(bus: &crate::bus::CdiBus, shell_state: u32, count: &mut u32) -> Option<(i32, i32, u8)> {
    let (ready, x, y) = match bus.mono4.as_deref() {
        Some(board) => ((*count != 0 || (board.ikat.disc_present && board.ikat.imr & 8 != 0
            && board.ciap.lba >= 16 && !board.ciap.drive_active)), 630, 420),
        None => (shell_state >= 2, 575, 335),
    };
    if !ready { return None; }
    *count = count.saturating_add(1);
    match *count % 120 {
        10 | 35 => Some((x,y,0)),
        20 => Some((x,y,1)),
        _ => None,
    }
}

pub fn disc_program_started(
    pc: u32,
    frames: u64,
    shell_state: u32,
    shell_click_frames: u32,
) -> bool {
    // DVC RAM also contains shell routines: finish the click and leave its menu.
    frames > 250
        && shell_click_frames >= 35
        && !matches!(shell_state, 2 | 3)
        && ((0x00010000..0x00300000).contains(&pc) || (0x00D00000..0x00E00000).contains(&pc))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shell_button_report_stays_relative_while_game_click_is_absolute() {
        let mut bus = crate::bus::CdiBus::new();
        bus.set_board(crate::bus::BoardKind::Mono4);
        bus.mono4.as_deref_mut().unwrap().ikat.write(0x1B,8,0);
        for _ in 0..4 { bus.mono4.as_deref_mut().unwrap().ikat.read(0x0B,0); }
        send_shell_pointer(&mut bus,630,420,0);
        for _ in 0..4 { bus.mono4.as_deref_mut().unwrap().ikat.read(0x0B,0); }
        send_shell_pointer(&mut bus,630,420,1);
        let shell: Vec<_> = (0..4).map(|_| bus.mono4.as_deref_mut().unwrap().ikat.read(0x0B,0)).collect();
        assert_eq!(shell[0] & 0x30,0x20);
        assert_eq!(shell[3] & 0x80,0,"shell press must not switch to an absolute report");
        send_game_pointer(&mut bus,630,420,0);
        let game: Vec<_> = (0..4).map(|_| bus.mono4.as_deref_mut().unwrap().ikat.read(0x0B,0)).collect();
        assert_eq!(game[3] & 0x80,0x80,"host gameplay coordinates must remain absolute");
    }

    #[test]
    fn mono1_shell_and_game_input_preserve_identical_slave_state() {
        use crate::savestate::State;
        let mut shell = crate::bus::CdiBus::new();
        let mut game = crate::bus::CdiBus::new();
        for (x,y,buttons) in [(350,140,0),(350,140,1),(350,140,0),(352,141,2)] {
            send_shell_pointer(&mut shell,x,y,buttons);
            send_game_pointer(&mut game,x,y,buttons);
            let (mut a,mut b) = (Vec::new(),Vec::new());
            shell.slave.save(&mut a);
            game.slave.save(&mut b);
            assert_eq!(a,b,"Mono-I input must not change when gameplay routing is selected");
        }
    }

    #[test]
    fn mono4_waits_for_disc_and_finishes_releasing_during_loading() {
        let mut bus = crate::bus::CdiBus::new();
        bus.set_board(crate::bus::BoardKind::Mono4);
        let mut count = 0;
        assert!(shell_play_click(&bus,0,&mut count).is_none());
        assert_eq!(count,0);
        let b = bus.mono4.as_deref_mut().unwrap();
        b.ikat.disc_present = true; b.ikat.imr = 8; b.ciap.lba = 16; b.ciap.drive_active = true;
        assert!(shell_play_click(&bus,0,&mut count).is_none());
        assert_eq!(count,0);
        bus.mono4.as_deref_mut().unwrap().ciap.drive_active = false;
        let mut events = Vec::new();
        for frame in 1..=35 {
            if frame == 21 { bus.mono4.as_deref_mut().unwrap().ciap.drive_active = true; }
            if let Some(event) = shell_play_click(&bus,0,&mut count) { events.push((frame,event)); }
        }
        assert_eq!(events,vec![(10,(630,420,0)),(20,(630,420,1)),(35,(630,420,0))]);
    }
}
