//! Disposable file-service investigation in a real application process.
use m68k::StepResult;
use opencdi::mcd212::VideoMode;
use opencdi::{bios_audit, machine::Machine};

fn precise_eof(m: &mut Machine, path: u32) -> (bool, u32) {
    let mut state = Vec::new();
    m.save_state(&[], &mut state);
    m.bus.sys_ram[..6].copy_from_slice(&[0x4e, 0x40, 0, 0x8d, 0x60, 0xfe]);
    m.cpu.set_sr(0x0700);
    m.cpu.set_irq(0);
    m.cpu.dar[0] = path;
    m.cpu.dar[1] = 6;
    m.cpu.dar[8] = 0;
    m.cpu.jump(0x500000);
    let mut returned = false;
    let mut cdfm_path = Vec::new();
    let mut driver_d1 = None;
    let mut d1_changes = Vec::new();
    let mut restored_d1 = None;
    for _ in 0..20000 {
        if m.cpu.pc == 0x500004 {
            returned = true;
            break;
        }
        if (0x412a3a..0x413bd8).contains(&m.cpu.pc) {
            cdfm_path.push(format!(
                "{:08x}:{:04x}",
                m.cpu.pc,
                m.bus.peek_word(m.cpu.pc).unwrap_or(0)
            ));
        }
        if matches!(m.cpu.pc, 0x4134ce | 0x4134d8) {
            driver_d1 = Some(m.cpu.dar[1]);
        }
        let before = (m.cpu.pc, m.cpu.dar[1]);
        let stack_d1 = if m.cpu.pc == 0x407fa4 {
            // Identified firmware's MOVEM.L (SP)+,D0-D7/A0-A6. Check the
            // caller's saved D1 against the actual stack word, independently
            // of the instruction's register-load implementation.
            assert_eq!(m.bus.peek_word(m.cpu.pc), Some(0x4cdf));
            assert_eq!(m.bus.peek_word(m.cpu.pc + 2), Some(0x7fff));
            m.bus.peek_long(m.cpu.dar[15] + 4)
        } else {
            None
        };
        match m.cpu.step(&mut m.bus) {
            StepResult::Ok { .. } => {}
            StepResult::TrapInstruction { trap_num } => {
                m.cpu.take_trap_exception(&mut m.bus, trap_num);
            }
            other => panic!("precise EOF exited unexpectedly: {other:?}"),
        }
        if let Some(saved_d1) = stack_d1 {
            assert_eq!(m.cpu.dar[1], saved_d1, "MOVEM D1 differs from caller stack");
            restored_d1 = Some(saved_d1);
        }
        if before.1 != m.cpu.dar[1] && d1_changes.len() < 32 {
            d1_changes.push(format!(
                "{:08x}:{:08x}>{:08x}",
                before.0, before.1, m.cpu.dar[1]
            ));
        }
    }
    assert!(returned, "precise EOF timeout");
    let result = (m.cpu.get_ccr() & 1 == 0, m.cpu.dar[1]);
    assert_eq!(
        restored_d1,
        Some(result.1),
        "caller register-restore path not reached"
    );
    assert_eq!(
        driver_d1,
        Some(if result.0 { 0 } else { 0xd3 }),
        "CDFM driver EOF result"
    );
    println!(
        "EOF_PRECISE ok={} d1={:08x} driver_d1={:08x} saved_d1={:08x} cdfm_path={} d1_changes={}",
        result.0,
        result.1,
        driver_d1.unwrap(),
        restored_d1.unwrap(),
        cdfm_path.join(","),
        d1_changes.join(",")
    );
    m.load_state(&state)
        .expect("restore EOF comparison checkpoint");
    result
}
fn call(m: &mut Machine, code: u16, d: &[u32], a0: u32) -> bool {
    m.bus.sys_ram[..6].copy_from_slice(&[0x4e, 0x40, (code >> 8) as u8, code as u8, 0x60, 0xfe]);
    m.cpu.set_sr(m.cpu.get_sr() & 0x0700);
    m.cpu.dar[..d.len()].copy_from_slice(d);
    m.cpu.dar[8] = a0;
    m.cpu.jump(0x500000);
    for _ in 0..300 {
        m.run_frame();
        if m.cpu.pc == 0x500004 {
            println!(
                "CALL code={code:04x} carry={} d0={:08x} d1={:08x} d2={:08x}",
                m.cpu.get_ccr() & 1,
                m.cpu.dar[0],
                m.cpu.dar[1],
                m.cpu.dar[2]
            );
            return m.cpu.get_ccr() & 1 == 0;
        }
    }
    panic!("call timeout: {:08x}", m.cpu.pc);
}
fn main() {
    let a: Vec<_> = std::env::args_os().skip(1).collect();
    assert!(
        (3..=5).contains(&a.len()) && a[3..].iter().all(|s| s == "--ntsc" || s == "--stream" || s == "--buffers")
            && !(a[3..].iter().any(|s| s == "--stream") && a[3..].iter().any(|s| s == "--buffers")),
        "usage: bios_file_contract bios dvc Third-Degree-disc [--ntsc] [--stream | --buffers]"
    );
    let mode = if a[3..].iter().any(|s| s == "--ntsc") {
        VideoMode::Ntsc
    } else {
        VideoMode::Pal
    };
    let mut m = Machine::new();
    m.bus.nvram.persist = false;
    let report = m.load_bios(&a[0]).unwrap();
    assert!(
        report.known.is_some_and(|r| r.file == "cdi220b.rom"),
        "identified 220 F2 BIOS required"
    );
    m.load_dvc(&a[1]).unwrap();
    m.load_disc(&a[2]).unwrap();
    assert_eq!(
        m.bus.cdrom.as_ref().unwrap().volume_id,
        "Third Degree",
        "Third Degree fixture required"
    );
    m.set_video_mode(mode);
    m.reset();
    let mut clicks = 0;
    let mut ready = false;
    for frame in 0..1800 {
        let s = m.bus.peek_long(0xd01202).unwrap_or(0);
        let s = if s >= 2 {
            s
        } else {
            m.bus.peek_long(0x5172).unwrap_or(0)
        };
        if s >= 2 {
            clicks += 1;
            match clicks % 120 {
                10 | 35 => m.bus.slave.update_mouse(575, 335, 0),
                20 => m.bus.slave.update_mouse(575, 335, 1),
                _ => {}
            }
        }
        m.run_frame();
        if frame > 500 && m.cpu.s_flag == 0 && (0x200000..0x280000).contains(&m.cpu.pc) {
            let modules = bios_audit::modules(&m.bus.mcd212.plane_b[..]);
            if modules.iter().any(|a| {
                a.name == "cdi_hotseat_loader"
                    && m.cpu.pc >= 0x200000 + a.offset as u32
                    && m.cpu.pc < 0x200000 + (a.offset + a.size) as u32
            }) {
                println!("APPLICATION_CONTEXT frame={frame}");
                ready = true;
                break;
            }
        }
    }
    assert!(ready, "real application context not reached");
    if a[3..].iter().any(|s| s == "--buffers") {
        buffer_cases(&mut m, mode);
        return;
    }
    if a[3..].iter().any(|s| s == "--stream") {
        stream_cases(&mut m, mode);
        return;
    }
    let mut checks = 0;
    for (name, mode, size) in [("copyright", 1, 69), ("cdi_hotseat_loader", 4, 30720)] {
        let pathname = format!("/cd/{name}\0");
        m.bus.sys_ram[0x1000..0x1000 + pathname.len()].copy_from_slice(pathname.as_bytes());
        expect(
            &mut m,
            0x84,
            &[mode],
            0x501000,
            true,
            "open file",
            &mut checks,
        );
        let path = m.cpu.dar[0] & 0xffff;
        expect(&mut m, 0x8d, &[path, 2], 0, true, "file size", &mut checks);
        assert_eq!(m.cpu.dar[2], size);
        position(&mut m, path, 0, &mut checks);
        let precise = precise_eof(&mut m, path);
        expect(&mut m, 0x8d, &[path, 6], 0, true, "not EOF", &mut checks);
        assert_eq!(precise, (true, m.cpu.dar[1]), "precise/batched EOF differs");
        checks += 1;
        // Preserve the observed successful-return D1 value as evidence; the
        // May 1994 text says zero, but this firmware currently preserves 6.
        println!("EOF_BEFORE d1={:08x}", m.cpu.dar[1]);
        if name == "copyright" {
            read(&mut m, path, name, 0, 100, size, &mut checks);
            position(&mut m, path, size, &mut checks);
            eof(&mut m, path, &mut checks);
            seek(&mut m, path, 7, &mut checks);
            read(&mut m, path, name, 7, 13, 13, &mut checks);
            position(&mut m, path, 20, &mut checks);
            seek(&mut m, path, 0, &mut checks);
            read(&mut m, path, name, 0, 5, 5, &mut checks);
            read(&mut m, path, name, 5, 5, 5, &mut checks);
            position(&mut m, path, 10, &mut checks);
            // The host source is opened read-only; this tests guest permission
            // handling, not a write to the user's image.
            expect(
                &mut m,
                0x8a,
                &[path, 4],
                0x502000,
                false,
                "write through read-only handle",
                &mut checks,
            );
        } else {
            seek(&mut m, path, 2040, &mut checks);
            read(&mut m, path, name, 2040, 32, 32, &mut checks);
            position(&mut m, path, 2072, &mut checks);
            seek(&mut m, path, size - 8, &mut checks);
            read(&mut m, path, name, size - 8, 32, 8, &mut checks);
            position(&mut m, path, size, &mut checks);
            eof(&mut m, path, &mut checks);
            seek(&mut m, path, 1, &mut checks);
            read(&mut m, path, name, 1, 5, 5, &mut checks);
            position(&mut m, path, 6, &mut checks);
        }
        expect(&mut m, 0x8f, &[path], 0, true, "close file", &mut checks);
        expect(
            &mut m,
            0x89,
            &[path, 1],
            0x502000,
            false,
            "read closed handle",
            &mut checks,
        );
    }
    let missing = b"/cd/ocdi_missing_7c319\0";
    m.bus.sys_ram[0x1000..0x1000 + missing.len()].copy_from_slice(missing);
    expect(
        &mut m,
        0x84,
        &[1],
        0x501000,
        false,
        "missing file",
        &mut checks,
    );
    expect(
        &mut m,
        0x89,
        &[0xffff, 1],
        0x502000,
        false,
        "invalid handle",
        &mut checks,
    );
    println!("BIOS_FILE_CONTRACT mode={mode:?} checks={checks} passed={checks}");
}

