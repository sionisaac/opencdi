use opencdi::mcd212;
use std::path::{Path, PathBuf};
use std::time::Instant;
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

    std::fs::write(path, &bmp)?;
    Ok(())
}

fn test_game(game_name: &str, disc_subpath: &str, is_dvc: bool, max_frames: u64) -> bool {
    println!("\n========================================================");
    println!("VERIFICA AVVIO GIOCO: {} (DVC: {})", game_name, is_dvc);
    println!("========================================================");

    let mut bus = CdiBus::new();
    let bios_path = find_file("bios/cdi220b.rom").expect("cdi220b.rom non trovata");
    bus.load_rom(&bios_path).unwrap();

    if let Some(nvram_path) = find_file("bios/nvram_mk48t08.bin") {
        let _ = bus.nvram.load_from_file(&nvram_path);
    }

    if is_dvc {
        let dvc_path = find_file("dvc/vmpega.rom").expect("dvc/vmpega.rom non trovata");
        bus.load_dvc_rom(&dvc_path).unwrap();
    }

    let disc_path = find_file(disc_subpath).expect("File disco non trovato");
    bus.load_cdrom(&disc_path).unwrap();

    bus.set_video_mode(crate::mcd212::VideoMode::Ntsc);

    let mut cpu = CpuCore::new();
    cpu.set_cpu_type(CpuType::SCC68070);
    cpu.reset(&mut bus);

    let mut play_a6 = if bus.dvc.is_fitted() { 0x00D08090 } else { 0x0000C000 };
    let mut disc_launched = false;
    let mut shell_ready_count = 0;
    let mut pad_x = 578;
    let mut pad_y = 336;
    bus.emulate_wait_states = true;
    bus.watch_addr1 = 0x0027EA14;
    bus.watch_addr2 = 0x0027EA16;
    bus.watch_enabled = true;

    println!("--- EXCEPTION VECTORS AT STARTUP ---");
    for vec in 24..=64 {
        let addr = (vec * 4) as u32;
        let handler = bus.read_long(addr);
        if handler != 0 {
            println!("  Vector 0x{:02X} ({:2}) [0x{:04X}]: 0x{:08X}", vec, vec, addr, handler);
        }
    }
    let start_time = Instant::now();
    let mut time_to_shell = None;
    let mut time_to_game = None;
    let mut game_reached = false;

    let total_lines = bus.mcd212.video_mode.total_scanlines();
    let cyc_per_line = bus.mcd212.video_mode.cycles_per_scanline();

    for frame in 1..=max_frames {
        let mut cycle_debt: i32 = 0;
        for line in 0..total_lines {
            bus.tick_scanline(line);

            if bus.mcd212.vblank_irq {
                bus.scc_timer.lir |= 0x80;
            }

            let int1_lvl = if (bus.scc_timer.lir & 0x80) != 0 {
                let prio = (bus.scc_timer.lir >> 4) & 7;
                if prio != 0 { prio } else { 5 }
            } else { 0 };

            let timer_lvl = if bus.scc_timer.timer_irq {
                let prio = bus.scc_timer.picr1 & 7;
                if prio != 0 { prio } else { 6 }
            } else { 0 };

            let cdic_lvl = if bus.cdic.irq_pending || bus.dvc.irq_pending() { 4 } else { 0 };
            let slave_lvl = if bus.slave.irq_pending { 2 } else { 0 };
            let highest_ipl = int1_lvl.max(timer_lvl).max(cdic_lvl).max(slave_lvl);
            cpu.set_irq(highest_ipl);

            let mut line_cycles = -cycle_debt;
            while line_cycles < cyc_per_line as i32 {
                let remaining = (cyc_per_line as i32 - line_cycles).max(1);
                let budget = if (875..=880).contains(&frame) { 1 } else { remaining.min(32) };

                bus.current_pc = cpu.pc;
                bus.current_line_cycle = line_cycles.max(0) as u32;

                if cpu.pc == 0x42D4A4 {
                    let a2 = cpu.a(2);
                    let a5 = cpu.a(5);
                    let val = bus.read_long(a2.wrapping_add(0x304));
                    println!(">>> [F{} L{}] cdapdriv 0x42D4A4: a2=0x{:08X}, a5=0x{:08X}, val at a2+0x304 = 0x{:08X}",
                        frame, line, a2, a5, val);
                }
                if frame == 878 && (42..=45).contains(&line) {
                    println!("  [F878 L{}] PC=0x{:06X} D0={:08X} D1={:08X} D2={:08X} D3={:08X} A0={:08X} SP={:08X} SR={:04X}",
                        line, cpu.pc, cpu.d(0), cpu.d(1), cpu.d(2), cpu.d(3), cpu.a(0), cpu.a(7), cpu.get_sr());
                }

                let batch = cpu.run_for_cycles(&mut bus, budget);
                line_cycles += batch.cycles + bus.consume_wait_cycles();

                if cpu.pc == 0x004619AC {
                    play_a6 = cpu.a(6);
                }
                if frame == 750 && (cpu.pc == 0x41C738 || cpu.pc == 0x41C742) && line % 30 == 0 {
                    let csrr0 = bus.mcd212.csrr[0];
                    let val = bus.read_byte(0x4FFFF1);
                    println!("[F{} L{}] PC=0x{:06X}, csrr[0]=0x{:02X}, val_4ffff1=0x{:02X}",
                        frame, line, cpu.pc, csrr0, val);
                }

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
                        line_cycles = cyc_per_line as i32;
                        break;
                    }
                    _ => {}
                }
            }
            cycle_debt = (line_cycles - cyc_per_line as i32).clamp(0, 100);

            bus.tick_slave(cyc_per_line);
            bus.tick_cdic(cyc_per_line);
            bus.tick_dvc(cyc_per_line);
            bus.tick_timer(cyc_per_line);
        }

        // Auto-launch verification
        if !disc_launched {
            let shell_state = bus.read_long(play_a6.wrapping_add(0xFFFF9172));
            let c390 = bus.read_long(play_a6.wrapping_add(0xFFFFC390));
            if shell_state >= 2 {
                if time_to_shell.is_none() {
                    let elapsed = start_time.elapsed();
                    time_to_shell = Some(elapsed);
                    println!(">>> [Frame {}] BIOS SHELL RAGGIUNTA in {:.2} secondi! shell_state={}, c390={}", frame, elapsed.as_secs_f64(), shell_state, c390);
                }
                if c390 == 1 || (frame >= 865 && shell_state >= 2) {
                    shell_ready_count += 1;
                    if shell_ready_count == 1 {
                        bus.slave.update_mouse(578, 336, 0);
                    } else if shell_ready_count == 15 {
                        bus.slave.update_mouse(578, 336, 1);
                    } else if shell_ready_count == 25 {
                        bus.slave.update_mouse(578, 336, 0);
                    } else if shell_ready_count >= 30 {
                        println!(">>> [Frame {}] Fallback Action 29 (PLAY CD-I) scritta a C344/C364/9172/91A6, play_a6=0x{:08X}", frame, play_a6);
                        bus.write_long(play_a6.wrapping_add(0xFFFFC364), 0x00000500);
                        bus.write_long(play_a6.wrapping_add(0xFFFF9172), 2);
                        bus.write_long(play_a6.wrapping_add(0xFFFF91A6), 1);
                        bus.write_long(play_a6.wrapping_add(0xFFFFC344), 29);
                        shell_ready_count = 0;
                    }
                }
            }
            if cpu.pc >= 0x00010000 && cpu.pc < 0x00300000 && frame > 400 {
                println!(">>> [Frame {}] Applicazione disco già in RAM (PC=0x{:06X})!", frame, cpu.pc);
                disc_launched = true;
            }
        } else {
            if cpu.pc >= 0x00010000 && cpu.pc < 0x00300000 {
                if !game_reached {
                    game_reached = true;
                    bus.emulate_wait_states = true; // Switch to hardware accurate timing once in game!
                    let elapsed = start_time.elapsed();
                    time_to_game = Some(elapsed);
                    println!(">>> [Frame {}] GIOCO AVVIATO IN RAM! PC=0x{:06X} in {:.2} secondi complessivi!",
                        frame, cpu.pc, elapsed.as_secs_f64());
                }
            }
        }

        if frame % 100 == 0 {
            if frame == 100 {
                println!("--- EXCEPTION VECTORS AT FRAME 100 ---");
                for vec in 24..=64 {
                    let addr = (vec * 4) as u32;
                    let handler = bus.read_long(addr);
                    if handler != 0 {
                        println!("  Vector 0x{:02X} ({:2}) [0x{:04X}]: 0x{:08X}", vec, vec, addr, handler);
                    }
                }
            }
            let v0 = (bus.mcd212.vsr[0] as usize) & 0x7FFFF;
            let v1 = (bus.mcd212.vsr[1] as usize) & 0x7FFFF;
            println!("[Frame {:4}] PC=0x{:06X}, vsr=[0x{:04X}, 0x{:04X}], plane_a[vsr0..+8]={:02X?}, plane_b[vsr1..+8]={:02X?}",
                frame, cpu.pc, bus.mcd212.vsr[0], bus.mcd212.vsr[1],
                &bus.mcd212.plane_a[v0..v0+8], &bus.mcd212.plane_b[v1..v1+8]);
        }
    }

    let bmp_name = format!("verify_{}.bmp", game_name.replace(" ", "_").to_lowercase());
    let _ = save_bmp(&bus, &bmp_name);
    println!("Screenshot salvato: {}", bmp_name);

    if game_reached {
        println!("RISULTATO: [SUCCESSO] {} si e' avviato regolarmente in {:.2}s!",
            game_name, time_to_game.unwrap().as_secs_f64());
        true
    } else {
        println!("RISULTATO: [FALLITO] {} non e' arrivato all'esecuzione del codice di gioco.", game_name);
        false
    }
}

fn main() {
    let mario_ok = test_game("Hotel Mario", "giochi/Hotel Mario (USA).chd", false, 1600);
    let striker_ok = test_game("Striker Pro", "giochi/Striker Pro (USA).chd", false, 1600);
    let dl_ok = test_game("Dragon's Lair", "giochi/Dragon's Lair (USA).chd", true, 1600);
    let rise_ok = test_game("Rise of the Robots", "giochi/Rise of the Robots (Europe).chd", false, 1600);

    println!("\n========================================================");
    println!("RIASSUNTO VERIFICA MULTI-GIOCO:");
    println!("  Hotel Mario:         {}", if mario_ok { "PASS" } else { "FAIL" });
    println!("  Striker Pro:         {}", if striker_ok { "PASS" } else { "FAIL" });
    println!("  Dragon's Lair:       {}", if dl_ok { "PASS" } else { "FAIL" });
    println!("  Rise of the Robots:  {}", if rise_ok { "PASS" } else { "FAIL" });
    println!("========================================================");

    if !mario_ok || !striker_ok || !dl_ok || !rise_ok {
        std::process::exit(1);
    }
}
