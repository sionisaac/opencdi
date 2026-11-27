//! Shared frontend/headless boot handover check. This is a player-shell heuristic,
//! not an operating-system or game-readiness conformance test.
/// The shell's Play sequence needs relative button-only reports after its
/// cursor positioning. Keep this distinct from absolute host gameplay input.
pub fn send_shell_pointer(bus: &mut crate::bus::CdiBus, x: i32, y: i32, buttons: u8) {
    match bus.mono4.as_deref_mut() {
        Some(board) => board.ikat.update_pointer(x, y, buttons),
        None => bus.slave.update_mouse(x, y, buttons),
    }
}

/// Host positions refer to the displayed picture. During gameplay a guest
/// PT_Pos call must not relocate the next host click away from its target.
pub fn send_game_pointer(bus: &mut crate::bus::CdiBus, x: i32, y: i32, buttons: u8) {
    match bus.mono4.as_deref_mut() {
        Some(board) => board.ikat.update_absolute_pointer(x, y, buttons),
        None => bus.slave.update_mouse(x, y, buttons),
    }
}

/// A normal CD-i controller supplies motion, rather than continuously replacing
/// the game's pointer position. Preserve guest PT_Pos recentring on Mono-IV.
/// `edge_push` carries held directions beyond the host coordinate range.
pub fn send_game_controller(
    bus: &mut crate::bus::CdiBus,
    x: i32,
    y: i32,
    buttons: u8,
    edge_push: (i32, i32),
) {
    match bus.mono4.as_deref_mut() {
        Some(board) => {
            board.ikat.update_pointer(x, y, buttons);
            board.ikat.push_pointer(edge_push.0, edge_push.1);
        }
        None => bus.slave.update_mouse(x, y, buttons),
    }
}

