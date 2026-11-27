//! Isolated, deterministic smoke/reproduction runs against local firmware/discs.
//! Reads user NVRAM into memory only. Never attaches or saves user persistence.
use opencdi::{
    bios_audit, boot,
    machine::Machine,
    mcd212::{SCREEN_HEIGHT, SCREEN_WIDTH, VideoMode},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

fn picture(m: &Machine, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut encoder = png::Encoder::new(
        fs::File::create(path)?,
        SCREEN_WIDTH as u32,
        SCREEN_HEIGHT as u32,
    );
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let rgb: Vec<_> = m
        .framebuffer()
        .iter()
        .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8])
        .collect();
    encoder.write_header()?.write_image_data(&rgb)?;
    Ok(())
}
fn fingerprint(pixels: &[u32]) -> u64 {
    pixels
        .iter()
        .step_by(23)
        .fold(0xcbf29ce484222325, |hash, p| {
            (hash ^ u64::from(*p)).wrapping_mul(0x100000001b3)
        })
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = std::env::args_os()
        .nth(1)
        .expect("usage: compatibility_probe case.json");
    let c: Value = serde_json::from_slice(&fs::read(arg)?)?;
    let out = PathBuf::from(c["output"].as_str().ok_or("output required")?);
    fs::create_dir_all(&out)?;
    let mut m = Machine::new();
    m.bus.nvram.persist = false;
    let report = m.load_bios(c["bios"].as_str().ok_or("bios required")?)?;
    m.bus.nvram.persist = false;
    m.load_dvc(c["dvc"].as_str().ok_or("dvc required")?)?;
    let disc = PathBuf::from(c["disc"].as_str().ok_or("disc required")?);
    m.load_disc(&disc)?;
    let ntsc = c["ntsc"].as_bool().unwrap_or_else(|| {
        m.bus
            .cdrom
            .as_ref()
            .is_some_and(|d| d.is_ntsc_hint(&disc.to_string_lossy()).unwrap_or(false))
    });
    m.set_video_mode(if ntsc {
        VideoMode::Ntsc
    } else {
        VideoMode::Pal
    });
    let mut nvram_loaded = false;
    if let Some(path) = c["nvram"].as_str() {
        let data = fs::read(path)?;
        if let Some(board) = m.bus.mono4.as_deref_mut() {
            if data.len() != board.nvram.ram.len() {
                return Err("NVRAM size mismatch".into());
            }
            board.nvram.ram.copy_from_slice(&data);
            board.nvram.save_path = None;
        } else {
            if data.len() != m.bus.nvram.ram.len() {
                return Err("NVRAM size mismatch".into());
            }
            m.bus.nvram.ram.copy_from_slice(&data);
        }
        nvram_loaded = true;
    }
    // Reproduce another launch in the same emulator, preserving the preceding
    // session's disposable NVRAM rather than silently constructing a new board.
    // This follows PlaySelectedDisc's full bus/CPU reset, not hot swapping.
    if let Some(path) = c["restart_state"].as_str() {
        m.load_state(&fs::read(path)?)?;
        m.bus.nvram.persist = false;
        if let Some(board) = m.bus.mono4.as_deref_mut() {
            board.nvram.save_path = None;
        }
        m.bus.reset_system();
    }
    m.reset();
    let mut launched = None;
    let mut clicks = 0;
    let mut pcs = BTreeMap::new();
    let mut samples = Vec::new();
    let mut audio = Vec::new();
    let mut audio_samples = 0u64;
    let mut audio_power = 0f64;
    let mut peak = 0f32;
    let mut shell_state = 0;
    let mut halted = false;
    let frames = c["frames"].as_u64().unwrap_or(2400).min(18000);
    let snapshots = c["snapshots"]
        .as_array()
        .cloned()
        .unwrap_or_else(|| vec![json!(800), json!(1600), json!(frames)]);
    let events = c["events"].as_array().cloned().unwrap_or_default();
    let from_state = c["load_state"].as_str();
    if let Some(path) = from_state {
        m.load_state(&fs::read(path)?)?;
        launched = Some(0);
    }
    if let Some(enabled)=c["diagnostic_wait_states"].as_bool() {
        m.bus.emulate_wait_states=enabled;
    }
    if let Some(addresses) = c["diagnostic_watch_writes"].as_array() {
        m.bus.watch_addr1 = addresses.first().and_then(Value::as_u64).unwrap_or(u32::MAX as u64) as u32;
        m.bus.watch_addr2 = addresses.get(1).and_then(Value::as_u64).unwrap_or(u32::MAX as u64) as u32;
        m.bus.watch_enabled = true;
    }
    // Explicit diagnostic perturbation, never enabled by compatibility runs.
    // Its result is a hardware hypothesis test, not a compatibility pass.
    let start = std::time::Instant::now();
    let mut input = None;
    for frame in 1..=frames {
        if let Some(at) = c["diagnostic_pointer_trace_at"].as_u64() {
            if let Some(board) = m.bus.mono4.as_deref_mut() { board.ikat.trace = frame >= at; }
        }
        if let Some(at)=c["diagnostic_drive_trace_at"].as_u64() {
            if let Some(board)=m.bus.mono4.as_deref_mut() { board.ciap.trace=frame>=at; }
        }
        if launched.is_none() {
            let at = 0xC000u32.wrapping_add(0xFFFF9172);
            let dvc = m
                .bus
                .peek_long(0xD08090u32.wrapping_add(0xFFFF9172))
                .unwrap_or(0);
            shell_state = if m.bus.dvc.is_fitted() && dvc >= 2 {
                dvc
            } else {
                m.bus.peek_long(at).unwrap_or(0)
            };
            if let Some((x, y, b)) = boot::shell_play_click(&m.bus, shell_state, &mut clicks) {
                boot::send_shell_pointer(&mut m.bus, x, y, b);
            }
        }
        for e in events.iter().filter(|e| e["frame"].as_u64() == Some(frame)) {
            let (x, y, b) = (
                e["x"].as_i64().unwrap_or(384) as i32,
                e["y"].as_i64().unwrap_or(280) as i32,
                e["buttons"].as_u64().unwrap_or(0) as u8,
            );
            input = Some((x, y, b, e["relative"].as_bool() == Some(true), frame));
        }
        if let Some((x, y, b, relative, at)) = input
            && (launched.is_some() || c["require_handover"].as_bool() != Some(true))
            && (at == frame || c["repeat_input"].as_bool() == Some(true))
        {
            if relative {
                boot::send_game_controller(&mut m.bus, x, y, b, (0, 0));
            } else {
                boot::send_game_pointer(&mut m.bus, x, y, b);
            }
            if !relative
                && c["continuous_absolute"].as_bool() == Some(true)
                && let Some(board) = m.bus.mono4.as_deref_mut()
                && board.ikat.imr & 8 != 0
            {
                board.ikat.write(0x1b, board.ikat.imr, 0);
            }
        }
        if let Some(q_mode)=c["diagnostic_q_mode"].as_bool()
            && frame>=c["diagnostic_q_mode_at"].as_u64().unwrap_or(0)
            && let Some(board)=m.bus.mono4.as_deref_mut() {
            board.ciap.q_mode=q_mode;
        }
        m.run_frame();
        // Explicit flow-control hypothesis, never a compatibility verdict.
        if c["diagnostic_audio_refill_ready"].as_bool()==Some(true)
            && m.bus.dvc.audio_enabled && m.bus.dvc.fma_dma_seen
            && m.bus.dvc.fma_demux.audio.len()<16384 && m.bus.dvc.fma_demux.due.len()<24 {
            let d=&mut m.bus.dvc;
            d.set_fma_word(2,d.get_fma_word(2)|8);
            d.or_fma_word(0x1a,8);
        }
        if launched.is_none()
            && (boot::disc_program_started(
                m.cpu.pc,
                frame,
                if m.bus.mono4.is_some() {
                    0
                } else {
                    shell_state
                },
                clicks,
            ) || (frame > 250
                && clicks >= 35
                && (m.disc_application_entered
                    || boot::disc_application_executing(&m.bus, m.cpu.pc, m.cpu.s_flag == 0))))
        {
            launched = Some(frame);
            boot::send_shell_pointer(&mut m.bus, 384, 280, 0);
        }
        *pcs.entry(format!("{:08x}", m.cpu.pc)).or_insert(0u64) += 1;
        audio.clear();
        m.drain_audio(&mut audio);
        for &(l, r) in &audio {
            audio_samples += 1;
            audio_power += (f64::from(l) * f64::from(l) + f64::from(r) * f64::from(r)) / 2.;
            peak = peak.max(l.abs()).max(r.abs());
        }
        if frame % 120 == 0 || frame == frames {
            let s = json!({"frame":frame,"pc":format!("{:08x}",m.cpu.pc),"supervisor":m.cpu.s_flag!=0,"shell_state":shell_state,"clicks":clicks,"launched":launched,"image_hash":format!("{:016x}",fingerprint(m.framebuffer())),"lba":m.bus.mono4.as_ref().map_or(m.bus.cdic.curr_lba,|b|b.ciap.lba),"audio_peak":peak});
            samples.push(s);
        }
        if snapshots.iter().any(|v| v.as_u64() == Some(frame)) {
            picture(&m, &out.join(format!("frame-{frame}.png")))?;
        }
        if m.bus.counters.halted {
            halted = true;
            break;
        }
    }
    if c["save_state"].as_bool() == Some(true) {
        let mut state = Vec::new();
        m.save_state(&[], &mut state);
        fs::write(out.join("checkpoint.state"), state)?;
    }
    if c["export_nvram"].as_bool() == Some(true) {
        let bytes = m.bus.mono4.as_deref().map(|b| &b.nvram.ram[..])
            .unwrap_or(&m.bus.nvram.ram[..]);
        fs::write(out.join("diagnostic-nvram.bin"), bytes)?;
    }
    if c["diagnostic_export_ram"].as_bool() == Some(true) {
        fs::write(out.join("diagnostic-sysram.bin"), &m.bus.sys_ram[..])?;
        fs::write(out.join("diagnostic-plane-a.bin"), &m.bus.mcd212.plane_a[..])?;
        fs::write(out.join("diagnostic-dvc-ram1.bin"), &m.bus.dvc.ram1[..])?;
    }
    let mut modules = Vec::new();
    for (base, ram) in [
        (0, &m.bus.mcd212.plane_a[..]),
        (0x200000, &m.bus.mcd212.plane_b[..]),
    ] {
        for module in bios_audit::modules(ram) {
            if module.module_type == 1 {
                modules.push(json!({"name":module.name,"start":format!("{:08x}",base+module.offset),"size":module.size}));
            }
        }
    }
    let result = json!({"title":c["title"],"bios":report.model(),"disc":disc.file_name(),"application_id":m.bus.cdrom.as_ref().map(|d|&d.application_id),"frames_requested":frames,"seconds":start.elapsed().as_secs_f64(),"launch_detected_frame":launched,"nvram_loaded":nvram_loaded,"halted":halted,"audio_samples":audio_samples,"audio_rms":if audio_samples>0{(audio_power/audio_samples as f64).sqrt()}else{0.},"audio_peak":peak,"samples":samples,"frame_end_pcs":pcs,"program_modules":modules,"classification":"automated observation only; not a playable/completed verdict"});
    let mut result=result;
    result["final_timer"]=json!({"tsr":m.bus.scc_timer.tsr,"tcr":m.bus.scc_timer.tcr,
        "reload":m.bus.scc_timer.reload,"timer0":m.bus.scc_timer.timer0,
        "picr1":m.bus.scc_timer.picr1,"lir":m.bus.scc_timer.lir,
        "pending":m.bus.scc_timer.timer_irq,"interrupt_acknowledgements":m.bus.counters.iack});
    result["final_cpu"]=json!({"pc":format!("{:08x}",m.cpu.pc),"registers":m.cpu.dar,
        "wait_word_a0":m.bus.peek_word(m.cpu.dar[8]),
        "signal_word_a6":m.bus.peek_word(m.cpu.dar[14].wrapping_add(0x7f0))});
    result["diagnostic_q_mode"]=c["diagnostic_q_mode"].clone();
    result["diagnostic_wait_states"]=c["diagnostic_wait_states"].clone();
    result["diagnostic_audio_refill_ready"]=c["diagnostic_audio_refill_ready"].clone();
    result["final_mpeg_audio"]=json!({"enabled":m.bus.dvc.audio_enabled,"armed":m.bus.dvc.audio_armed,
        "pio":m.bus.dvc.impeg_pio,"command":m.bus.dvc.get_fma_word(0),
        "isr":m.bus.dvc.get_fma_word(0x1a),"ier":m.bus.dvc.get_fma_word(0x1c),
        "timer":m.bus.dvc.get_fmv_word(0x64),"fifo_bytes":m.bus.dvc.fma_stream_fifo.len(),
        "parsed_bytes":m.bus.dvc.fma_demux.audio.len(),"clock":m.bus.dvc.dclk});
    result["final_mpeg_video"]=json!({"enabled":m.bus.dvc.decoder_enabled,"armed":m.bus.dvc.video_armed,
        "playing":m.bus.dvc.video_playing,"fifo_bytes":m.bus.dvc.dvc_stream_fifo.len(),
        "frames_decoded":m.bus.dvc.frames_decoded_count,"frames_presented":m.bus.dvc.frames_presented_count,
        "isr":m.bus.dvc.fmv_isr,"ier":m.bus.dvc.fmv_ier,"has_frame":m.bus.dvc.has_valid_frame,
        "status":m.bus.dvc.get_fmv_word(0x5e),"dts_head":m.bus.dvc.get_fmv_word(0xa0),
        "picture_queue":m.bus.dvc.get_fmv_word(0xa4)});
    if let Some(words)=c["watch_words"].as_array(){
        result["watched_words"]=json!(words.iter().filter_map(|v|v.as_u64()).map(|at|
            json!({"address":format!("{at:08x}"),"value":m.bus.peek_word(at as u32)})).collect::<Vec<_>>());
    }
    if let Some(board)=m.bus.mono4.as_deref() {
        result["final_ciap"]=json!({"running":board.ciap.running,"drive_active":board.ciap.drive_active,
            "audio_playing":board.ciap.audio_playing,"audio_wait":board.ciap.audio_wait,
            "event_armed":board.ciap.event_armed,"apcr":board.ciap.peek_reg(0x26),"astat":board.ciap.peek_reg(0x2a),
            "q_mode":board.ciap.q_mode,"asel":board.ciap.asel,"lba":board.ciap.lba,
            "isr":board.ciap.peek_reg(6),"ier":board.ciap.peek_reg(4),"bman":board.ciap.peek_reg(0x14),
            "file":board.ciap.peek_reg(0x12),"tacs":board.ciap.peek_reg(8)});
    }
    if let Some(sectors)=c["diagnostic_sectors"].as_array()
        && let Some(cd)=m.bus.cdrom.as_mut() {
        let mut headers=Vec::new();
        for sector in sectors.iter().filter_map(Value::as_u64) {
            let mut raw=[0;2352];
            if cd.read_sector(sector as u32,&mut raw) {
                headers.push(json!({"lba":sector,"header":&raw[12..24]}));
            }
        }
        result["diagnostic_sector_headers"]=json!(headers);
    }
    let bytes = serde_json::to_vec_pretty(&result)?;
    fs::write(out.join("result.json"), &bytes)?;
    std::io::stdout().write_all(&bytes)?;
    Ok(())
}
