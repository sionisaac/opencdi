use opencdi::mcd212;
use std::path::{Path, PathBuf};
use m68k::{CpuCore, CpuType, AddressBus};

use opencdi::bus::CdiBus;
use opencdi::mcd212::{TOTAL_SCANLINES, CYCLES_PER_SCANLINE};

fn find_file<P: AsRef<Path>>(rel: P) -> Option<PathBuf> {
    let candidates = [
        PathBuf::from(".").join(&rel),
        PathBuf::from("..").join(&rel),
        PathBuf::from("../file utili").join(&rel),
    ];
    for p in candidates {
        if p.exists() {
            return Some(p);
        }
    }
    None
}

fn save_bmp(bus: &CdiBus, filename: &str) -> std::io::Result<()> {
    let width = mcd212::SCREEN_WIDTH as i32;
    let height = mcd212::SCREEN_HEIGHT as i32;
    let file_size = 54 + (width * height * 4) as u32;

    let mut bmp = Vec::with_capacity(file_size as usize);
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&file_size.to_le_bytes());
    bmp.extend_from_slice(&[0, 0, 0, 0]);
    bmp.extend_from_slice(&54u32.to_le_bytes());

    bmp.extend_from_slice(&40u32.to_le_bytes());
    bmp.extend_from_slice(&width.to_le_bytes());
    bmp.extend_from_slice(&(-height).to_le_bytes()); // top-down
    bmp.extend_from_slice(&1u16.to_le_bytes()); // planes
    bmp.extend_from_slice(&32u16.to_le_bytes()); // 32 bpp
    bmp.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    bmp.extend_from_slice(&((width * height * 4) as u32).to_le_bytes());
    bmp.extend_from_slice(&2835u32.to_le_bytes());
    bmp.extend_from_slice(&2835u32.to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());

    for &pixel in &bus.mcd212.framebuffer[..] {
        let b = (pixel & 0xFF) as u8;
        let g = ((pixel >> 8) & 0xFF) as u8;
        let r = ((pixel >> 16) & 0xFF) as u8;
        bmp.push(b);
        bmp.push(g);
        bmp.push(r);
        bmp.push(0xFF);
    }
    std::fs::write(filename, &bmp)
}

