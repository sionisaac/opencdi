//! Disposable 490 file-service probe. Only hashes and status metadata leave RAM.
use m68k::AddressBus;
use opencdi::{bios_audit, machine::Machine, mcd212::VideoMode};

fn write(m: &mut Machine, at: u32, bytes: &[u8]) {
    for (n,b) in bytes.iter().enumerate() { m.bus.write_byte(at + n as u32,*b); }
    m.bus.consume_wait_cycles(); // host setup is not emulated CPU work
}
fn read(m: &Machine, at: u32, n: usize) -> Vec<u8> {
    (0..n).map(|i| m.bus.peek_byte(at + i as u32).expect("mapped allocated memory")).collect()
}
fn call(m: &mut Machine, code_at: u32, code: u16, d: &[u32], a0: u32) -> bool {
    write(m,code_at,&[0x4e,0x40,(code >> 8) as u8,code as u8,0x60,0xfe]);
    m.cpu.set_sr(m.cpu.get_sr() & 0x0700);
    m.cpu.dar[..d.len()].copy_from_slice(d);
    m.cpu.dar[8] = a0;
    m.cpu.jump(code_at);
    for _ in 0..600 {
        m.run_frame();
        if m.cpu.pc == code_at + 4 { return m.cpu.get_ccr() & 1 == 0; }
    }
    panic!("service {code:04x} timed out at {:08x}",m.cpu.pc);
}
fn main() {
    let a: Vec<_> = std::env::args_os().skip(1).collect();
    assert!((3..=5).contains(&a.len()) && a[3..].iter().all(|s| s == "--ntsc" || s == "--buffers"),
        "usage: bios_490_contract bios impeg Third-Degree-disc [--ntsc] [--buffers]");
    let mode = if a[3..].iter().any(|s| s == "--ntsc") { VideoMode::Ntsc } else { VideoMode::Pal };
    let mut m = Machine::new();
    m.bus.nvram.persist = false;
    assert!(m.load_bios(&a[0]).unwrap().known.is_some_and(|b| b.file == "cdi490a.rom"));
    m.load_dvc(&a[1]).unwrap(); m.load_disc(&a[2]).unwrap();
    assert_eq!(m.bus.cdrom.as_ref().unwrap().volume_id,"Third Degree");
    m.set_video_mode(mode); m.reset();
    let mut clicks = 0;
    let mut ready = false;
    for frame in 0..3000 {
        if let Some((x,y,b)) = opencdi::boot::shell_play_click(&m.bus,0,&mut clicks) {
            m.bus.mono4.as_deref_mut().unwrap().ikat.update_pointer(x,y,b);
        }
        m.run_frame();
        if frame > 500 && m.cpu.s_flag == 0 && (0x200000..0x280000).contains(&m.cpu.pc) {
            let found = bios_audit::modules(&m.bus.mcd212.plane_b[..]).iter().any(|a|
                a.name == "cdi_hotseat_loader" && (0x200000 + a.offset as u32..0x200000 + (a.offset+a.size) as u32).contains(&m.cpu.pc));
            if found { ready = true; break; }
        }
    }
    assert!(ready,"real application context not reached");
    // Borrow six bytes at the paused application's PC for the allocation call,
    // then restore them. All subsequent code/data uses BIOS-allocated memory.
    let borrowed = m.cpu.pc;
    let original = read(&m,borrowed,6);
    assert!(call(&mut m,borrowed,0x28,&[0x10000],0),"F$SRqMem failed");
    let base = m.cpu.dar[10];
    assert!(m.cpu.dar[0] >= 0x10000 && base & 1 == 0);
    write(&mut m,borrowed,&original);
    println!("ALLOC mode={mode:?} base={base:08x} bytes={}",m.cpu.dar[0]);
    if a[3..].iter().any(|s| s == "--buffers") { buffers(&mut m,base,mode); return; }
    let mut checks = 1;
    // Fixed fixture hashes independently obtained by disc_data_audit, not from
    // the BIOS's file reads. No file contents are embedded or exported.
    for (name,size,hash) in [
        ("copyright",69,"64bfced9a750d576c611a74507cb120def62020e"),
        ("cdi_hotseat_loader",30720,"0f96c8c2c1027b717ff21771579b81947b2380e9"),
        ("cdi_bumper",12288,"84c78c7e2bf862c30f2b990be74249d8128de5f8"),
        ("cdi_hotseat_attract",51200,"5960c207d4da8bc7c349cfebe8d0ae659a6c4fa1"),
    ] {
        write(&mut m,base+0x1000,format!("/cd/{name}\0").as_bytes());
        assert!(call(&mut m,base,0x84,&[1],base+0x1000),"open {name}"); checks += 1;
        let path = m.cpu.dar[0] & 0xffff;
        assert!(call(&mut m,base,0x8d,&[path,2],0));
        assert_eq!(m.cpu.dar[2],size); checks += 1;
        assert!(call(&mut m,base,0x8d,&[path,6],0)); checks += 1;
        write(&mut m,base+0x2000,&vec![0xA5;size as usize+16]);
        assert!(call(&mut m,base,0x89,&[path,size],base+0x2000));
        assert_eq!(m.cpu.dar[1],size); checks += 1;
        let data = read(&m,base+0x2000,size as usize);
        assert_eq!(opencdi::bios::sha1_hex(&data),hash,"payload {name}"); checks += 1;
        assert_eq!(read(&m,base+0x2000+size,16),vec![0xA5;16]); checks += 1;
        assert!(!call(&mut m,base,0x8d,&[path,6],0));
        assert_eq!(m.cpu.dar[1] & 0xffff,0xd3); checks += 1;
        assert!(call(&mut m,base,0x88,&[path,0],0)); checks += 1;
        assert!(call(&mut m,base,0x89,&[path,5],base+0x2000));
        assert_eq!(read(&m,base+0x2000,5),data[..5]); checks += 1;
        assert!(call(&mut m,base,0x8f,&[path],0)); checks += 1;
        println!("FILE mode={mode:?} name={name} bytes={size} sha1={hash} PASS");
    }
    assert!(!call(&mut m,base,0x89,&[0xffff,1],base+0x2000)); checks += 1;
    println!("BIOS_490_FILE_CONTRACT mode={mode:?} checks={checks} passed={checks}");
}

