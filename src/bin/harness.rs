use opencdi::audio::to_i16;
use opencdi::input_script::{self, Event, Player};
use opencdi::bus::BoardKind;
use opencdi::machine::Machine;
use opencdi::mcd212::{VideoMode, SCREEN_HEIGHT, SCREEN_WIDTH};

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn fnv(mut h: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

fn video_hash(h: u64, fb: &[u32]) -> u64 {
    let mut h = h;
    for &p in fb {
        h = fnv(h, &p.to_le_bytes());
    }
    h
}

// Hashes the frames the last run_frame made; counts them and those that are not silent.
fn drain_audio(m: &mut Machine, buf: &mut Vec<(f32, f32)>, h: &mut u64, samples: &mut u64, nonzero: &mut u64) {
    buf.clear();
    m.drain_audio(buf);
    for &(l, r) in buf.iter() {
        *h = fnv(*h, &l.to_bits().to_le_bytes());
        *h = fnv(*h, &r.to_bits().to_le_bytes());
        *samples += 1;
        *nonzero += (l != 0.0 || r != 0.0) as u64;
    }
}

fn dump_ppm(fb: &[u32], path: &str) {
    let mut v = format!("P6\n{} {}\n255\n", SCREEN_WIDTH, SCREEN_HEIGHT).into_bytes();
    for &p in fb {
        v.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8]);
    }
    std::fs::write(path, v).expect("write ppm");
}