/// Retry the known player menu's Play button. Mono-IV becomes eligible after
/// initial disc reads finish and the BIOS enables pointer reports. Once started,
/// finish the press/release sequence even when launching makes the drive busy.
pub fn shell_play_click(
    bus: &crate::bus::CdiBus,
    shell_state: u32,
    count: &mut u32,
) -> Option<(i32, i32, u8)> {
    let (ready, x, y) = match bus.mono4.as_deref() {
        Some(board) => (
            (*count != 0
                || (board.ikat.disc_present
                    && board.ikat.imr & 8 != 0
                    && board.ciap.lba >= 16
                    && !board.ciap.drive_active)),
            630,
            420,
        ),
        None => (shell_state >= 2, 575, 335),
    };
    if !ready {
        return None;
    }
    *count = count.saturating_add(1);
    match *count % 120 {
        10 | 35 => Some((x, y, 0)),
        20 => Some((x, y, 1)),
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

/// A player's status word can remain at its menu value after launching a disc.
/// Accept execution inside the disc-labelled OS-9 program as stronger evidence.
/// Do not accept DVC/player RAM or an arbitrary address in application RAM.
pub fn disc_application_executing(bus: &crate::bus::CdiBus, pc: u32, user_mode: bool) -> bool {
    if !user_mode {
        return false;
    }
    let Some(disc) = bus.cdrom.as_ref() else {
        return false;
    };
    let application = disc
        .application_module
        .as_deref()
        .unwrap_or(&disc.application_id);
    match pc {
        0x10000..=0x1fffff => {
            application_module_at(&bus.mcd212.plane_a[..], pc as usize, application)
        }
        0x200000..=0x2fffff => application_module_at(
            &bus.mcd212.plane_b[..],
            (pc - 0x200000) as usize,
            application,
        ),
        _ => false,
    }
}

fn application_module_at(ram: &[u8], pc: usize, application: &str) -> bool {
    let Some(wanted) = application.rsplit('/').next().filter(|s| !s.is_empty()) else {
        return false;
    };
    let stem = |s: &str| s.to_ascii_lowercase().trim_end_matches(".mod").to_string();
    let wanted = stem(wanted);
    if pc >= ram.len() {
        return false;
    }
    for start in (0..pc / 2 + 1).rev().map(|word| word * 2) {
        let Some(header) = ram.get(start..start + 48) else {
            continue;
        };
        if header[..2] != [0x4a, 0xfc] || header[18] != 1 {
            continue;
        }
        if header
            .chunks_exact(2)
            .fold(0u16, |p, w| p ^ u16::from_be_bytes([w[0], w[1]]))
            != 0xffff
        {
            continue;
        }
        let long = |at| u32::from_be_bytes(header[at..at + 4].try_into().unwrap()) as usize;
        let size = long(4);
        let name = long(12);
        if size < 51
            || name < 48
            || name >= size - 3
            || pc < start + 48
            || pc >= start.saturating_add(size)
        {
            continue;
        }
        let Some(bytes) = ram.get(start + name..start.saturating_add(size) - 3) else {
            continue;
        };
        let mut text = String::new();
        let mut terminated = false;
        for &b in bytes.iter().take(128) {
            if b == 0 {
                terminated = true;
                break;
            }
            let ch = b & 0x7f;
            if !(0x20..0x7f).contains(&ch) {
                break;
            }
            text.push(ch as char);
            if b & 0x80 != 0 {
                terminated = true;
                break;
            }
        }
        if terminated && stem(&text) == wanted {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_shell_flag_requires_execution_inside_the_disc_named_program() {
        let mut ram = vec![0u8; 512];
        let start = 128;
        ram[start..start + 2].copy_from_slice(&[0x4a, 0xfc]);
        ram[start + 4..start + 8].copy_from_slice(&128u32.to_be_bytes());
        ram[start + 12..start + 16].copy_from_slice(&48u32.to_be_bytes());
        ram[start + 18] = 1;
        ram[start + 48..start + 61].copy_from_slice(b"cdi_main.mod\0");
        let parity = ram[start..start + 46]
            .chunks_exact(2)
            .fold(0xffffu16, |p, w| p ^ u16::from_be_bytes([w[0], w[1]]));
        ram[start + 46..start + 48].copy_from_slice(&parity.to_be_bytes());
        assert!(application_module_at(&ram, start + 80, "CMDS/cdi_main"));
        assert!(!application_module_at(&ram, start + 80, "CMDS/other"));
        assert!(!application_module_at(&ram, start + 10, "CMDS/cdi_main"));
        assert!(!application_module_at(&ram, start + 150, "CMDS/cdi_main"));
        assert!(!application_module_at(&ram, start + 80, ""));
        ram[start + 46] ^= 1;
        assert!(!application_module_at(&ram, start + 80, "CMDS/cdi_main"));
    }
    #[test]
    fn shell_button_report_stays_relative_while_game_click_is_absolute() {
        let mut bus = crate::bus::CdiBus::new();
        bus.set_board(crate::bus::BoardKind::Mono4);
        bus.mono4.as_deref_mut().unwrap().ikat.write(0x1B, 8, 0);
        for _ in 0..4 {
            bus.mono4.as_deref_mut().unwrap().ikat.read(0x0B, 0);
        }
        send_shell_pointer(&mut bus, 630, 420, 0);
        for _ in 0..4 {
            bus.mono4.as_deref_mut().unwrap().ikat.read(0x0B, 0);
        }
        send_shell_pointer(&mut bus, 630, 420, 1);
        let shell: Vec<_> = (0..4)
            .map(|_| bus.mono4.as_deref_mut().unwrap().ikat.read(0x0B, 0))
            .collect();
        assert_eq!(shell[0] & 0x30, 0x20);
        assert_eq!(
            shell[3] & 0x80,
            0,
            "shell press must not switch to an absolute report"
        );
        send_game_pointer(&mut bus, 630, 420, 0);
        let game: Vec<_> = (0..4)
            .map(|_| bus.mono4.as_deref_mut().unwrap().ikat.read(0x0B, 0))
            .collect();
        assert_eq!(
            game[3] & 0x80,
            0x80,
            "host gameplay coordinates must remain absolute"
        );
    }

    #[test]
    fn mono1_shell_and_game_input_preserve_identical_slave_state() {
        use crate::savestate::State;
        let mut shell = crate::bus::CdiBus::new();
        let mut game = crate::bus::CdiBus::new();
        for (x, y, buttons) in [(350, 140, 0), (350, 140, 1), (350, 140, 0), (352, 141, 2)] {
            send_shell_pointer(&mut shell, x, y, buttons);
            send_game_pointer(&mut game, x, y, buttons);
            let (mut a, mut b) = (Vec::new(), Vec::new());
            shell.slave.save(&mut a);
            game.slave.save(&mut b);
            assert_eq!(
                a, b,
                "Mono-I input must not change when gameplay routing is selected"
            );
        }
    }

    #[test]
    fn mono4_waits_for_disc_and_finishes_releasing_during_loading() {
        let mut bus = crate::bus::CdiBus::new();
        bus.set_board(crate::bus::BoardKind::Mono4);
        let mut count = 0;
        assert!(shell_play_click(&bus, 0, &mut count).is_none());
        assert_eq!(count, 0);
        let b = bus.mono4.as_deref_mut().unwrap();
        b.ikat.disc_present = true;
        b.ikat.imr = 8;
        b.ciap.lba = 16;
        b.ciap.drive_active = true;
        assert!(shell_play_click(&bus, 0, &mut count).is_none());
        assert_eq!(count, 0);
        bus.mono4.as_deref_mut().unwrap().ciap.drive_active = false;
        let mut events = Vec::new();
        for frame in 1..=35 {
            if frame == 21 {
                bus.mono4.as_deref_mut().unwrap().ciap.drive_active = true;
            }
            if let Some(event) = shell_play_click(&bus, 0, &mut count) {
                events.push((frame, event));
            }
        }
        assert_eq!(
            events,
            vec![
                (10, (630, 420, 0)),
                (20, (630, 420, 1)),
                (35, (630, 420, 0))
            ]
        );
    }

    #[test]
    fn regular_controller_moves_and_clicks_do_not_force_lightgun_positions() {
        let mut bus = crate::bus::CdiBus::new();
        bus.set_board(crate::bus::BoardKind::Mono4);
        bus.mono4.as_deref_mut().unwrap().ikat.write(0x1B, 8, 0);
        let read = |bus: &mut crate::bus::CdiBus| -> Vec<u8> {
            (0..4)
                .map(|_| bus.mono4.as_deref_mut().unwrap().ikat.read(0x0B, 0))
                .collect()
        };
        read(&mut bus);
        send_game_controller(&mut bus, 387, 280, 0, (0, 0));
        assert_eq!(read(&mut bus), [0x40, 3, 0, 0]);
        send_game_controller(&mut bus, 387, 280, 1, (0, 0));
        assert_eq!(read(&mut bus), [0x60, 0, 0, 0]);
        send_game_controller(&mut bus, 387, 280, 0, (0, 0));
        assert_eq!(read(&mut bus), [0x40, 0, 0, 0]);
        send_game_controller(&mut bus, 767, 280, 0, (0, 0));
        read(&mut bus);
        send_game_controller(&mut bus, 767, 280, 0, (3, 0));
        assert_eq!(read(&mut bus), [0x40, 3, 0, 0]);
        send_game_controller(&mut bus, 767, 280, 0, (0, 0));
        assert_eq!(bus.mono4.as_ref().unwrap().ikat.pending() & 8, 0);
    }
}