fn stream_cases(m: &mut Machine, mode: VideoMode) {
    let mut checks = 0;
    let name = b"/cd/bumper.rtf\0";
    m.bus.sys_ram[0x1000..0x1000 + name.len()].copy_from_slice(name);
    expect(
        m,
        0x84,
        &[1],
        0x501000,
        true,
        "open real-time file",
        &mut checks,
    );
    let path = m.cpu.dar[0] & 0xffff;
    prepare_stream(m);
    // SS_Play=0x2d and SS_Abort=0x31 in the identified firmware. The PCB
    // selects all channels; audio 0/15 goes to AP, other payloads are ignored.
    expect(
        m,
        0x8e,
        &[path, 0x2d],
        0x503000,
        true,
        "start asynchronous play",
        &mut checks,
    );
    for _ in 0..10 {
        m.run_frame();
    }
    assert_eq!(
        m.bus.peek_word(0x503000).unwrap() & 0x100,
        0,
        "stream ended before abort test"
    );
    checks += 1;
    expect(
        m,
        0x8e,
        &[path, 0x2d],
        0x503000,
        false,
        "second play on busy path",
        &mut checks,
    );
    m.bus.sys_ram[0x2000] = 0xa5;
    expect(
        m,
        0x89,
        &[path, 1],
        0x502000,
        false,
        "synchronous read on busy path",
        &mut checks,
    );
    assert_eq!(m.bus.sys_ram[0x2000], 0xa5, "busy read changed buffer");
    checks += 1;
    expect(
        m,
        0x8d,
        &[path, 5],
        0,
        true,
        "position during play",
        &mut checks,
    );
    assert!(m.cpu.dar[2] > 0, "asynchronous play made no progress");
    checks += 1;
    expect(
        m,
        0x8e,
        &[path, 0x31],
        0,
        true,
        "abort asynchronous play",
        &mut checks,
    );
    aborted(m, &mut checks);
    let decoded = m.bus.cdic.audio.total_sectors_decoded;
    expect(
        m,
        0x8d,
        &[path, 5],
        0,
        true,
        "position after abort",
        &mut checks,
    );
    let position = m.cpu.dar[2];
    for _ in 0..15 {
        m.run_frame();
    }
    assert_eq!(
        m.bus.cdic.audio.total_sectors_decoded, decoded,
        "disc audio decoded after abort"
    );
    checks += 1;
    expect(
        m,
        0x8d,
        &[path, 5],
        0,
        true,
        "position remains stopped",
        &mut checks,
    );
    assert_eq!(m.cpu.dar[2], position, "file position advanced after abort");
    checks += 1;
    expect(
        m,
        0x88,
        &[path, 0],
        0,
        true,
        "rewind after abort",
        &mut checks,
    );
    prepare_stream(m);
    expect(
        m,
        0x8e,
        &[path, 0x2d],
        0x503000,
        true,
        "restart after abort",
        &mut checks,
    );
    for _ in 0..4 {
        m.run_frame();
    }
    assert_eq!(
        m.bus.peek_word(0x503000).unwrap() & 0x100,
        0,
        "restart ended prematurely"
    );
    checks += 1;
    expect(
        m,
        0x8e,
        &[path, 0x31],
        0,
        true,
        "abort restarted play",
        &mut checks,
    );
    aborted(m, &mut checks);
    expect(
        m,
        0x8f,
        &[path],
        0,
        true,
        "close real-time file",
        &mut checks,
    );
    expect(
        m,
        0x8e,
        &[0xffff, 0x31],
        0,
        false,
        "abort invalid handle",
        &mut checks,
    );
    println!("BIOS_STREAM_CONTRACT mode={mode:?} checks={checks} passed={checks}");
}
// Bounded CD-i root-directory lookup, independent of the BIOS filesystem calls.
fn fixture_extent(cd: &mut opencdi::cdrom::CdRom, wanted: &str) -> u32 {
    let be = |d: &[u8], at: usize| u32::from_be_bytes(d[at..at + 4].try_into().unwrap());
    let mut sector = [0; 2048];
    assert!(cd.read_data_sector(16, &mut sector));
    assert_eq!(&sector[1..6], b"CD-I ");
    let table = be(&sector, 148);
    assert!(cd.read_data_sector(table, &mut sector));
    let root = be(&sector, 2);
    assert!(cd.read_data_sector(root, &mut sector));
    let size = be(&sector, 14) as usize;
    assert!((34..=65536).contains(&size));
    let mut directory = Vec::new();
    for n in 0..size.div_ceil(2048) {
        assert!(cd.read_data_sector(root + n as u32, &mut sector));
        directory.extend_from_slice(&sector);
    }
    let mut at = 0;
    while at < size {
        let n = directory[at] as usize;
        if n == 0 { at = (at / 2048 + 1) * 2048; continue; }
        assert!(n >= 34 && at + n <= size && at % 2048 + n <= 2048);
        let d = &directory[at..at + n];
        let len = d[32] as usize;
        assert!(33 + len <= n);
        let name = String::from_utf8_lossy(&d[33..33 + len]);
        if name.split(';').next() == Some(wanted) {
            return be(d, 6).checked_add(d[1] as u32).unwrap();
        }
        at += n;
    }
    panic!("fixture file not found: {wanted}");
}