fn usage() -> ! {
    eprintln!("usage: harness <bios.rom> <disc|none> <frames> [--dvc <rom>] [--video-levels full|studio] [--ppm <out.ppm>] [--ntsc] [--autoplay [--require-launch] [--print-autoplay-script]] [--input <script|@file> | --post-input <script|@file>] [--absolute-input | --absolute-game-input] [--sample <n> [--sample-audio]] [--dump-every <n> <prefix>] [--roundtrip <frame>] [--counters] [--idle-pc <hex>] [--board mono1|mono4] [--audio-out <file.s16>] [--nvram-in <file>] [--nvram-out <file>] [--state-in <file>] [--state-out <file>] [--peek <hex-address> <bytes>]");
    std::process::exit(2);
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 4 {
        usage();
    }
    let (bios, disc) = (&a[1], &a[2]);
    let frames: u64 = a[3].parse().unwrap_or_else(|_| usage());
    let mut dvc: Option<&String> = None;
    let mut ppm: Option<&String> = None;
    let mut ntsc = false;
    let mut autoplay = false;
    let mut require_launch = false;
    let mut print_script = false;
    let mut input: Option<String> = None;
    let mut post_input = false;
    let mut absolute_input = false;
    let mut absolute_game_input = false;
    let mut sample = 0u64;
    let mut sample_audio = false;
    let mut dump: Option<(u64, String)> = None;
    let mut roundtrip: Option<u64> = None;
    let mut counters = false;
    let mut board: Option<BoardKind> = None;
    let mut idle_pc: Option<u32> = None;
    let mut audio_out: Option<(String, Vec<u8>)> = None;
    let mut nvram_out: Option<&String> = None;
    let mut nvram_in: Option<&String> = None;
    let mut state_in: Option<&String> = None;
    let mut state_out: Option<&String> = None;
    let mut peeks: Vec<(u32,usize)> = Vec::new();
    let mut i = 4;
    while i < a.len() {
        match a[i].as_str() {
            "--absolute-input" => absolute_input = true,
            "--absolute-game-input" => absolute_game_input = true,
            "--dvc" => { i += 1; dvc = a.get(i); }
            "--video-levels" => {
                i += 1;
                match a.get(i).map(|s| s.as_str()) {
                    Some("full") => opencdi::mcd212::set_studio_levels(false),
                    Some("studio") => opencdi::mcd212::set_studio_levels(true),
                    _ => usage(),
                }
            }
            "--ppm" => { i += 1; ppm = a.get(i); }
            "--audio-out" => { i += 1; audio_out = Some((a.get(i).cloned().unwrap_or_else(|| usage()), Vec::new())); }
            "--nvram-out" => { i += 1; nvram_out = Some(a.get(i).unwrap_or_else(|| usage())); }
            "--nvram-in" => { i += 1; nvram_in = Some(a.get(i).unwrap_or_else(|| usage())); }
            "--state-in" => { i += 1; state_in = Some(a.get(i).unwrap_or_else(|| usage())); }
            "--state-out" => { i += 1; state_out = Some(a.get(i).unwrap_or_else(|| usage())); }
            "--peek" => {
                let at = a.get(i+1).and_then(|s| u32::from_str_radix(s.trim_start_matches("0x"),16).ok()).unwrap_or_else(|| usage());
                let n = a.get(i+2).and_then(|s| s.parse::<usize>().ok()).filter(|n| *n <= 256).unwrap_or_else(|| usage());
                peeks.push((at,n)); i += 2;
            }
            "--ntsc" => ntsc = true,
            "--counters" => counters = true,
            "--board" => {
                i += 1;
                board = Some(a.get(i).and_then(|s| BoardKind::parse(s)).unwrap_or_else(|| usage()));
            }
            "--idle-pc" => { i += 1; idle_pc = Some(a.get(i).and_then(|s| u32::from_str_radix(s.trim_start_matches("0x"), 16).ok()).unwrap_or_else(|| usage())); }
            "--autoplay" => autoplay = true,
            "--require-launch" => { autoplay = true; require_launch = true; }
            "--print-autoplay-script" => print_script = true,
            "--input" | "--post-input" => {
                if input.is_some() { usage(); }
                post_input = a[i] == "--post-input";
                if post_input { autoplay = true; require_launch = true; }
                i += 1; input = Some(a.get(i).cloned().unwrap_or_else(|| usage()));
            }
            "--sample" => { i += 1; sample = a.get(i).and_then(|s| s.parse().ok()).unwrap_or_else(|| usage()); }
            "--sample-audio" => sample_audio = true,
            "--roundtrip" => { i += 1; roundtrip = Some(a.get(i).and_then(|s| s.parse().ok()).unwrap_or_else(|| usage())); }
            "--dump-every" => {
                let n = a.get(i + 1).and_then(|s| s.parse().ok()).filter(|&n: &u64| n > 0).unwrap_or_else(|| usage());
                dump = Some((n, a.get(i + 2).cloned().unwrap_or_else(|| usage())));
                i += 2;
            }
            _ => usage(),
        }
        i += 1;
    }
    if (print_script && !autoplay) || (autoplay && input.is_some() && !post_input) || (sample_audio && sample == 0) || roundtrip.is_some_and(|n| n >= frames)
        || (require_launch && disc == "none") || (state_in.is_some() && autoplay)
        || (state_out.is_some() && roundtrip.is_some()) {
        usage();
    }
    let mut player = input.map(|s| {
        let text = match s.strip_prefix('@') {
            Some(f) => std::fs::read_to_string(f).expect("read input script"),
            None => s,
        };
        Player::new(input_script::parse(&text).unwrap_or_else(|e| panic!("input script: {}", e)))
    });
    let mut recorded: Vec<Event> = Vec::new();

    let mut m = Machine::new();
    // Tests must never flush guest writes into a user's persistent save.
    m.bus.nvram.persist = false;
    let report = m.load_bios(bios).unwrap_or_else(|e| panic!("load bios {}: {}", bios, e));
    eprintln!("bios {}: {}", report.status, report.message);
    if let Some(b) = board {
        m.bus.set_board(b);
    }
    if let Some(p) = nvram_in {
        let bytes = std::fs::read(p).expect("read nvram input");
        match m.bus.mono4.as_deref_mut() {
            Some(board) => {
                assert!(matches!(bytes.len(),8192 | 32768), "Mono-IV NVRAM input must be 8192 or 32768 bytes");
                board.nvram.load_bytes(&bytes);
            }
            None => {
                assert_eq!(bytes.len(), m.bus.nvram.ram.len(), "Mono-I NVRAM input must be exactly 8192 bytes");
                m.bus.nvram.ram.copy_from_slice(&bytes);
            }
        }
    }
    eprintln!("board {:?}", m.bus.board);
    if let Some(p) = dvc {
        let report = m.load_dvc(p).unwrap_or_else(|e| panic!("load dvc {}: {}", p, e));
        eprintln!("dvc {}: {}", report.status, report.message);
    }
    if disc != "none" {
        m.load_disc(disc).expect("load disc");
    }
    m.set_video_mode(if ntsc { VideoMode::Ntsc } else { VideoMode::Pal });
    m.reset();
    if let Some(path) = state_in {
        let bytes = std::fs::read(path).expect("read input state");
        m.load_state(&bytes).expect("load input state");
        m.bus.nvram.persist = false;
    }

    let mut video_all = FNV_OFFSET;
    let mut audio_all = FNV_OFFSET;
    let mut audio_samples = 0u64;
    let mut audio_nonzero = 0u64;
    let mut disc_launched = false;
    let mut boot_frame: Option<u64> = None;
    let mut shell_ready_count = 0u32;
    let mut audio_buf: Vec<(f32, f32)> = Vec::new();
    let (mut win_samples, mut win_nonzero) = (0u64, 0u64);
    let mut idle_first: Option<u64> = None;

    // Round trip: save at `at`, run to the end (A), load, rerun the same frames with the same input (B).
    let mut saved: Option<(Vec<u8>, bool, u32, Option<Player>, Option<u64>)> = None;
    let mut pass_a: Option<(u64, u64, u64, u64)> = None;
    let mut resave_same = true;
    let mut start = 0;
    'passes: loop {
        for frame in start..frames {
            if roundtrip == Some(frame) && saved.is_none() {
                let mut buf = Vec::new();
                m.save_state(&[], &mut buf);
                saved = Some((buf, disc_launched, shell_ready_count, player.clone(), boot_frame));
                (video_all, audio_all, audio_samples) = (FNV_OFFSET, FNV_OFFSET, 0);
            }
            if autoplay && m.bus.cdrom.is_some() && !disc_launched {
                let (bus, cpu) = (&mut m.bus, &m.cpu);
                let shell_state = if bus.dvc.is_fitted() {
                    let s = bus.peek_long(0x00D08090u32.wrapping_add(0xFFFF9172)).unwrap_or(0);
                    if s >= 2 { s } else { bus.peek_long(0x0000C000u32.wrapping_add(0xFFFF9172)).unwrap_or(0) }
                } else {
                    bus.peek_long(0x0000C000u32.wrapping_add(0xFFFF9172)).unwrap_or(0)
                };
                if let Some((x,y,b)) = opencdi::boot::shell_play_click(bus, shell_state, &mut shell_ready_count) {
                    if absolute_input { opencdi::boot::send_game_pointer(bus,x,y,b); }
                    else { opencdi::boot::send_shell_pointer(bus,x,y,b); }
                    recorded.push(Event::Pointer { frame, x, y, buttons: b });
                }
                let started = opencdi::boot::disc_program_started(cpu.pc, frame,
                    if bus.mono4.is_some() { 0 } else { shell_state }, shell_ready_count);
                if started {
                    disc_launched = true;
                    boot_frame = Some(frame);
                    opencdi::boot::send_shell_pointer(bus,384,280,0);
                    recorded.push(Event::Pointer { frame, x: 384, y: 280, buttons: 0 });
                    eprintln!("AUTOPLAY handover_frame={frame} pc={:08x} shell_state={shell_state}", cpu.pc);
                }
            }
            let input_frame = if post_input { boot_frame.map(|at| frame - at) } else { Some(frame) };
            if let (Some(p),Some(input_frame)) = (player.as_mut(),input_frame) {
                if m.bus.slave.device_mouse_updated {
                    m.bus.slave.device_mouse_updated = false;
                    p.warp(m.bus.slave.device_mouse_x, m.bus.slave.device_mouse_y);
                }
                if let Some((x, y, b)) = p.step(input_frame) {
                    if absolute_input || absolute_game_input { opencdi::boot::send_game_pointer(&mut m.bus,x,y,b); }
                    else { opencdi::boot::send_shell_pointer(&mut m.bus,x,y,b); }
                }
            }

            m.run_frame();
            if idle_first.is_none() && idle_pc == Some(m.cpu.pc) {
                idle_first = Some(frame + 1);
            }

            video_all = video_hash(video_all, m.framebuffer());
            drain_audio(&mut m, &mut audio_buf, &mut audio_all, &mut audio_samples, &mut audio_nonzero);
            if let Some((_, bytes)) = audio_out.as_mut() {
                bytes.extend(audio_buf.iter().flat_map(|&(l, r)| [to_i16(l), to_i16(r)]).flat_map(i16::to_le_bytes));
            }
            if sample_audio {
                win_samples += audio_buf.len() as u64;
                win_nonzero += audio_buf.iter().filter(|&&(l, r)| l != 0.0 || r != 0.0).count() as u64;
            }
            if sample > 0 && (frame + 1) % sample == 0 {
                let nb = nonblack_count(m.framebuffer());
                eprintln!("SAMPLE frame={} nonblack={} pc={:08x} shell_ram={:08x} shell_dvc={:08x}", frame + 1, nb, m.cpu.pc,
                    m.bus.peek_long(0x5172).unwrap_or(0), m.bus.peek_long(0xD01202).unwrap_or(0));
                if let Some(board) = m.bus.mono4.as_deref() {
                    eprintln!("BOARD frame={} sectors={} lba={} drive_active={} selecting={} imr={:02x}", frame + 1,
                        board.ciap.sectors, board.ciap.lba, board.ciap.drive_active, board.ciap.selecting, board.ikat.imr);
                }
                if m.bus.dvc.is_fitted() {
                    let d = &m.bus.dvc;
                    eprintln!("MPEG frame={} video={} audio={} pictures={} visr={:04x} vier={:04x} aisr={:04x} aier={:04x}",frame+1,
                        d.video_playing,d.audio_enabled,d.frames_presented_count,
                        d.get_fmv_word(opencdi::dvc::FMV_ISR),d.get_fmv_word(opencdi::dvc::FMV_IER),
                        d.get_fma_word(opencdi::dvc::FMA_ISR),d.get_fma_word(opencdi::dvc::FMA_IER));
                }
                if std::env::var_os("CDI_PROBE_PC").is_some() {
                    let at = m.cpu.pc.saturating_sub(64);
                    let words: Vec<_> = (0..48).map(|n| m.bus.peek_word(at+n*2).unwrap_or(0)).collect();
                    eprintln!("PROBE pc={:08x} from={at:08x} words={:04x?} regs={:08x?}",m.cpu.pc,words,m.cpu.dar);
                }
                if std::env::var_os("CDI_PROBE_DISPLAY").is_some() {
                    let d = &m.bus.mcd212;
                    eprintln!("DISPLAY icm={:08x} tcr={:08x} order={} dcr={:04x?} ddr={:04x?} vsr={:08x?} dcp={:08x?} dca={:08x?} key={:08x?} mask={:08x?} matte={:08x?} weight={:?}",
                        d.image_coding_method,d.transparency_control,d.plane_order,d.dcr,d.ddr,d.vsr,d.dcp,d.dca,d.transparent_color,d.mask_color,d.matte_control,[d.weight_factor[0][384],d.weight_factor[1][384]]);
                }
                if sample_audio {
                    eprintln!("AUDIO frame={} samples={} nonzero={}", frame + 1, win_samples, win_nonzero);
                    (win_samples, win_nonzero) = (0, 0);
                }
            }
            if let Some((n, prefix)) = &dump {
                if (frame + 1) % n == 0 {
                    dump_ppm(m.framebuffer(), &format!("{}-{:05}.ppm", prefix, frame + 1));
                }
            }
        }
        if let (Some(at), Some((buf, dl, src, pl, bf))) = (roundtrip, saved.as_ref()) {
            let now = (video_all, video_hash(FNV_OFFSET, m.framebuffer()), audio_all, audio_samples);
            match pass_a {
                None => {
                    pass_a = Some(now);
                    m.load_state(buf).expect("load_state");
                    let mut again = Vec::new();
                    m.save_state(&[], &mut again);
                    resave_same = &again == buf;
                    eprintln!("ROUNDTRIP-RESAVE {} size={}", if resave_same { "same" } else { "DIFF" }, buf.len());
                    (disc_launched, shell_ready_count, player) = (*dl, *src, pl.clone());
                    boot_frame = *bf;
                    (video_all, audio_all, audio_samples) = (FNV_OFFSET, FNV_OFFSET, 0);
                    start = at;
                    continue 'passes;
                }
                Some(a) => {
                    eprintln!("ROUNDTRIP-DETAIL a={:016x?} b={:016x?}", a, now);
                    println!(
                        "ROUNDTRIP at={} frames={} video={} audio={}",
                        at, frames - at,
                        if (a.0, a.1) == (now.0, now.1) { "match" } else { "DIFFER" },
                        if (a.2, a.3) == (now.2, now.3) { "match" } else { "DIFFER" }
                    );
                    if a != now || !resave_same || (require_launch && !disc_launched) {
                        eprintln!("validation failed: replay mismatch, changed saved state, or required handover absent");
                        std::process::exit(1);
                    }
                    return;
                }
            }
        }
        break;
    }

    let video_last = video_hash(FNV_OFFSET, m.framebuffer());
    for (at,n) in peeks {
        let bytes: Vec<_> = (0..n).map(|i| m.bus.peek_byte(at.wrapping_add(i as u32))).collect();
        eprintln!("PEEK at={at:08x} bytes={bytes:02x?}");
    }
    if let Some(path) = state_out {
        let mut bytes = Vec::new();
        m.save_state(&[], &mut bytes);
        std::fs::write(path,bytes).expect("write output state");
    }
    let nonblack = nonblack_count(m.framebuffer());
    if let Some(p) = ppm {
        dump_ppm(m.framebuffer(), p);
    }
    if let Some((p, bytes)) = &audio_out {
        if let Err(e) = std::fs::write(p, bytes) {
            eprintln!("cannot write audio {}: {}", p, e);
            std::process::exit(1);
        }
    }
    if let Some(p) = nvram_out {
        if let Err(e) = std::fs::write(p, &m.bus.nvram.ram[..]) {
            eprintln!("cannot write nvram {}: {}", p, e);
            std::process::exit(1);
        }
    }
    println!(
        "\nHARNESS frames={} pc={:08x} nonblack={} video_last={:016x} video_all={:016x} audio={:016x} audio_samples={} audio_nonzero={}",
        frames, m.cpu.pc, nonblack, video_last, video_all, audio_all, audio_samples, audio_nonzero
    );
    if counters || idle_pc.is_some() {
        let idle = idle_first.map_or("none".to_string(), |f| f.to_string());
        let ciap = m.bus.mono4.as_deref().map_or(String::new(), |m4| {
            let c = &m4.ciap;
            format!(" buserr={} seeks={} sectors={} skipped={} dmas={} descrambled={} q={} abuf={} asec={} aover={}", m.bus.counters.bus_errors, c.seeks, c.sectors, c.skipped, c.dmas, c.descrambled, c.q_reports, c.audio_buffers, c.audio_sectors, c.audio_overlap)
        });
        println!("COUNTERS {}{} idle_first={}", m.bus.counters.summary(), ciap, idle);
    }
    if print_script {
        println!("AUTOPLAY_SCRIPT {}", input_script::format(&recorded));
    }
    if require_launch && !disc_launched {
        eprintln!("validation failed: no disc-program handover within {frames} frames");
        std::process::exit(1);
    }
}

// Pixels with any channel above the black level of the selected output range (16 studio, 0 full).
fn nonblack_count(fb: &[u32]) -> usize {
    let black = if opencdi::mcd212::studio_levels() { 16 } else { 0 };
    fb.iter().filter(|&&p| [16, 8, 0].iter().any(|&s| (p >> s) & 0xFF > black)).count()
}