fn buffers(m: &mut Machine, base: u32, mode: VideoMode) {
    let mut expected = Vec::new();
    for lba in [2305,2306] {
        let mut raw = [0;opencdi::cdrom::CD_SECTOR_RAW_SIZE];
        assert!(m.bus.cdrom.as_mut().unwrap().read_sector(lba,&mut raw));
        if raw[0..2] == [0xFF,0] { for p in raw.chunks_exact_mut(2) { p.swap(0,1); } }
        assert_eq!((raw[15],raw[17],raw[18] & 0x22),(2,15,0x22));
        expected.push((raw[18],raw[19],raw[24..2348].to_vec()));
    }
    write(m,base+0x1000,b"/cd/bumper.rtf\0");
    assert!(call(m,base,0x84,&[1],base+0x1000));
    let path = m.cpu.dar[0] & 0xffff;
    write(m,base+0x3000,&vec![0;0x240]);
    write(m,base+0x3004,&1000u32.to_be_bytes());
    write(m,base+0x3008,&(1u32 << 15).to_be_bytes());
    write(m,base+0x300e,&(base+0x3100).to_be_bytes());
    write(m,base+0x313c,&(base+0x3200).to_be_bytes());
    write(m,base+0x3ff0,&vec![0xA5;0x4010]);
    for (pcl,next,buf) in [(0x3200,base+0x3220,base+0x4000),(0x3220,0,base+0x6000)] {
        write(m,base+pcl+6,&next.to_be_bytes());
        write(m,base+pcl+10,&buf.to_be_bytes());
        write(m,base+pcl+14,&1u32.to_be_bytes());
    }
    assert!(call(m,base,0x8e,&[path,0x2d],base+0x3000));
    for _ in 0..600 {
        if m.bus.peek_byte(base+0x3220).unwrap() & 1 != 0 { break; }
        m.run_frame();
    }
    for (i,(submode,coding,payload)) in expected.iter().enumerate() {
        let pcl = base+0x3200+i as u32*0x20;
        let buf = base+0x4000+i as u32*0x2000;
        println!("BUFFER index={i} ctrl={:02x} count={} pcb={:04x}",m.bus.peek_byte(pcl).unwrap(),m.bus.peek_long(pcl+24).unwrap(),m.bus.peek_word(base+0x3000).unwrap());
        assert_eq!(read(m,pcl,4),vec![1,0,*submode,*coding]);
        assert_eq!(m.bus.peek_long(pcl+24),Some(2324));
        assert_eq!(read(m,buf,2324),*payload,"buffer differs from raw disc");
        assert!(read(m,buf+2324,0x2000-2324).iter().all(|b| *b == 0xA5));
    }
    assert_eq!(m.bus.peek_long(base+0x313c),Some(0));
    assert!(call(m,base,0x8e,&[path,0x31],0));
    assert_eq!(m.bus.peek_word(base+0x3000).unwrap() & 0x8100,0x8100);
    let before = read(m,base+0x4000,0x4000);
    for _ in 0..15 { m.run_frame(); }
    assert_eq!(read(m,base+0x4000,0x4000),before);
    assert!(call(m,base,0x8f,&[path],0));
    println!("BIOS_490_BUFFER_CONTRACT mode={mode:?} PASS");
}
