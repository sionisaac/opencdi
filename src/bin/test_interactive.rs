use opencdi::mcd212;
use std::path::{Path, PathBuf};
use m68k::{CpuCore, CpuType, AddressBus};
use opencdi::bus::CdiBus;

fn find_file(relative_subpath: &str) -> Option<PathBuf> {
    let candidates = [
        Path::new(relative_subpath).to_path_buf(),
        Path::new("..").join(relative_subpath),
        Path::new("file utili").join(relative_subpath),
        Path::new("..").join("file utili").join(relative_subpath),
    ];
    for c in &candidates {
        if c.exists() {
            return Some(c.clone());
        }
    }
    None
}

fn save_bmp(bus: &CdiBus, path: &str) -> std::io::Result<()> {
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
    bmp.extend_from_slice(&1u16.to_le_bytes());
    bmp.extend_from_slice(&32u16.to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());
    bmp.extend_from_slice(&(width * height * 4).to_le_bytes());
    bmp.extend_from_slice(&2835u32.to_le_bytes());
    bmp.extend_from_slice(&2835u32.to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());

    for pixel in bus.mcd212.framebuffer.iter() {
        bmp.extend_from_slice(&pixel.to_le_bytes());
    }

    std::fs::write(path, bmp)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let target = args.get(1).map(|s| s.as_str()).unwrap_or("mario");
    let max_frames: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2500);

    let mut bus = CdiBus::new();
    let bios_path = find_file("bios/cdi220b.rom").expect("cdi220b.rom missing");
    bus.load_rom(&bios_path)?;

    if let Some(nvram_path) = find_file("bios/nvram_mk48t08.bin") {
        let _ = bus.nvram.load_from_file(&nvram_path);
    }
    if let Some(dvc_path) = find_file("dvc/vmpega.rom") {
        let _ = bus.load_dvc_rom(&dvc_path);
    }

    let disc_path = match target {
        "mario" => find_file("giochi/Hotel Mario (USA).chd"),
        "lair" => find_file("giochi/Dragon's Lair (USA).chd"),
        "7th" => find_file("giochi/7th Guest, The (USA).chd"),
        _ => None,
    };

    if let Some(p) = &disc_path {
        bus.load_cdrom(p)?;
        println!("Loaded CD-ROM: {:?}", p);
    }

    bus.set_video_mode(crate::mcd212::VideoMode::Pal);

    let mut cpu = CpuCore::new();
    cpu.set_cpu_type(CpuType::SCC68070);
    cpu.reset(&mut bus);

    let mut play_a6: u32 = 0x0000C000;
    let mut disc_launched = false;
    let mut shell_ready_count = 0;

    let total_lines = bus.mcd212.video_mode.total_scanlines();
    let cyc_per_line = bus.mcd212.video_mode.cycles_per_scanline();

    for frame in 1..=max_frames {
        // Track play_a6 dynamically
        if cpu.pc >= 0x00461614 && cpu.pc <= 0x00479C0E {
            let cur_a6 = cpu.a(6);
            if cur_a6 != play_a6 && cur_a6 > 0x1000 {
                play_a6 = cur_a6;
                println!("[Frame {:4}] Dynamic play_a6 updated to 0x{:08X} (PC=0x{:06X})", frame, play_a6, cpu.pc);
            }
        }

        let shell_state = bus.read_long(play_a6.wrapping_add(0xFFFF9172));

        if !disc_launched && bus.cdrom.is_some() && frame >= 820 {
            if shell_state >= 2 {
                shell_ready_count += 1;
                if shell_ready_count == 30 {
                    println!("[Frame {:4}] SHELL READY! State={} at A6=0x{:08X}. Clicking PLAY CD-I at (575, 335)...", frame, shell_state, play_a6);
                    bus.slave.update_mouse(575, 335, 1);
                } else if shell_ready_count == 50 {
                    println!("[Frame {:4}] Releasing click at (575, 335)...", frame);
                    bus.slave.update_mouse(575, 335, 0);
                    disc_launched = true;
                    bus.slave.update_mouse(384, 280, 0);
                }
            } else if cpu.pc >= 0x00010000 && cpu.pc < 0x00300000 {
                println!("[Frame {:4}] Disc app running in RAM directly (PC=0x{:06X})", frame, cpu.pc);
                disc_launched = true;
            }
        }

        if frame % 100 == 0 || (frame >= 700 && frame <= 1650 && frame % 50 == 0) {
            println!(
                "[Frame {:4}] PC=0x{:06X} SP=0x{:06X} A6=0x{:08X} | shell_state={} disc_launched={} LBA={} CDIC_CMD=0x{:02X}",
                frame, cpu.pc, cpu.a(7), cpu.a(6), shell_state, disc_launched, bus.cdic.curr_lba, bus.cdic.command
            );
        }

        let mut cycle_debt: i32 = 0;
        for line in 0..total_lines {
            bus.tick_scanline(line);

            if bus.mcd212.vblank_irq {
                bus.scc_timer.lir |= 0x80;
            }
            let int1_lvl = if (bus.scc_timer.lir & 0x80) != 0 {
                let lvl = (bus.scc_timer.lir >> 4) & 7;
                if lvl != 0 { lvl } else { 5 }
            } else { 0 };

            let timer_lvl = if bus.scc_timer.timer_irq {
                let lvl = bus.scc_timer.picr1 & 7;
                if lvl != 0 { lvl } else { 6 }
            } else { 0 };

            let cdic_lvl = if bus.cdic.irq_pending || bus.dvc.irq_pending() { 4 } else { 0 };
            let slave_lvl = if bus.slave.irq_pending { 2 } else { 0 };
            let uart_lvl = if bus.scc_uart_tx_irq {
                let lvl = bus.scc_timer.picr2 & 7;
                if lvl != 0 { lvl } else { 5 }
            } else { 0 };

            let highest_ipl = int1_lvl.max(timer_lvl).max(cdic_lvl).max(slave_lvl).max(uart_lvl);
            cpu.set_irq(highest_ipl);

            let mut line_cycles = -cycle_debt;
            while line_cycles < cyc_per_line as i32 {
                let remaining = (cyc_per_line as i32 - line_cycles).max(1);
                let budget = remaining.min(32);

                bus.current_pc = cpu.pc;
                bus.current_line_cycle = line_cycles.max(0) as u32;

                let batch = cpu.run_for_cycles(&mut bus, budget);
                line_cycles += batch.cycles + bus.consume_wait_cycles();

                match batch.exit {
                    m68k::CycleBatchExit::TrapInstruction { trap_num } => {
                        if trap_num == 0 && frame >= 1200 && frame <= 1650 {
                            let next_word = bus.read_word(cpu.pc);
                            println!("[OS9 TRAP0 F{:4} PC=0x{:06X}] code=0x{:04X}, D0=0x{:08X}, D1=0x{:08X}, A0=0x{:08X}, A1=0x{:08X}",
                                frame, cpu.pc, next_word, cpu.d(0), cpu.d(1), cpu.a(0), cpu.a(1));
                        }
                        let trap_cyc = cpu.take_trap_exception(&mut bus, trap_num);
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
                        line_cycles = cyc_per_line as i32;
                        break;
                    }
                    _ => {}
                }
            }

            cycle_debt = (line_cycles - cyc_per_line as i32).clamp(0, 100);
            bus.render_scanline(line);
            bus.tick_slave(cyc_per_line);
            bus.tick_cdic(cyc_per_line);
            bus.tick_dvc(cyc_per_line);
            bus.tick_timer(cyc_per_line);
        }

        if frame == 820 || frame == 1200 || frame == 1800 || frame == 2500 || frame == max_frames {
            let out_bmp = format!(
                "C:/Users/Gian/.gemini/antigravity/brain/47421117-6e2a-4536-8928-519a0c9b76ed/inter_{}_{}.bmp",
                target, frame
            );
            let _ = save_bmp(&bus, &out_bmp);
            println!("Saved screenshot: {}", out_bmp);
        }
    }

    Ok(())
}