fn buffer_cases(m: &mut Machine, mode: VideoMode) {
    let mut checks = 0;
    let cd = m.bus.cdrom.as_mut().unwrap();
    let extent = fixture_extent(cd, "bumper.rtf");
    let mut expected = Vec::new();
    let mut channel = None;
    for lba in extent..extent + 256 {
        let mut raw = [0; opencdi::cdrom::CD_SECTOR_RAW_SIZE];
        assert!(cd.read_sector(lba, &mut raw));
        if raw[0..2] == [0xFF, 0] {
            for pair in raw.chunks_exact_mut(2) { pair.swap(0, 1); }
        }
        if raw[15] != 2 || raw[16..20] != raw[20..24] {
            let mask = opencdi::cdic::generate_sector_scramble();
            for i in 12..raw.len() { raw[i] ^= mask[i]; }
        }
        assert_eq!(raw[15], 2);
        assert_eq!(&raw[16..20], &raw[20..24]);
        if raw[18] & 2 != 0 && raw[17] < 32 {
            let wanted = *channel.get_or_insert(raw[17]);
            if raw[17] == wanted {
                let len = if raw[18] & 0x20 != 0 { 2324 } else { 2048 };
                expected.push((lba, raw[18], raw[19], raw[24..24 + len].to_vec()));
                if expected.len() == 2 { break; }
            }
        }
    }
    assert_eq!(expected.len(), 2, "two video sectors required in bounded fixture window");
    let channel = channel.unwrap();
    let name = b"/cd/bumper.rtf\0";
    m.bus.sys_ram[0x1000..0x1000 + name.len()].copy_from_slice(name);
    expect(m, 0x84, &[1], 0x501000, true, "open buffered real-time file", &mut checks);
    let path = m.cpu.dar[0] & 0xFFFF;
    prepare_stream(m);
    m.bus.sys_ram[0x3008..0x300C].copy_from_slice(&(1u32 << channel).to_be_bytes());
    m.bus.sys_ram[0x300C..0x300E].fill(0); // audio goes to neither AP nor a CIL
    m.bus.sys_ram[0x300E..0x3012].copy_from_slice(&0x503100u32.to_be_bytes());
    m.bus.sys_ram[0x3100..0x3240].fill(0);
    let ci = 0x3100 + channel as usize * 4;
    m.bus.sys_ram[ci..ci + 4].copy_from_slice(&0x503200u32.to_be_bytes());
    m.bus.sys_ram[0x3FF0..0x6000].fill(0xA5);
    m.bus.sys_ram[0x6000..0x8000].fill(0x5A);
    for (pcl, next, buf) in [(0x3200, 0x503220u32, 0x504000u32), (0x3220, 0, 0x506000)] {
        m.bus.sys_ram[pcl + 6..pcl + 10].copy_from_slice(&next.to_be_bytes());
        m.bus.sys_ram[pcl + 10..pcl + 14].copy_from_slice(&buf.to_be_bytes());
        m.bus.sys_ram[pcl + 14..pcl + 18].copy_from_slice(&1u32.to_be_bytes());
    }
    expect(m, 0x8E, &[path, 0x2D], 0x503000, true, "start linked-buffer play", &mut checks);
    for _ in 0..600 {
        if m.bus.sys_ram[0x3220] & 1 != 0 { break; }
        m.run_frame();
    }
    assert_eq!(m.bus.sys_ram[0x3220] & 1, 1, "second linked buffer never filled");
    checks += 1;
    for (i, (lba, submode, coding, data)) in expected.iter().enumerate() {
        let pcl = 0x3200 + i * 0x20;
        let buf = 0x4000 + i * 0x2000;
        assert_eq!(m.bus.sys_ram[pcl], 1, "buffer-full without data error");
        assert_eq!(&m.bus.sys_ram[pcl + 2..pcl + 4], &[*submode, *coding]);
        assert_eq!(m.bus.peek_long(0x500000 + pcl as u32 + 24), Some(data.len() as u32));
        checks += 1;
        assert_eq!(&m.bus.sys_ram[buf..buf + data.len()], data.as_slice(), "BIOS payload differs from disc");
        checks += 1;
        let guard = if i == 0 { 0xA5 } else { 0x5A };
        assert!(m.bus.sys_ram[buf + data.len()..buf + 0x2000].iter().all(|b| *b == guard));
        checks += 1;
        println!("BUFFER mode={mode:?} index={i} lba={lba} channel={channel} bytes={} sha1={} PASS", data.len(), opencdi::bios::sha1_hex(data));
    }
    assert!(m.bus.sys_ram[0x3FF0..0x4000].iter().all(|b| *b == 0xA5));
    assert_eq!(m.bus.peek_long(0x500000 + ci as u32), Some(0), "CIL must advance through both PCLs");
    checks += 1;
    expect(m, 0x8E, &[path, 0x31], 0, true, "abort buffered play", &mut checks);
    aborted(m, &mut checks);
    let filled = m.bus.sys_ram[0x4000..0x8000].to_vec();
    for _ in 0..15 { m.run_frame(); }
    assert_eq!(m.bus.sys_ram[0x4000..0x8000], filled);
    checks += 1;
    expect(m, 0x8F, &[path], 0, true, "close buffered file", &mut checks);
    println!("BIOS_BUFFER_CONTRACT mode={mode:?} checks={checks} passed={checks}");
}