fn test_game(game_rel_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=======================================================");
    println!("Testing natural disc detection for: {}", game_rel_path);
    println!("=======================================================");

    let mut bus = CdiBus::new();
    let bios_path = find_file("bios/cdi220b.rom").expect("BIOS not found");
    bus.load_rom(&bios_path)?;

    let game_path = find_file(game_rel_path).expect("Game not found");
    bus.load_cdrom(&game_path)?;

    let mut cpu = CpuCore::new();
    cpu.set_cpu_type(CpuType::SCC68070);
    cpu.reset(&mut bus);

    let mut disc_ready_frame = None;
    let mut toc_read_frame = None;

    for frame in 1..=1000 {
        if bus.slave.disc_state == 0x44 && disc_ready_frame.is_none() {
            println!("  [Frame {}] Slave transitioned to 0x44 (Disc Ready) automatically via tick!", frame);
            disc_ready_frame = Some(frame);
        }

        if (bus.cdic.command == 0x24 || bus.cdic.command == 0x29) && toc_read_frame.is_none() {
            println!("  [Frame {}] BIOS CDIC initiated disc read: cmd=0x{:02X}, LBA={}", frame, bus.cdic.command, bus.cdic.curr_lba);
            toc_read_frame = Some(frame);
        }

        let play_a6 = 0x0000C000u32;
        let state_9172 = bus.read_long(play_a6.wrapping_add(0xFFFF9172));

        if frame % 200 == 0 {
            println!("  [Frame {}] PC=0x{:08X}, state_9172={}, CDIC cmd=0x{:02X}, lba={}",
                frame, cpu.pc, state_9172, bus.cdic.command, bus.cdic.curr_lba);
            let path = format!("C:/Users/Gian/.gemini/antigravity/brain/47421117-6e2a-4536-8928-519a0c9b76ed/framebuffer_dl_{}.bmp", frame);
            let _ = save_bmp(&bus, &path);
        }

        for _line in 0..TOTAL_SCANLINES {
            if bus.scc_timer.timer_irq {
                cpu.set_irq(6);
            } else if bus.cdic.irq_pending {
                cpu.set_irq(4);
            } else if bus.slave.irq_pending {
                cpu.set_irq(2);
            } else if bus.mcd212.vblank_irq {
                let mcd_lvl = (bus.scc_timer.picr1 >> 4) & 7;
                let irq_lvl = if mcd_lvl != 0 { mcd_lvl } else { 1 };
                cpu.set_irq(irq_lvl);
            } else {
                cpu.set_irq(0);
            }

            let mut line_cycles = 0;
            while line_cycles < CYCLES_PER_SCANLINE as i32 {
                let budget = (CYCLES_PER_SCANLINE as i32 - line_cycles).max(1);
                bus.current_pc = cpu.pc;
                let batch = cpu.run_for_cycles(&mut bus, budget);
                line_cycles += batch.cycles;

                match batch.exit {
                    m68k::CycleBatchExit::TrapInstruction { trap_num } => {
                        let trap_cyc = cpu.take_trap_exception(&mut bus, trap_num);
                        line_cycles += trap_cyc;
                    }
                    m68k::CycleBatchExit::AlineTrap { .. } => {
                        let trap_cyc = cpu.take_aline_exception(&mut bus);
                        line_cycles += trap_cyc;
                    }
                    m68k::CycleBatchExit::FlineTrap { .. } => {
                        let trap_cyc = cpu.take_fline_exception(&mut bus);
                        line_cycles += trap_cyc;
                    }
                    m68k::CycleBatchExit::IllegalInstruction { .. } => {
                        let trap_cyc = cpu.take_illegal_exception(&mut bus);
                        line_cycles += trap_cyc;
                    }
                    m68k::CycleBatchExit::Breakpoint { .. } => {
                        let trap_cyc = cpu.take_bkpt_exception(&mut bus);
                        line_cycles += trap_cyc;
                    }
                    m68k::CycleBatchExit::Stopped => {
                        line_cycles = CYCLES_PER_SCANLINE as i32;
                        break;
                    }
                    _ => {}
                }
            }

            let spent = (line_cycles as u32).max(CYCLES_PER_SCANLINE);
            bus.mcd212.tick_cycles(spent);
            bus.tick_cdic(spent);
            bus.tick_timer(spent);
        }
    }

    save_bmp(&bus, "C:/Users/Gian/.gemini/antigravity/brain/47421117-6e2a-4536-8928-519a0c9b76ed/framebuffer_mario_2000.bmp")?;
    println!("Saved framebuffer at frame 2000 to framebuffer_mario_2000.bmp");

    let disc_ok = disc_ready_frame.is_some();
    let toc_ok = toc_read_frame.is_some();
    let play_a6 = 0x0000C000u32;
    let final_state_9172 = bus.read_long(play_a6.wrapping_add(0xFFFF9172));
    let final_pc = cpu.pc;
    println!("  >> Game: {}", game_rel_path);
    println!("  >> Result: DiscReady={:?}, TocRead={:?}, state_9172={}, final_pc=0x{:08X}",
        disc_ready_frame, toc_read_frame, final_state_9172, final_pc);
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let games = [
        "giochi/Dragon's Lair (USA).chd",
        "giochi/Tangram - The Ultimate Chinese Game (Europe).chd",
        "giochi/7th Guest, The (USA).chd",
        "giochi/Dark Fables of Aesop The (Europe).chd",
        "giochi/Lost Eden (Europe).chd",
        "giochi/Rise of the Robots (Europe).chd",
        "giochi/Striker Pro (USA).chd",
    ];

    for g in &games {
        if let Err(e) = test_game(g) {
            println!("Error testing {}: {:?}", g, e);
        }
    }
    Ok(())
}
