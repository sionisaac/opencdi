//! Disposable BIOS pointer-service contracts with full machine/device timing.
//! Scratch calls run in user state. No user NVRAM or app settings are saved.
use opencdi::{machine::Machine, mcd212::VideoMode};

fn call(m: &mut Machine, code: u16, d: &[u32], a0: u32) -> bool {
    m.bus.mcd212.plane_a[0x70000..0x70006].copy_from_slice(&[
        0x4e,
        0x40,
        (code >> 8) as u8,
        code as u8,
        0x60,
        0xfe,
    ]);
    m.cpu.set_sr(0);
    m.cpu.dar[15] = 0x7f000;
    m.cpu.dar[..d.len()].copy_from_slice(d);
    m.cpu.dar[8] = a0;
    m.cpu.stopped = 0;
    m.cpu.jump(0x70000);
    for _ in 0..200 {
        m.run_frame();
        if m.cpu.pc == 0x70004 {
            return m.cpu.get_ccr() & 1 == 0;
        }
    }
    panic!("service {code:04x} did not return (pc={:08x})", m.cpu.pc);
}

fn check(m: &mut Machine, path: u32, x: i32, y: i32, buttons: u8, checks: &mut usize) {
    assert!(call(m, 0x8d, &[path, 0x59, 0], 0), "PT_Coord error");
    let expected = (((x as u16) as u32) << 16) | (y as u16) as u32;
    assert_eq!(m.cpu.dar[1], expected, "BIOS coordinate differs");
    assert_eq!(
        m.cpu.dar[0] & 0xffff,
        0x8000 | buttons as u32,
        "BIOS active/button state differs"
    );
    *checks += 1;
}

fn mono4_cases(m: &mut Machine, mode: VideoMode, path: u32, checks: &mut usize) {
    let scale_y = |y: i32| if matches!(mode, VideoMode::Ntsc) { (y * 6 + 3) / 7 } else { y };
    for (x,y,b) in [(0,0,0),(1,1,1),(127,127,2),(128,128,3),(255,255,4),
                    (767,559,0),(384,280,0),(384,280,1),(384,280,0),(-100,-100,0),(1000,1000,0)] {
        m.bus.mono4.as_deref_mut().unwrap().ikat.update_absolute_pointer(x,y,b);
        for _ in 0..3 { m.run_frame(); }
        println!("M4_POINTER input={x},{y},{b} mode={mode:?}");
        check(m,path,x.clamp(0,767),scale_y(y.clamp(0,559)),if b & 4 != 0 { 3 } else { b & 3 },checks);
    }
    let ikat = &mut m.bus.mono4.as_deref_mut().unwrap().ikat;
    ikat.update_absolute_pointer(400,300,0);
    for _ in 0..3 { m.run_frame(); }
    assert!(call(m,0x8e,&[path,0x59,4,(50 << 16) | 25],0));
    check(m,path,350,scale_y(300)-25,0,checks);
    assert!(call(m,0x8e,&[path,0x59,3,(120 << 16) | 80],0));
    check(m,path,120,80,0,checks);
    m.bus.mono4.as_deref_mut().unwrap().ikat.update_pointer(401,307,0);
    for _ in 0..3 { m.run_frame(); }
    check(m,path,121,80 + scale_y(307)-scale_y(300),0,checks);
    assert!(!call(m,0x8d,&[0xffff,0x59,0],0)); *checks += 1;
    assert!(call(m,0x8f,&[path],0));
    assert!(!call(m,0x8d,&[path,0x59,0],0)); *checks += 1;
}

fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 1 {
        eprintln!("usage: bios_io_contract <cdi220b.rom|cdi490a.rom>");
        std::process::exit(2);
    }
    let mut checks = 0;
    for mode in [VideoMode::Pal, VideoMode::Ntsc] {
        // This BIOS scales slave Y coordinates to 480 lines in NTSC. Explicit
        // origin zero removes the player's separate letterbox-origin setting.
        let scale_y = |y: i32| {
            if matches!(mode, VideoMode::Ntsc) {
                (y * 6 + 3) / 7
            } else {
                y
            }
        };
        let mut m = Machine::new();
        m.bus.nvram.persist = false;
        let report = m.load_bios(&args[0]).expect("load BIOS");
        assert!(
            report.known.is_some_and(|r| matches!(r.file, "cdi220b.rom" | "cdi490a.rom")),
            "identified 220 F2 or 490 BIOS required"
        );
        m.set_video_mode(mode);
        m.reset();
        for _ in 0..700 {
            m.run_frame();
        }
        m.bus.mcd212.plane_a[0x71000..0x71005].copy_from_slice(b"/ptr\0");
        assert!(call(&mut m, 0x84, &[3], 0x71000), "open pointer");
        let path = m.cpu.dar[0] & 0xffff;
        // The player shell can leave a video-mode-specific pointer origin.
        // Set an explicit origin before comparing raw device coordinates.
        assert!(call(&mut m, 0x8e, &[path, 0x59, 4, 0], 0), "reset origin");
        if m.bus.mono4.is_some() {
            mono4_cases(&mut m, mode, path, &mut checks);
            println!("POINTER mode={mode:?} PASS");
            continue;
        }
        for (x, y, b) in [
            (0, 0, 0),
            (1, 1, 1),
            (127, 127, 2),
            (128, 128, 3),
            (255, 255, 4),
            (767, 559, 0),
            (384, 280, 0),
            (384, 280, 1),
            (384, 280, 0),
            (-100, -100, 0),
            (1000, 1000, 0),
        ] {
            m.bus.slave.update_mouse(x, y, b);
            for _ in 0..3 {
                m.run_frame();
            }
            let buttons = if b & 4 != 0 { 3 } else { b & 3 };
            check(
                &mut m,
                path,
                x.clamp(0, 767),
                scale_y(y.clamp(0, 559)),
                buttons,
                &mut checks,
            );
        }
        m.bus.slave.update_mouse(400, 300, 0);
        for _ in 0..3 {
            m.run_frame();
        }
        // Numeric SS_PT selectors observed for this identified firmware:
        // PT_Coord=0, PT_Pos=3, PT_Org=4. Do not infer them from TOC order.
        assert!(
            call(&mut m, 0x8e, &[path, 0x59, 4, (50 << 16) | 25], 0),
            "set origin"
        );
        check(&mut m, path, 350, scale_y(300) - 25, 0, &mut checks);
        assert!(
            call(&mut m, 0x8e, &[path, 0x59, 3, (120 << 16) | 80], 0),
            "set position"
        );
        check(&mut m, path, 120, 80, 0, &mut checks);
        if matches!(mode, VideoMode::Pal) {
            assert_eq!(
                (m.bus.slave.device_mouse_x, m.bus.slave.device_mouse_y),
                (170, 105)
            );
        }
        let (x, y) = (
            m.bus.slave.device_mouse_x + 1,
            m.bus.slave.device_mouse_y + 7,
        );
        m.bus.slave.update_mouse(x, y, 0);
        for _ in 0..3 {
            m.run_frame();
        }
        check(&mut m, path, x - 50, scale_y(y) - 25, 0, &mut checks);
        assert!(
            !call(&mut m, 0x8d, &[0xffff, 0x59, 0], 0),
            "invalid path accepted"
        );
        checks += 1;
        assert!(call(&mut m, 0x8f, &[path], 0), "close pointer");
        assert!(
            !call(&mut m, 0x8d, &[path, 0x59, 0], 0),
            "closed path accepted"
        );
        checks += 1;
        println!("POINTER mode={mode:?} PASS");
    }
    println!("BIOS_IO_CONTRACT checks={checks} passed={checks}");
}