fn prepare_stream(m: &mut Machine) {
    m.bus.sys_ram[0x3000..0x3040].fill(0);
    m.bus.sys_ram[0x3004..0x3008].copy_from_slice(&1000u32.to_be_bytes());
    m.bus.sys_ram[0x3008..0x300c].copy_from_slice(&u32::MAX.to_be_bytes());
    m.bus.sys_ram[0x300c..0x300e].copy_from_slice(&0x8001u16.to_be_bytes());
}
fn aborted(m: &mut Machine, checks: &mut usize) {
    let status = m.bus.peek_word(0x503000).unwrap();
    let signal = m.bus.peek_word(0x503002).unwrap();
    assert_eq!(status & 0x8100, 0x8100, "abort must set done and error");
    assert_eq!(signal, 0x060a, "abort status must report E$Abort");
    assert_eq!(
        m.bus.cdic.disc_command, 0,
        "drive transfer remained active after abort"
    );
    *checks += 1;
    println!("ABORT status={status:04x} signal={signal:04x} stopped=true");
}

fn expect(m: &mut Machine, code: u16, d: &[u32], a0: u32, ok: bool, label: &str, n: &mut usize) {
    assert_eq!(call(m, code, d, a0), ok, "{label}");
    *n += 1;
    println!("CHECK\t{label}\tPASS");
}
fn position(m: &mut Machine, path: u32, at: u32, n: &mut usize) {
    expect(m, 0x8d, &[path, 5], 0, true, "file position", n);
    assert_eq!(m.cpu.dar[2], at, "file position");
}
fn seek(m: &mut Machine, path: u32, at: u32, n: &mut usize) {
    expect(m, 0x88, &[path, at], 0, true, "seek", n);
    position(m, path, at, n);
}
fn read(
    m: &mut Machine,
    path: u32,
    name: &str,
    at: u32,
    requested: u32,
    count: u32,
    n: &mut usize,
) {
    m.bus.sys_ram[0x2000..0x2100].fill(0xa5);
    expect(m, 0x89, &[path, requested], 0x502000, true, "read", n);
    assert_eq!(m.cpu.dar[1], count, "returned byte count");
    assert!(
        m.bus.sys_ram[0x2000 + requested as usize..0x2100]
            .iter()
            .all(|&b| b == 0xa5),
        "read overwrote guard"
    );
    println!(
        "READ\t{name}\t{at}\t{count}\t{}",
        opencdi::bios::sha1_hex(&m.bus.sys_ram[0x2000..0x2000 + count as usize])
    );
}
fn eof(m: &mut Machine, path: u32, n: &mut usize) {
    let precise = precise_eof(m, path);
    expect(m, 0x8d, &[path, 6], 0, false, "EOF status", n);
    assert_eq!(
        precise,
        (false, m.cpu.dar[1]),
        "precise/batched EOF differs"
    );
    *n += 1;
    assert_eq!(m.cpu.dar[1] & 0xffff, 0xd3, "E$EOF");
    m.bus.sys_ram[0x2000] = 0xa5;
    expect(m, 0x89, &[path, 1], 0x502000, false, "read at EOF", n);
    assert_eq!(m.cpu.dar[1] & 0xffff, 0xd3, "E$EOF");
    assert_eq!(m.bus.sys_ram[0x2000], 0xa5, "EOF read changed buffer");
}
