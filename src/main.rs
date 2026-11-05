// OpenCDi Release Build - 2026-09-29 15:45
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
use std::path::{Path, PathBuf};
use log::{info, error};
use std::collections::HashSet;
use std::rc::Rc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{DeviceEvent, DeviceId, ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode as Key, ModifiersState, PhysicalKey};
use winit::window::{CursorGrabMode, Fullscreen, Window, WindowId};

use opencdi::audio::HostAudioOutput;
use opencdi::frontend::{app_data_dir, app_data_file, migrate_app_data_dir, pop_action, push_action, ArtworkPathKind, DisplayMode, GuiAction, InputDeviceMode, KeyBindings, NvramMode, UiShortcuts};
use crate::controller_config::{Action as ControllerConfigAction, Hit as ControllerConfigHit, State as ControllerConfigState, Target as ControllerConfigTarget};
#[cfg(target_os = "windows")]
use opencdi::gui;
use opencdi::machine::Machine;
use opencdi::mcd212::{SCREEN_WIDTH, SCREEN_HEIGHT, VideoMode};
mod library;
mod launchbox;
mod controller_config;

#[cfg(target_os = "windows")]
#[link(name = "winmm")]
unsafe extern "system" {
    fn timeBeginPeriod(uPeriod: u32) -> u32;
    fn timeEndPeriod(uPeriod: u32) -> u32;
}

#[cfg(target_os = "windows")]
#[link(name = "user32")]
unsafe extern "system" {
    fn MessageBoxW(hwnd: *mut std::ffi::c_void, lptext: *const u16, lpcaption: *const u16, utype: u32) -> i32;
}

#[cfg(target_os = "windows")]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn AttachConsole(process_id: u32) -> i32;
    fn AllocConsole() -> i32;
    fn GetStdHandle(id: u32) -> *mut std::ffi::c_void;
    fn SetStdHandle(id: u32, handle: *mut std::ffi::c_void) -> i32;
    fn GetFileType(handle: *mut std::ffi::c_void) -> u32;
    fn CreateFileW(name: *const u16, access: u32, share: u32, security: *mut std::ffi::c_void, disposition: u32, flags: u32, template: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    fn LoadLibraryW(name: *const u16) -> *mut std::ffi::c_void;
    fn GetProcAddress(module: *mut std::ffi::c_void, name: *const u8) -> *mut std::ffi::c_void;
}

// XInputGetState omits the Xbox Guide button. The XInputGetStateEx export
// (ordinal 100) reports Guide/Home for applications that need the system button.
#[cfg(windows)]
fn xbox_guide_pressed() -> bool {
    use std::sync::OnceLock;
    #[repr(C)] struct Gamepad { buttons: u16, _lt: u8, _rt: u8, _lx: i16, _ly: i16, _rx: i16, _ry: i16 }
    #[repr(C)] struct State { _packet: u32, gamepad: Gamepad }
    type GetStateEx = unsafe extern "system" fn(u32, *mut State) -> u32;
    static GET_STATE: OnceLock<Option<GetStateEx>> = OnceLock::new();
    let get_state = *GET_STATE.get_or_init(|| unsafe {
        for dll in ["xinput1_4.dll", "xinput1_3.dll", "xinput9_1_0.dll"] {
            let wide: Vec<u16> = dll.encode_utf16().chain(std::iter::once(0)).collect();
            let module = LoadLibraryW(wide.as_ptr());
            if module.is_null() { continue; }
            let proc = GetProcAddress(module, 100usize as *const u8);
            if !proc.is_null() { return Some(std::mem::transmute::<*mut std::ffi::c_void, GetStateEx>(proc)); }
        }
        None
    });
    let Some(get_state) = get_state else { return false; };
    (0..4).any(|index| { let mut state = State { _packet: 0, gamepad: Gamepad { buttons: 0, _lt: 0, _rt: 0, _lx: 0, _ly: 0, _rx: 0, _ry: 0 } }; unsafe { get_state(index, &mut state) == 0 && state.gamepad.buttons & 0x0400 != 0 } })
}

#[cfg(not(windows))]
fn xbox_guide_pressed() -> bool { false }

// --console: the parent's console (cmd, PowerShell) or a new one, so a GUI-subsystem release build can show its log
#[cfg(target_os = "windows")]
fn attach_console() {
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
    const STD_ERROR_HANDLE: u32 = -12i32 as u32;
    const INVALID_HANDLE_VALUE: *mut std::ffi::c_void = -1isize as *mut std::ffi::c_void;
    unsafe {
        let usable = |h: *mut std::ffi::c_void| !h.is_null() && h != INVALID_HANDLE_VALUE && GetFileType(h) != 0;
        let saved = [GetStdHandle(STD_OUTPUT_HANDLE), GetStdHandle(STD_ERROR_HANDLE)];
        // Output already goes to a file, a pipe or (under WINE) the Unix terminal
        if saved.iter().all(|&h| usable(h)) {
            return;
        }
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 && AllocConsole() == 0 {
            return;
        }
        // Redirected handles stay; the others are pointed at the console
        let conout: Vec<u16> = "CONOUT$".encode_utf16().chain(std::iter::once(0)).collect();
        for (id, h) in [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE].into_iter().zip(saved) {
            if usable(h) {
                SetStdHandle(id, h);
                continue;
            }
            let con = CreateFileW(conout.as_ptr(), 0x8000_0000 | 0x4000_0000, 0x1 | 0x2, std::ptr::null_mut(), 3, 0, std::ptr::null_mut());
            if con != INVALID_HANDLE_VALUE {
                SetStdHandle(id, con);
            }
        }
    }
}

const USAGE: &str = "\
Usage: opencdi [options] [disc image (cue, chd, iso, bin)]

  --rom <path>        System BIOS (default: search %APPDATA%/OpenCDI/bios and the program directory)
  --frames <n>        Quit after n emulated frames
  --mouse             Start in mouse mode (host mouse drives the CD-i pointer)
  --recent [n]        List the recent disc images, or boot recent image number n
  --no-host-input     Ignore host keyboard, mouse and gamepads; never grab the cursor
  --bios, --standalone  Boot the BIOS shell without a disc
  --dvc <path>        Digital Video Cartridge ROM (default: search %APPDATA%/OpenCDI/bios and the program directory)
  --no-dvc, --nodvc   Boot without the Digital Video Cartridge
  --slave-debug       Log slave (mouse channel) bytes
  --nvram <path>      Console NVRAM save file (default: %APPDATA%/OpenCDI/nvram_mk48t08.bin)
  --run-in-menu       Windows: keep emulating while a menu or dialog is open, or the window is moved (default: paused)
  --console           Windows: show the log in the parent console or a new console window
  --help, -h          Show this text

Settings and BIOS files use %APPDATA%/OpenCDI by default. Create Portable/ next to opencdi.exe to keep app data beside the program.
";


fn savestate_path_for(game_name: &str, slot: usize) -> PathBuf {
    let safe_name: String = game_name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' || c == '(' || c == ')' { c } else { '_' })
        .collect();
    let trimmed = safe_name.trim();
    let base = if trimmed.is_empty() { "cdi_bios" } else { trimmed };
    let dir = migrate_app_data_dir("savestates");
    let _ = std::fs::create_dir_all(&dir);
    dir.join(format!("{}_slot{}.st", base, slot))
}

fn default_nvram_save_path() -> PathBuf {
    let target = app_data_file("nvram_mk48t08.bin");
    if target.is_file() { return target; }
    if let Some(old) = find_nvram_file() {
        if old != target && std::fs::copy(&old, &target).is_ok() { return target; }
        return old;
    }
    target
}

fn per_disc_nvram_save_path(global_path: &Path, disc_path: &Path) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let identity = std::fs::canonicalize(disc_path).unwrap_or_else(|_| disc_path.to_path_buf());
    let identity_text = identity.to_string_lossy().to_ascii_lowercase();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    identity_text.hash(&mut hasher);
    let mut stem = disc_path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "disc".into());
    stem = stem.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    if stem.is_empty() { stem = "disc".into(); }
    let base = global_path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    let per_disc_dir = if base == app_data_dir() { migrate_app_data_dir("nvram_per_disc") } else { base.join("nvram_per_disc") };
    per_disc_dir.join(format!("{}_{}.bin", stem, hasher.finish()))
}

fn find_file(relative_subpath: &str) -> Option<PathBuf> {
    let sub = Path::new(relative_subpath);
    let leaf_name = sub.file_name().and_then(|s| s.to_str()).unwrap_or(relative_subpath);

    let mut candidate_bases: Vec<PathBuf> = vec![
        app_data_dir(),
        PathBuf::from("."),
        PathBuf::from(".."),
        PathBuf::from("file utili"),
        PathBuf::from("..").join("file utili"),
    ];

    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            candidate_bases.push(exe_dir.to_path_buf());
            candidate_bases.push(exe_dir.join(".."));
            candidate_bases.push(exe_dir.join("file utili"));
            candidate_bases.push(exe_dir.join("..").join("file utili"));
        }
    }

    for base in &candidate_bases {
        // 1. Try exact relative subpath (e.g., bios/cdi220b.rom)
        let exact = base.join(relative_subpath);
        if exact.is_file() {
            return Some(exact);
        }
        // 2. Try direct leaf filename (e.g., cdi220b.rom alongside executable)
        let leaf = base.join(leaf_name);
        if leaf.is_file() {
            return Some(leaf);
        }
    }
    None
}

fn find_rom_file(relative_subpath: &str) -> Option<PathBuf> {
    let data_dir = app_data_dir();
    let local = data_dir.join(relative_subpath);
    if local.is_file() { return Some(local); }
    #[cfg(windows)]
    if let Some(appdata) = std::env::var_os("APPDATA") {
        let previous = PathBuf::from(appdata).join("OpenCDI");
        if previous != data_dir {
            let sub = Path::new(relative_subpath);
            let exact = previous.join(sub);
            if exact.is_file() { return Some(exact); }
            if let Some(leaf) = sub.file_name() {
                let direct = previous.join(leaf);
                if direct.is_file() { return Some(direct); }
            }
        }
    }
    find_file(relative_subpath).or_else(|| {
        #[cfg(windows)]
        {
            let root = PathBuf::from(std::env::var_os("APPDATA")?).join("OpenCDI");
            let sub = Path::new(relative_subpath);
            let exact = root.join(sub);
            if exact.is_file() { return Some(exact); }
            let leaf = root.join(sub.file_name()?);
            if leaf.is_file() { return Some(leaf); }
        }
        None
    })
}

fn find_bios_rom() -> Option<PathBuf> {
    const BIOS_CANDIDATES: &[&str] = &[
        "bios/cdi220b.rom",
        "cdi220b.rom",
        "bios/cdi220.rom",
        "cdi220.rom",
        "bios/cdi200.rom",
        "cdi200.rom",
        "bios/cdi.rom",
        "cdi.rom",
        "bios/cdi490a.rom",
        "cdi490a.rom",
    ];
    for &name in BIOS_CANDIDATES {
        if let Some(p) = find_rom_file(name) {
            return Some(store_rom_in_app_data(p, name));
        }
    }
    None
}

fn find_dvc_rom() -> Option<PathBuf> {
    const DVC_CANDIDATES: &[&str] = &[
        "dvc/vmpega.rom",
        "bios/vmpega.rom",
        "vmpega.rom",
        "dvc/impega.rom",
        "bios/impega.rom",
        "impega.rom",
    ];
    for &name in DVC_CANDIDATES {
        if let Some(p) = find_rom_file(name) {
            return Some(store_rom_in_app_data(p, name));
        }
    }
    None
}

fn store_rom_in_app_data(found: PathBuf, relative_name: &str) -> PathBuf {
    let data_dir = app_data_dir();
    if found.starts_with(&data_dir) { return found; }
    let target = data_dir.join(relative_name);
    if target.is_file() { return target; }
    if let Some(parent) = target.parent() { let _ = std::fs::create_dir_all(parent); }
    if std::fs::copy(&found, &target).is_ok() { target } else { found }
}

fn find_nvram_file() -> Option<PathBuf> {
    const NVRAM_CANDIDATES: &[&str] = &[
        "bios/nvram_mk48t08.bin",
        "nvram_mk48t08.bin",
        "bios/nvram.bin",
        "nvram.bin",
    ];
    for &name in NVRAM_CANDIDATES {
        if let Some(p) = find_file(name) {
            return Some(p);
        }
    }
    None
}

fn save_screenshot(frame_count: u64, framebuffer: &[u32]) {
    let _ = std::fs::create_dir_all("screenshots_hotel_mario");
    let filename = format!("screenshots_hotel_mario/screenshot_frame_{}.bmp", frame_count);
    let width = SCREEN_WIDTH as i32;
    let height = SCREEN_HEIGHT as i32;
    let file_size = 54 + (width * height * 4) as u32;
    let mut bmp = Vec::with_capacity(file_size as usize);
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&file_size.to_le_bytes());
    bmp.extend_from_slice(&[0, 0, 0, 0]);
    bmp.extend_from_slice(&54u32.to_le_bytes());
    bmp.extend_from_slice(&40u32.to_le_bytes());
    bmp.extend_from_slice(&width.to_le_bytes());
    bmp.extend_from_slice(&(-height).to_le_bytes());
    bmp.extend_from_slice(&1u16.to_le_bytes());
    bmp.extend_from_slice(&32u16.to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());
    bmp.extend_from_slice(&((width * height * 4) as u32).to_le_bytes());
    bmp.extend_from_slice(&2835u32.to_le_bytes());
    bmp.extend_from_slice(&2835u32.to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());
    for &pixel in framebuffer {
        bmp.push((pixel & 0xFF) as u8);
        bmp.push(((pixel >> 8) & 0xFF) as u8);
        bmp.push(((pixel >> 16) & 0xFF) as u8);
        bmp.push(0xFF);
    }
    if let Ok(_) = std::fs::write(&filename, &bmp) {
        info!("Screenshot salvato in {}", filename);
    }
}

// F8: binary PPM (no image crate needed) into screenshots/ next to the binary
fn save_screenshot_ppm(frame_count: u64, framebuffer: &[u32]) {
    let dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("screenshots")))
        .unwrap_or_else(|| PathBuf::from("screenshots"));
    if let Err(e) = std::fs::create_dir_all(&dir) {
        error!("Cannot create screenshot directory {:?}: {}", dir, e);
        return;
    }
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let path = dir.join(format!("opencdi_{}_frame{}.ppm", stamp, frame_count));
    let mut ppm = format!("P6\n{} {}\n255\n", SCREEN_WIDTH, SCREEN_HEIGHT).into_bytes();
    ppm.reserve(SCREEN_WIDTH * SCREEN_HEIGHT * 3);
    for &pixel in &framebuffer[..SCREEN_WIDTH * SCREEN_HEIGHT] {
        ppm.extend_from_slice(&[(pixel >> 16) as u8, (pixel >> 8) as u8, pixel as u8]);
    }
    match std::fs::write(&path, &ppm) {
        Ok(_) => info!("Screenshot saved to {:?}", path),
        Err(e) => error!("Cannot write screenshot {:?}: {}", path, e),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let library_mode = args.is_empty();
    let has_flag = |f: &str| args.iter().any(|a| a == f);
    let help = has_flag("--help") || has_flag("-h");
    #[cfg(target_os = "windows")]
    if help || has_flag("--console") || has_flag("--recent") {
        attach_console();
    }
    if help {
        print!("{}", USAGE);
        return Ok(());
    }
    // --recent alone lists the recent disc images; --recent <n> boots number n
    let recent_pick: Option<usize> = args.iter().position(|a| a == "--recent").and_then(|i| args.get(i + 1)).and_then(|s| s.parse().ok());
    if has_flag("--recent") && recent_pick.is_none() {
        let list = opencdi::frontend::recent_images();
        if list.is_empty() {
            println!("No recent disc images");
        }
        for (i, p) in list.iter().enumerate() {
            println!("{:2}  {}", i + 1, p.display());
        }
        return Ok(());
    }
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    #[cfg(target_os = "windows")]
    unsafe {
        timeBeginPeriod(1);
    }

    info!("=== OpenCDi - Philips CD-i Emulator ===");

    let flag_value = |f: &str| args.iter().position(|a| a == f).and_then(|i| args.get(i + 1)).cloned();
    let rom_arg = flag_value("--rom").map(PathBuf::from);
    let max_frames: Option<u64> = flag_value("--frames").and_then(|s| s.parse().ok());

    let mut m = Machine::new();
    m.bus.slave.debug_log = has_flag("--slave-debug");
    if m.bus.slave.debug_log {
        info!("Debug Slave XBUS abilitato (--slave-debug): log byte CH0 (mouse)");
    }

    // 1. Load BIOS ROM
    let bios_path = match rom_arg.or_else(find_bios_rom) {
        Some(path) => path,
        None => {
            let data_dir = app_data_dir();
            let error_msg = format!(
                "OpenCDi - Required System BIOS Missing\n\
                ======================================\n\n\
                The emulator cannot start because no Philips CD-i system BIOS ROM was found.\n\n\
                Recommended File:\n\
                  • cdi220b.rom (512 KB, Philips CD-i 220 European PAL BIOS)\n\
                    SHA-1: 53360A1F21DDAC952E95306CED64186A3FC0B93E\n\
                    MD5:   AC0D468BE366779C9DF633BE98DA250A\n\n\
                Alternative Supported BIOSes:\n\
                  • cdi220.rom, cdi200.rom, cdi490a.rom\n\n\
                Default data folder: {}\n\
                Place the BIOS in its 'bios/' subfolder (or directly in that data folder).\n\
                You may also place the BIOS beside opencdi.exe or pass --rom <path>.\n\n\
                Digital Video Cartridge (DVC) for MPEG / FMV games:\n\
                  • Place 'vmpega.rom' (256 KB) in 'dvc/' or 'bios/' inside the data folder.", data_dir.display());

            error!("{}", error_msg);

            #[cfg(target_os = "windows")]
            unsafe {
                use std::os::windows::ffi::OsStrExt;
                let wide_text: Vec<u16> = std::ffi::OsStr::new(&error_msg)
                    .encode_wide()
                    .chain(std::iter::once(0))
                    .collect();
                let wide_title: Vec<u16> = std::ffi::OsStr::new("OpenCDi - Required BIOS Missing")
                    .encode_wide()
                    .chain(std::iter::once(0))
                    .collect();
                MessageBoxW(
                    std::ptr::null_mut(),
                    wide_text.as_ptr(),
                    wide_title.as_ptr(),
                    0x0000_0000 | 0x0000_0010, // MB_OK | MB_ICONERROR
                );
            }
            std::process::exit(1);
        }
    };
    match m.load_bios(&bios_path) {
        Ok(()) => info!("Loaded system BIOS from {:?}", bios_path),
        Err(e) => {
            error!("Cannot use system BIOS {:?}: {}", bios_path, e);
            eprintln!("Cannot use system BIOS {:?}: {}", bios_path, e);
            std::process::exit(1);
        }
    }

    // 2. Load NVRAM if present
    let nvram_arg = args.iter().position(|a| a == "--nvram").and_then(|i| args.get(i + 1)).map(PathBuf::from);
    let nvram_mode = UiShortcuts::load().nvram_mode;
    let resolved_nvram_path = nvram_arg.unwrap_or_else(default_nvram_save_path);
    let mut current_nvram_path = resolved_nvram_path.clone();
    if resolved_nvram_path.is_file() {
        if let Err(e) = m.bus.nvram.load_from_file(&resolved_nvram_path) {
            log::warn!("Impossibile caricare NVRAM da {:?}: {:?}", resolved_nvram_path, e);
        } else {
            info!("NVRAM caricata da {:?}", resolved_nvram_path);
        }
    }

    // 2b. Load DVC ROM (Digital Video Cartridge) by default if present
    let dvc_disabled = has_flag("--no-dvc") || has_flag("--nodvc");
    let dvc_arg = flag_value("--dvc").map(PathBuf::from);
    if !dvc_disabled {
        if let Some(dvc_path) = dvc_arg.clone().or_else(find_dvc_rom) {
            if let Ok(_) = m.load_dvc_rom(&dvc_path) {
                info!("Loaded Digital Video Cartridge (DVC) ROM from {:?}", dvc_path);
            }
        } else {
            info!("No DVC ROM (vmpega.rom) found: booting in Base CD-i mode (MPEG video disabled).");
        }
    } else {
        info!("Forced boot without DVC (--no-dvc): Base CD-i mode");
    }

    // 3. Optional Game Disc (from command line argument, or None for standalone BIOS)
    let mut current_game_name = "No disc inserted (Standalone BIOS)".to_string();
    // Disc = first positional argument (skipping flag values); --bios / --standalone force no disc
    let standalone = has_flag("--bios") || has_flag("--standalone");
    let mut disc_arg = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--rom" || a == "--frames" || a == "--dvc" || (a == "--recent" && recent_pick.is_some()) {
            it.next();
        } else if !a.starts_with("--") {
            disc_arg = Some(a.clone());
            break;
        }
    }
    if let Some(n) = recent_pick {
        disc_arg = opencdi::frontend::recent_images().get(n.wrapping_sub(1)).map(|p| p.to_string_lossy().to_string());
        if disc_arg.is_none() {
            log::warn!("No recent disc image number {} (see --recent)", n);
        }
    }
    let initial_disc = disc_arg.filter(|_| !standalone && !library_mode).and_then(|a| {
        find_file(&a).or_else(|| {
            let p = PathBuf::from(&a);
            if p.exists() || opencdi::cdrom::parse_drive_letter(&p).is_some() { Some(p) } else { None }
        })
    });

    let mut current_disc_path: Option<PathBuf> = None;
    if let Some(p) = initial_disc {
        match m.bus.load_cdrom(&p) {
            Ok(_) => {
                current_disc_path = Some(p.clone());
                if nvram_mode == NvramMode::PerDisc {
                    current_nvram_path = per_disc_nvram_save_path(&resolved_nvram_path, &p);
                    if let Err(e) = m.bus.nvram.load_or_initialize(&current_nvram_path) {
                        log::warn!("Cannot load per-disc NVRAM {:?}: {}", current_nvram_path, e);
                    }
                }
                opencdi::frontend::add_recent_image(&p);
                current_game_name = opencdi::frontend::disc_display_name(&p);
                info!("Loaded game disc at startup: {}", current_game_name);

                let name_upper = current_game_name.to_uppercase();
                let is_ntsc = (name_upper.contains("(USA)") || name_upper.contains("(JAPAN)") || name_upper.contains("NTSC"))
                    && m.bus.cdrom.as_ref().map_or(true, |cd| cd.disc_type != 0x20);
                if is_ntsc {
                    info!("Detected NTSC disc (USA/Japan): auto-configured NTSC 60 Hz video mode");
                    m.bus.set_video_mode(VideoMode::Ntsc);
                } else {
                    info!("Console booted in standard PAL 50 Hz mode (CD-i 220 BIOS)");
                    m.bus.set_video_mode(VideoMode::Pal);
                }
            }
            Err(e) => error!("Error loading game disc from {:?}: {:?}", p, e),
        }
    } else {
        info!("Starting console in Standalone BIOS mode (no disc inserted) - PAL 50 Hz");
        m.bus.set_video_mode(VideoMode::Pal);
    }

    // 4. Initialize 68070 CPU Core
    m.reset();
    info!("CPU SCC68070 reset - PC: 0x{:08X}", m.cpu.pc);

    // Keep user artwork beside the rest of app data and bring forward artwork
    // from older versions before defaults create the new storage folders.
    let _ = migrate_app_data_dir("Artwork");

    // 5. Initialize Controls Configuration
    let mut bindings = KeyBindings::new();
    let global_bindings = bindings.clone();
    let active_disc_config_path = current_disc_path.as_deref().map(disc_config_path);
    if let Some(path) = &active_disc_config_path { bindings.load_disc_config(path); bindings.save_disc_config(path); }
    let skip_shell = bindings.ui_shortcuts.skip_shell;
    let disc_paths = bindings.ui_shortcuts.disc_paths.iter().map(|p| resolve_config_path(p)).collect::<Vec<_>>();
    let covers_path = resolve_config_path(&bindings.ui_shortcuts.covers_path);
    let cd_art_path = resolve_config_path(&bindings.ui_shortcuts.cd_art_path);
    let bezels_path = resolve_config_path(&bindings.ui_shortcuts.bezels_path);
    let managed_artwork_root = app_data_dir().join("Artwork");
    let managed_covers_path = managed_artwork_root.join("Covers");
    let managed_cd_path = managed_artwork_root.join("CD");
    let mut scan_roots = library_roots();
    scan_roots.extend(disc_paths.iter().cloned());
    let library_items = if library_mode { library::scan(&scan_roots, &covers_path, &cd_art_path, &managed_covers_path, &managed_cd_path) } else { Vec::new() };
    let boot_disc_art = current_disc_path.as_deref().and_then(|p| library::load_disc_artwork(p, &cd_art_path).or_else(|| library::load_disc_artwork(p, &managed_cd_path)));
    let input_device_mode = if has_flag("--mouse") { InputDeviceMode::Mouse } else { bindings.ui_shortcuts.input_device };

    // 6. Initialize Host Audio Playback
    let host_audio = HostAudioOutput::try_start(
        m.bus.cdic.audio.sample_queue.clone(),
        m.bus.cdic.audio.sfx_queue.clone(),
        m.bus.cdic.audio.dvc_queue.clone(),
        m.bus.cdic.audio.source_rate.clone(),
        m.bus.cdic.audio.dvc_rate.clone(),
    );

    // 7. Initialize USB Gamepad subsystem (Gilrs / XInput / DirectInput)
    let gilrs = gilrs::Gilrs::new().ok();
    if gilrs.is_some() {
        info!("USB Gamepad subsystem active (Xbox / PlayStation / Generic USB)");
    }

    m.bus.emulate_wait_states = true; // Authentic hardware wait-states (15.5 MHz)
    let (pad_x, pad_y) = if m.bus.cdrom.is_some() { (575, 335) } else { (384, 280) };
    let prev_vsr = (m.bus.mcd212.get_vsr::<0>(), m.bus.mcd212.get_vsr::<1>());
    let now = Instant::now();
    let mut app = App {
        m,
        bindings,
        global_bindings,
        active_disc_config_path,
        controller_config: None,
        controller_art: controller_config::load_controller_art(),
        mouse_art: controller_config::load_mouse_art(),
        controller_config_was_captured: false,
        config_prev_up: false,
        config_prev_down: false,
        config_prev_left: false,
        config_prev_right: false,
        config_prev_confirm: false,
        skip_shell,
        input_device_mode,
        dvc_disabled,
        dvc_arg,
        max_frames,
        current_game_name,
        nvram_path: Some(current_nvram_path),
        global_nvram_path: resolved_nvram_path,
        current_disc_path,
        nvram_mode,
        window: None,
        surface: None,
        image_rect: (0, 0, SCREEN_WIDTH, SCREEN_HEIGHT),
        windowed: None,
        display_mode: DisplayMode::BorderlessFullscreen,
        restore_check: None,
        _host_audio: host_audio,
        gilrs,
        keys: HashSet::new(),
        mods: ModifiersState::empty(),
        focused: false,
        cursor: None,
        rel_pointer: None,
        mouse_left: false,
        mouse_right: false,
        mouse_middle: false,
        prev_middle: false,
        mouse_captured: skip_shell,
        host_input: !has_flag("--no-host-input"),
        run_in_menu: has_flag("--run-in-menu"),
        in_modal_pump: false,
        paused: false,
        fast_forward: false,
        last_present: now,
        occluded: false,
        grab_applied: None,
        frame_count: 0,
        frames_run: 0,
        menu_visible: true,
        prev_menu_pad: false,
        pause_menu_selected: 0,
        prev_pause_pad: false,
        prev_pause_up: false,
        prev_pause_down: false,
        prev_pause_confirm: false,
        toolbar_popup: None,
        exit_requested: false,
        disc_launched: false,
        shell_ready_count: 0,
        pad_x,
        pad_y,
        pad_x_f: pad_x as f32,
        pad_y_f: pad_y as f32,
        prev_key_btn: 0,
        run_at: now,
        next_frame_time: now,
        fps_timer: now,
        telemetry_frames: 0,
        paused_ticks: 0,
        measured_fps: 50.0,
        prev_instructions: 0,
        prev_cd_sectors: 0,
        prev_vsr,
        buffer_flips_interval: 0,
        measured_render_fps: 0.0,
        measured_mips: 0.0,
        measured_cd_rate: 0.0,
        library_mode,
        library_items,
        artwork_fetch_rx: None,
        library_selected: 0,
        disc_paths,
        covers_path,
        cd_art_path,
        bezels_path,
        managed_artwork_root,
        managed_covers_path,
        managed_cd_path,
        boot_disc_art,
        library_click: None,
        library_scroll_accum: 0.0,
        library_size_dragging: false,
        library_prev_left: false,
        library_prev_right: false,
        library_prev_up: false,
        library_prev_down: false,
        library_prev_confirm: false,
    };
    app.start_artwork_lookup(false);

    info!("Emulation loop started. Shortcuts: [F11]/[Alt+Enter] Fullscreen, [F5]/[H] Reset, [P] Pause, [Space] hold Fast-forward, [F8] Screenshot (PPM), [F12] Screenshot (BMP), [M] Mute, [+]/[-] Volume, [Ctrl+I] Insert Disc, [Ctrl+O] Open & Reset, [Middle Click] Capture/Release Mouse, [Esc] Quit");

    let event_loop = EventLoop::new()?;
    event_loop.run_app(&mut app)?;

    #[cfg(target_os = "windows")]
    unsafe {
        timeEndPeriod(1);
    }

    app.m.bus.nvram.flush_if_dirty();
    info!("Emulation stopped. Final PC: 0x{:08X}", app.m.cpu.pc);
    Ok(())
}

// Letterbox rectangle (x, y, w, h) of the CD-i picture in a dst_w x dst_h surface, as minifb's AspectRatioStretch computes it
// Application icon from icon.ico (the 64x64 PNG entry), as on the Windows executable
fn window_icon() -> Option<winit::window::Icon> {
    let ico: &[u8] = include_bytes!("../icon.ico");
    let count = u16::from_le_bytes([*ico.get(4)?, *ico.get(5)?]) as usize;
    let entry = (0..count).map(|i| &ico[6 + i * 16..22 + i * 16]).find(|e| e[0] == 64)?;
    let size = u32::from_le_bytes(entry[8..12].try_into().ok()?) as usize;
    let offset = u32::from_le_bytes(entry[12..16].try_into().ok()?) as usize;
    let mut decoder = png::Decoder::new(std::io::Cursor::new(ico.get(offset..offset + size)?));
    decoder.set_transformations(png::Transformations::normalize_to_color8() | png::Transformations::ALPHA);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    if info.color_type != png::ColorType::Rgba {
        return None;
    }
    buf.truncate(info.buffer_size());
    winit::window::Icon::from_rgba(buf, info.width, info.height).ok()
}

fn library_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(exe) = std::env::current_exe() { if let Some(parent) = exe.parent() { roots.push(parent.to_path_buf()); } }
    if let Ok(cwd) = std::env::current_dir() { if !roots.contains(&cwd) { roots.push(cwd.clone()); } }
    let bases = roots.clone();
    for base in bases { for name in ["games", "discs", "roms", "images"] {
        let candidate = base.join(name); if candidate.is_dir() && !roots.contains(&candidate) { roots.push(candidate); }
    }}
    roots
}

fn resolve_config_path(path: &Path) -> PathBuf {
    if path.is_absolute() { return path.to_path_buf(); }
    std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join(path)))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default().join(path))
}

fn image_rect(dst_w: usize, dst_h: usize) -> (usize, usize, usize, usize) {
    if dst_w == 0 || dst_h == 0 {
        return (0, 0, 0, 0);
    }
    let buffer_aspect = SCREEN_WIDTH as f32 / SCREEN_HEIGHT as f32;
    let win_aspect = dst_w as f32 / dst_h as f32;
    if buffer_aspect > win_aspect {
        let h = ((dst_w as f32 / buffer_aspect) as usize).clamp(1, dst_h);
        (0, (dst_h - h) / 2, dst_w, h)
    } else {
        let w = ((dst_h as f32 * buffer_aspect) as usize).clamp(1, dst_w);
        ((dst_w - w) / 2, 0, w, dst_h)
    }
}

fn disc_config_path(disc: &Path) -> PathBuf {
    let raw = disc.file_stem().unwrap_or_default().to_string_lossy();
    let safe: String = raw.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    app_data_dir().join("Configs").join(format!("{}.ini", if safe.is_empty() { "Disc" } else { &safe }))
}

// Host cursor position mapped through the letterbox onto the CD-i picture, clamped
fn cursor_to_cdi(pos: PhysicalPosition<f64>, rect: (usize, usize, usize, usize)) -> (i32, i32) {
    let (x0, y0, w, h) = rect;
    if w == 0 || h == 0 {
        return (0, 0);
    }
    let mx = (pos.x - x0 as f64) * SCREEN_WIDTH as f64 / w as f64;
    let my = (pos.y - y0 as f64) * SCREEN_HEIGHT as f64 / h as f64;
    ((mx.round() as i32).clamp(0, SCREEN_WIDTH as i32 - 1), (my.round() as i32).clamp(0, SCREEN_HEIGHT as i32 - 1))
}

// Nearest-neighbour stretch with black bars, same 10-bit fixed-point stepping as minifb's scaler
fn blit_aspect(dst: &mut [u32], dst_w: usize, dst_h: usize, src: &[u32], rect: (usize, usize, usize, usize)) {
    dst.fill(0);
    let (x0, y0, w, h) = rect;
    if w == 0 || h == 0 {
        return;
    }
    let step_x = ((SCREEN_WIDTH as f32 / w as f32) * 1024.0) as usize;
    let step_y = ((SCREEN_HEIGHT as f32 / h as f32) * 1024.0) as usize;
    let mut fixed_y = 0usize;
    for row in dst.chunks_exact_mut(dst_w).skip(y0).take(h.min(dst_h - y0)) {
        let src_row = &src[(fixed_y >> 10).min(SCREEN_HEIGHT - 1) * SCREEN_WIDTH..][..SCREEN_WIDTH];
        let mut fixed_x = 0usize;
        for px in &mut row[x0..x0 + w] {
            *px = src_row[(fixed_x >> 10).min(SCREEN_WIDTH - 1)];
            fixed_x += step_x;
        }
        fixed_y += step_y;
    }
}

fn blit_smooth_aspect(dst: &mut [u32], dst_w: usize, dst_h: usize, src: &[u32], rect: (usize, usize, usize, usize)) {
    dst.fill(0xFF000000);
    let (x0, y0, w, h) = rect;
    if w == 0 || h == 0 { return; }
    for oy in 0..h.min(dst_h.saturating_sub(y0)) {
        let fy = ((oy as f32 + 0.5) * SCREEN_HEIGHT as f32 / h as f32 - 0.5).clamp(0.0, (SCREEN_HEIGHT - 1) as f32);
        let sy0 = fy.floor() as usize; let sy1 = (sy0 + 1).min(SCREEN_HEIGHT - 1); let ty = fy - sy0 as f32;
        for ox in 0..w.min(dst_w.saturating_sub(x0)) {
            let fx = ((ox as f32 + 0.5) * SCREEN_WIDTH as f32 / w as f32 - 0.5).clamp(0.0, (SCREEN_WIDTH - 1) as f32);
            let sx0 = fx.floor() as usize; let sx1 = (sx0 + 1).min(SCREEN_WIDTH - 1); let tx = fx - sx0 as f32;
            let pixels = [src[sy0 * SCREEN_WIDTH + sx0], src[sy0 * SCREEN_WIDTH + sx1], src[sy1 * SCREEN_WIDTH + sx0], src[sy1 * SCREEN_WIDTH + sx1]];
            let mut out = 0xFF000000u32;
            for shift in [0, 8, 16] {
                let c = |p: u32| ((p >> shift) & 255) as f32;
                let top = c(pixels[0]) * (1.0 - tx) + c(pixels[1]) * tx;
                let bottom = c(pixels[2]) * (1.0 - tx) + c(pixels[3]) * tx;
                out |= (((top * (1.0 - ty) + bottom * ty).round() as u32).min(255)) << shift;
            }
            dst[(y0 + oy) * dst_w + x0 + ox] = out;
        }
    }
}

struct App {
    m: Machine,
    bindings: KeyBindings,
    global_bindings: KeyBindings,
    active_disc_config_path: Option<PathBuf>,
    controller_config: Option<ControllerConfigState>,
    controller_art: Option<library::Artwork>,
    mouse_art: Option<library::Artwork>,
    controller_config_was_captured: bool,
    config_prev_up: bool,
    config_prev_down: bool,
    config_prev_left: bool,
    config_prev_right: bool,
    config_prev_confirm: bool,
    skip_shell: bool,
    input_device_mode: InputDeviceMode,
    dvc_disabled: bool,
    dvc_arg: Option<PathBuf>,
    max_frames: Option<u64>,
    current_game_name: String,
    nvram_path: Option<PathBuf>,
    global_nvram_path: PathBuf,
    current_disc_path: Option<PathBuf>,
    nvram_mode: NvramMode,
    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    image_rect: (usize, usize, usize, usize),
    windowed: Option<(Option<PhysicalPosition<i32>>, PhysicalSize<u32>)>,
    display_mode: DisplayMode,
    restore_check: Option<Instant>,
    _host_audio: Option<HostAudioOutput>,
    gilrs: Option<gilrs::Gilrs>,
    keys: HashSet<Key>,
    mods: ModifiersState,
    focused: bool,
    cursor: Option<PhysicalPosition<f64>>,
    rel_pointer: Option<(f64, f64)>,
    mouse_left: bool,
    mouse_right: bool,
    mouse_middle: bool,
    prev_middle: bool,
    mouse_captured: bool,
    host_input: bool,
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    run_in_menu: bool,
    in_modal_pump: bool,
    occluded: bool,
    paused: bool,
    fast_forward: bool,
    last_present: Instant,
    grab_applied: Option<bool>,
    frame_count: u64,
    frames_run: u64,
    menu_visible: bool,
    prev_menu_pad: bool,
    pause_menu_selected: usize,
    prev_pause_pad: bool,
    prev_pause_up: bool,
    prev_pause_down: bool,
    prev_pause_confirm: bool,
    toolbar_popup: Option<library::ToolbarPopup>,
    exit_requested: bool,
    disc_launched: bool,
    shell_ready_count: u32,
    pad_x: i32,
    pad_y: i32,
    pad_x_f: f32,
    pad_y_f: f32,
    prev_key_btn: u8,
    run_at: Instant,
    next_frame_time: Instant,
    fps_timer: Instant,
    telemetry_frames: u32,
    paused_ticks: u32,
    measured_fps: f64,
    prev_instructions: u64,
    prev_cd_sectors: u32,
    prev_vsr: (u32, u32),
    buffer_flips_interval: u32,
    measured_render_fps: f64,
    measured_mips: f64,
    measured_cd_rate: f64,
    library_mode: bool,
    library_items: Vec<library::Item>,
    artwork_fetch_rx: Option<std::sync::mpsc::Receiver<usize>>,
    library_selected: usize,
    disc_paths: Vec<PathBuf>,
    covers_path: PathBuf,
    cd_art_path: PathBuf,
    bezels_path: PathBuf,
    managed_artwork_root: PathBuf,
    managed_covers_path: PathBuf,
    managed_cd_path: PathBuf,
    boot_disc_art: Option<library::Artwork>,
    library_click: Option<(usize, Instant)>,
    library_scroll_accum: f64,
    library_size_dragging: bool,
    library_prev_left: bool,
    library_prev_right: bool,
    library_prev_up: bool,
    library_prev_down: bool,
    library_prev_confirm: bool,
}

impl App {
    fn toggle_menu(&mut self) {
        if !self.library_mode || self.controller_config.is_some() { return; }
        self.menu_visible = !self.menu_visible;
        if let Some(window) = &self.window { window.request_redraw(); }
    }

    fn pause_is_available(&self) -> bool {
        !self.library_mode && self.current_disc_path.is_some() && self.disc_launched
    }

    fn toggle_pause_menu(&mut self) {
        if !self.pause_is_available() && !self.paused { return; }
        self.paused = !self.paused;
        if self.paused { self.pause_menu_selected = 0; }
        self.prev_pause_confirm = false;
        self.reset_telemetry();
        self.refresh_title();
        if let Some(window) = &self.window { window.request_redraw(); }
    }

    fn move_pause_selection(&mut self, delta: isize) {
        self.pause_menu_selected = (self.pause_menu_selected as isize + delta).rem_euclid(5) as usize;
        if let Some(window) = &self.window { window.request_redraw(); }
    }

    fn choose_pause_action(&mut self) {
        match self.pause_menu_selected {
            0 => self.toggle_pause_menu(),
            1 => { push_action(GuiAction::QuickSaveState); self.paused = false; },
            2 => { push_action(GuiAction::QuickLoadState); self.paused = false; },
            3 => {
                self.controller_config_was_captured = self.mouse_captured;
                self.controller_config = Some(controller_config::State::default());
                self.mouse_captured = false;
                self.m.bus.slave.update_mouse(self.pad_x, self.pad_y, 0);
                self.prev_key_btn = 0;
                self.apply_grab();
            }
            4 => self.stop_to_library(),
            _ => {}
        }
        self.reset_telemetry();
        if let Some(window) = &self.window { window.request_redraw(); }
    }

    fn stop_to_library(&mut self) {
        self.save_active_disc_settings();
        self.paused = false;
        self.toolbar_popup = None;
        self.library_mode = true;
        // The South/A button used to confirm this action may still be held on the
        // next library tick. Consume that edge so it cannot relaunch the selected disc.
        self.library_prev_confirm = true;
        self.mouse_captured = false;
        self.m.bus.slave.set_disc_inserted(false);
        self.m.bus.cdrom = None;
        self.current_disc_path = None;
        self.active_disc_config_path = None;
        self.bindings = self.global_bindings.clone();
        self.input_device_mode = self.bindings.ui_shortcuts.input_device;
        self.activate_nvram_for_selection();
        self.disc_launched = false;
        self.shell_ready_count = 0;
        self.current_game_name = "Disc Library".to_string();
        self.pad_x = 384; self.pad_y = 280; self.pad_x_f = 384.0; self.pad_y_f = 280.0;
        self.grab_applied = None;
        self.apply_grab();
        self.reset_telemetry();
        self.refresh_title();
        if let Some(window) = &self.window { window.request_redraw(); }
    }

    fn update_pause_input(&mut self) {
        let mut guide = self.bindings.ui_shortcuts.pause_pad.contains(&gilrs::Button::Mode) && xbox_guide_pressed();
        let mut exit = false;
        let mut up = false; let mut down = false; let mut confirm = false;
        if let Some(g) = self.gilrs.as_mut() {
            while g.next_event().is_some() {}
            for (_, pad) in g.gamepads().filter(|_| self.host_input) {
                if !pad.is_connected() { continue; }
                let pause_combo = &self.bindings.ui_shortcuts.pause_pad;
                guide |= !pause_combo.is_empty() && pause_combo.iter().all(|button| pad.is_pressed(*button));
                let exit_chord = &self.bindings.ui_shortcuts.exit_pad;
                exit |= !exit_chord.is_empty() && exit_chord.iter().all(|button| pad.is_pressed(*button));
                up |= pad.is_pressed(gilrs::Button::DPadUp) || pad.value(gilrs::Axis::LeftStickY) > 0.35;
                down |= pad.is_pressed(gilrs::Button::DPadDown) || pad.value(gilrs::Axis::LeftStickY) < -0.35;
                confirm |= pad.is_pressed(gilrs::Button::South);
            }
        }
        if guide && !self.prev_pause_pad { self.toggle_pause_menu(); }
        self.prev_pause_pad = guide;
        if exit { self.exit_requested = true; }
        if !self.paused { return; }
        if up && !self.prev_pause_up { self.move_pause_selection(-1); }
        if down && !self.prev_pause_down { self.move_pause_selection(1); }
        if confirm && !self.prev_pause_confirm { self.choose_pause_action(); }
        self.prev_pause_up = up; self.prev_pause_down = down; self.prev_pause_confirm = confirm;
    }

    fn handle_toolbar_action(&mut self, action: library::ToolbarAction) {
        use library::ToolbarAction as T;
        match action {
            T::OpenDisc => self.open_dialog_action(true),
            T::Controller => push_action(GuiAction::OpenControllerConfig),
            T::Paths => self.toolbar_popup = if self.toolbar_popup == Some(library::ToolbarPopup::Paths) { None } else { Some(library::ToolbarPopup::Paths) },
            T::Refresh => { self.toolbar_popup = None; push_action(GuiAction::RescanLibrary); }
            T::Display => self.toolbar_popup = if self.toolbar_popup == Some(library::ToolbarPopup::Display) { None } else { Some(library::ToolbarPopup::Display) },
            T::Exit => { self.toolbar_popup = None; push_action(GuiAction::Exit); }
            T::Borderless => { self.toolbar_popup = None; self.set_display_mode(DisplayMode::BorderlessFullscreen); }
            T::Decorated => { self.toolbar_popup = None; self.set_display_mode(DisplayMode::DecoratedFullscreen); }
            T::Windowed => { self.toolbar_popup = None; self.set_display_mode(DisplayMode::Windowed); }
            T::DownloadArtwork => { self.toolbar_popup = None; self.start_artwork_lookup(true); }
            T::AddDiscFolder | T::SetCovers | T::SetCd | T::SetBezels => {
                self.toolbar_popup = None;
                #[cfg(target_os = "windows")]
                {
                    let hwnd = self.hwnd();
                    let title = match action {
                        T::AddDiscFolder => "Choose a folder to scan for disc images",
                        T::SetCovers => "Choose the Covers artwork folder",
                        T::SetCd => "Choose the CD artwork folder",
                        _ => "Choose the bezels artwork folder",
                    };
                    if let Some(path) = gui::modal(hwnd, || gui::select_folder_dialog(hwnd, title)) {
                        match action {
                            T::AddDiscFolder => push_action(GuiAction::AddDiscPath(path)),
                            T::SetCovers => push_action(GuiAction::SetArtworkPath(ArtworkPathKind::Covers, path)),
                            T::SetCd => push_action(GuiAction::SetArtworkPath(ArtworkPathKind::Cd, path)),
                            T::SetBezels => push_action(GuiAction::SetArtworkPath(ArtworkPathKind::Bezels, path)),
                            _ => {}
                        }
                    }
                }
            }
        }
        if let Some(window) = &self.window { window.request_redraw(); }
    }

    fn launch_library_selection(&mut self) {
        if let Some(item) = self.library_items.get(self.library_selected) {
            self.boot_disc_art = item.disc.clone();
            push_action(GuiAction::OpenDisc(item.path.clone()));
        }
    }

    fn refresh_library(&mut self) {
        let mut roots = library_roots();
        roots.extend(self.disc_paths.iter().cloned());
        self.library_items = library::scan(&roots, &self.covers_path, &self.cd_art_path, &self.managed_covers_path, &self.managed_cd_path);
        if self.library_selected >= self.library_items.len() { self.library_selected = 0; }
        self.start_artwork_lookup(false);
    }

    fn start_artwork_lookup(&mut self, download_all: bool) {
        if self.artwork_fetch_rx.is_some() { return; }
        let targets: Vec<_> = self.library_items.iter().filter(|item| download_all || item.cover.is_none() || item.disc.is_none())
            .map(|item| launchbox::FetchTarget { title: item.title.clone(), cover_missing: item.cover.is_none(), disc_missing: item.disc.is_none(), download_all }).collect();
        if targets.is_empty() { return; }
        let artwork_root = self.managed_artwork_root.clone();
        let cache = artwork_root.join("Cache");
        let (tx, rx) = std::sync::mpsc::channel();
        self.artwork_fetch_rx = Some(rx);
        let result = std::thread::Builder::new().name("launchbox-artwork".into()).spawn(move || {
            let count = launchbox::fetch_missing_artwork(&targets, &artwork_root, &cache);
            let _ = tx.send(count);
        });
        if result.is_err() { self.artwork_fetch_rx = None; }
    }

    fn set_artwork_size(&mut self, value: u8) {
        let value = value.clamp(opencdi::frontend::ART_SIZE_MIN, opencdi::frontend::ART_SIZE_MAX);
        if self.bindings.ui_shortcuts.artwork_size != value {
            self.bindings.ui_shortcuts.artwork_size = value;
            if let Some(window) = &self.window { window.request_redraw(); }
        }
    }

    fn persist_paths(&mut self) {
        self.bindings.ui_shortcuts.disc_paths = self.disc_paths.clone();
        self.bindings.ui_shortcuts.covers_path = self.covers_path.clone();
        self.bindings.ui_shortcuts.cd_art_path = self.cd_art_path.clone();
        self.bindings.ui_shortcuts.bezels_path = self.bezels_path.clone();
        self.bindings.save_to_ini();
    }

    fn save_input_settings(&mut self) {
        if let Some(path) = &self.active_disc_config_path {
            self.bindings.ui_shortcuts.input_device = self.input_device_mode;
            self.bindings.save_disc_config(path);
        } else {
            self.bindings.ui_shortcuts.input_device = self.input_device_mode;
            self.bindings.save_to_ini();
            self.global_bindings = self.bindings.clone();
        }
    }

    fn save_active_disc_settings(&self) {
        if let Some(path) = &self.active_disc_config_path { self.bindings.save_disc_config(path); }
    }

    fn activate_disc_settings(&mut self, path: &Path) {
        self.bindings = self.global_bindings.clone();
        let config = disc_config_path(path);
        self.bindings.load_disc_config(&config);
        self.bindings.save_disc_config(&config);
        self.input_device_mode = self.bindings.ui_shortcuts.input_device;
        self.active_disc_config_path = Some(config);
    }

    fn move_library_selection(&mut self, delta: isize) {
        if self.library_items.is_empty() { return; }
        let n = self.library_items.len() as isize;
        self.library_selected = (self.library_selected as isize + delta).rem_euclid(n) as usize;
    }

    fn activate_nvram_for_selection(&mut self) {
        let target = if self.nvram_mode == NvramMode::PerDisc {
            self.current_disc_path.as_deref().map(|p| per_disc_nvram_save_path(&self.global_nvram_path, p))
                .unwrap_or_else(|| self.global_nvram_path.clone())
        } else { self.global_nvram_path.clone() };
        if self.nvram_path.as_ref() == Some(&target) { return; }
        self.m.bus.nvram.flush_if_dirty();
        match self.m.bus.nvram.load_or_initialize(&target) {
            Ok(()) => {
                self.nvram_path = Some(target.clone());
                info!("NVRAM loaded from {:?}", target);
            }
            Err(e) => error!("Unable to activate NVRAM {:?}: {}", target, e),
        }
    }

    fn library_key(&mut self, el: &ActiveEventLoop, code: Key) -> bool {
        if !self.library_mode { return false; }
        match code {
            Key::ArrowLeft => self.move_library_selection(-1),
            Key::ArrowRight => self.move_library_selection(1),
            Key::ArrowUp => self.move_library_selection(-(library::layout(self.bindings.ui_shortcuts.artwork_size).columns as isize)),
            Key::ArrowDown => self.move_library_selection(library::layout(self.bindings.ui_shortcuts.artwork_size).columns as isize),
            Key::Enter | Key::NumpadEnter | Key::Space => self.launch_library_selection(),
            Key::Escape => el.exit(),
            Key::KeyO if self.mods.control_key() => self.open_dialog_action(true),
            _ => return false,
        }
        true
    }
    fn key(&self, k: Key) -> bool {
        self.keys.contains(&k)
    }

    #[cfg(target_os = "windows")]
    fn hwnd(&self) -> *mut std::ffi::c_void {
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        self.window
            .as_ref()
            .and_then(|w| w.window_handle().ok())
            .and_then(|h| match h.as_raw() {
                RawWindowHandle::Win32(h) => Some(h.hwnd.get() as *mut std::ffi::c_void),
                _ => None,
            })
            .unwrap_or(std::ptr::null_mut())
    }

    fn set_display_mode(&mut self, mode: DisplayMode) {
        if self.display_mode == mode { return; }
        let Some(window) = self.window.as_ref() else { self.display_mode = mode; return; };
        let old_mode = self.display_mode;
        if old_mode == DisplayMode::Windowed && mode != DisplayMode::Windowed && !window.is_maximized() {
            self.windowed = Some((window.outer_position().ok(), window.inner_size()));
        }
        window.set_fullscreen(None);
        self.restore_check = None;
        match mode {
            DisplayMode::BorderlessFullscreen => {
                window.set_maximized(false);
                window.set_decorations(false);
                window.set_fullscreen(Some(Fullscreen::Borderless(None)));
            }
            DisplayMode::DecoratedFullscreen => {
                window.set_decorations(true);
                window.set_maximized(true);
            }
            DisplayMode::Windowed => {
                window.set_decorations(true);
                window.set_maximized(false);
                if self.windowed.is_some() {
                    self.restore_windowed();
                    self.restore_check = Some(Instant::now() + Duration::from_millis(300));
                } else {
                    let _ = window.request_inner_size(LogicalSize::new(SCREEN_WIDTH as f64, SCREEN_HEIGHT as f64));
                }
            }
        }
        self.display_mode = mode;
        #[cfg(target_os = "windows")]
        {
            gui::set_menu_visible(self.hwnd(), false);
            gui::update_display_mode_menu(mode);
        }
        info!("Display mode changed to: {:?}", mode);
    }

    fn toggle_fullscreen(&mut self) {
        let next = match self.display_mode {
            DisplayMode::BorderlessFullscreen => DisplayMode::DecoratedFullscreen,
            DisplayMode::DecoratedFullscreen => DisplayMode::Windowed,
            DisplayMode::Windowed => DisplayMode::BorderlessFullscreen,
        };
        self.set_display_mode(next);
    }

    // Puts the window back to the size and position it had before fullscreen
    fn restore_windowed(&self) {
        let (Some(window), Some((pos, size))) = (&self.window, self.windowed) else { return };
        if let Some(pos) = pos.filter(|&p| window.outer_position().ok() != Some(p)) {
            window.set_outer_position(pos);
        }
        if window.inner_size() != size {
            let _ = window.request_inner_size(size);
        }
    }

    fn reset_state(&mut self) {
        self.frame_count = 0;
        self.disc_launched = false;
        self.shell_ready_count = 0;
        self.pad_x = if self.m.bus.cdrom.is_some() { 575 } else { 384 };
        self.pad_y = if self.m.bus.cdrom.is_some() { 335 } else { 280 };
        self.pad_x_f = self.pad_x as f32;
        self.pad_y_f = self.pad_y as f32;
    }

    // Hide and confine the host cursor while captured and focused (Win32 ShowCursor/ClipCursor before)
    fn apply_grab(&mut self) {
        let Some(window) = &self.window else { return };
        let want = self.host_input && self.focused && self.mouse_captured && !self.library_mode && self.controller_config.is_none();
        if self.grab_applied == Some(want) {
            return;
        }
        self.grab_applied = Some(want);
        // While grabbed the host cursor is hidden and pinned (Windows: window centre), so mouse mode follows raw motion
        self.rel_pointer = want.then(|| match self.cursor {
            Some(pos) => {
                let (x, y) = cursor_to_cdi(pos, self.image_rect);
                (x as f64, y as f64)
            }
            None => (self.pad_x as f64, self.pad_y as f64),
        });
        window.set_cursor_visible(!want);
        let mode = if want { CursorGrabMode::Confined } else { CursorGrabMode::None };
        if let Err(e) = window.set_cursor_grab(mode) {
            info!("Cursor grab {:?} not available: {}", mode, e);
        }
    }

    fn add_recent(&self, path: &Path) {
        #[cfg(target_os = "windows")]
        gui::add_recent_image(path);
        #[cfg(not(target_os = "windows"))]
        opencdi::frontend::add_recent_image(path);
    }

    fn open_dialog_action(&self, open_and_reset: bool) {
        #[cfg(target_os = "windows")]
        gui::post_menu_command(self.hwnd(), if open_and_reset { gui::ID_FILE_OPEN } else { gui::ID_FILE_INSERT });
        #[cfg(not(target_os = "windows"))]
        {
            let _ = open_and_reset;
            info!("No file dialog on this platform: pass the disc path on the command line, or --recent <n>");
        }
    }

    // A key bound to a CD-i control in opencdi_controls.ini is not a shortcut
    fn bound(&self, k: Key) -> bool {
        let b = &self.bindings;
        [b.up, b.down, b.left, b.right, b.btn1, b.btn2, b.btn3].contains(&k)
    }

    fn on_key_pressed(&mut self, el: &ActiveEventLoop, code: Key) {
        let mut save_input = false;
        let mut closed_controller_config = false;
        if let Some(state) = self.controller_config.as_mut() {
            match state.key_pressed(code) {
                ControllerConfigAction::AssignedKey(row, key) => {
                    if row == 7 { self.bindings.ui_shortcuts.set_pause_key(key); }
                    else { self.bindings.set_key_binding(row, key); }
                    save_input = true;
                }
                ControllerConfigAction::AssignedPauseKey(key) => { self.bindings.ui_shortcuts.set_pause_key(key); save_input = true; }
                ControllerConfigAction::AssignedExitKey(key) => { self.bindings.ui_shortcuts.set_exit_key(key); save_input = true; }
                ControllerConfigAction::Close => {
                    self.controller_config = None;
                    closed_controller_config = true;
                    self.mouse_captured = self.controller_config_was_captured;
                    self.apply_grab();
                }
                ControllerConfigAction::Cancelled | ControllerConfigAction::Handled => {}
            }
            if self.controller_config.is_some() { return; }
            if closed_controller_config {
                if let Some(window) = &self.window { window.request_redraw(); }
                return;
            }
        }
        if save_input { self.save_input_settings(); }
        if self.bindings.ui_shortcuts.exit_key == Some(code) { el.exit(); return; }
        if self.paused {
            if self.bindings.ui_shortcuts.pause_key == Some(code) { self.toggle_pause_menu(); return; }
            match code {
                Key::ArrowUp | Key::ArrowLeft => self.move_pause_selection(-1),
                Key::ArrowDown | Key::ArrowRight => self.move_pause_selection(1),
                Key::Enter | Key::NumpadEnter | Key::Space => self.choose_pause_action(),
                Key::Escape => self.toggle_pause_menu(),
                _ => {}
            }
            return;
        }
        if self.bindings.ui_shortcuts.pause_key == Some(code) && self.pause_is_available() {
            self.toggle_pause_menu();
            return;
        }
        if self.library_key(el, code) { return; }
        if self.bindings.ui_shortcuts.menu_key == Some(code) {
            self.toggle_menu();
            return;
        }
        if self.bound(code) { return; }
        let ctrl = self.mods.control_key();
        let alt = self.mods.alt_key();
        match code {
            Key::Escape => el.exit(),
            Key::F1 => push_action(GuiAction::OpenFeedback),
            Key::F6 => push_action(GuiAction::QuickSaveState),
            Key::F7 => push_action(GuiAction::QuickLoadState),
            Key::F9 => {
                let next_mode = match self.input_device_mode {
                    InputDeviceMode::Gamepad => InputDeviceMode::Mouse,
                    InputDeviceMode::Mouse => InputDeviceMode::Gamepad,
                };
                push_action(GuiAction::SetInputDevice(next_mode));
            }
            Key::KeyI if ctrl => self.open_dialog_action(false),
            Key::KeyO if ctrl => self.open_dialog_action(true),
            Key::F5 => push_action(GuiAction::Reset),
            Key::KeyH if !ctrl && !self.bound(code) => push_action(GuiAction::Reset),
            Key::KeyP if !ctrl && !self.bound(code) => {
                self.paused = !self.paused;
                info!("Emulation: {}", if self.paused { "PAUSED" } else { "RUNNING" });
                self.reset_telemetry();
                self.refresh_title();
            }
            Key::F11 => self.toggle_fullscreen(),
            Key::Enter | Key::NumpadEnter if alt => self.toggle_fullscreen(),
            Key::F8 => save_screenshot_ppm(self.frame_count, &self.m.bus.mcd212.framebuffer[..]),
            Key::F12 => save_screenshot(self.frame_count, &self.m.bus.mcd212.framebuffer[..]),
            Key::KeyM if !self.bound(code) => {
                self.m.bus.cdic.audio.muted = !self.m.bus.cdic.audio.muted;
                info!("Audio: {}", if self.m.bus.cdic.audio.muted { "MUTED" } else { "UNMUTED" });
            }
            _ => {}
        }
    }

    // Returns false when the loop must stop
    fn handle_actions(&mut self) -> bool {
        while let Some(action) = pop_action() {
            match action {
                GuiAction::InsertDisc(path) => {
                    info!("Hot-swapping disc into CD-i drive: {:?}", path);
                    match self.m.bus.change_disc(&path) {
                        Ok(_) => {
                            self.save_active_disc_settings();
                            self.activate_disc_settings(&path);
                            self.current_disc_path = Some(path.clone());
                            self.activate_nvram_for_selection();
                            self.boot_disc_art = library::load_disc_artwork(&path, &self.cd_art_path).or_else(|| library::load_disc_artwork(&path, &self.managed_cd_path));
                            self.current_game_name = opencdi::frontend::disc_display_name(&path);
                            self.add_recent(&path);
                            info!("New disc inserted successfully: {}", self.current_game_name);
                        }
                        Err(e) => {
                            error!("Failed to hot-swap disc {:?}: {:?}", path, e);
                            #[cfg(target_os = "windows")]
                            gui::show_disc_error(self.hwnd(), &format!("Errore nell'inserimento del disco:\n\n{}", e));
                        }
                    }
                }
                GuiAction::OpenDisc(path) => {
                    info!("Loading and booting new disc (Reset): {:?}", path);
                    match self.m.bus.load_cdrom(&path) {
                        Ok(_) => {
                            self.save_active_disc_settings();
                            self.activate_disc_settings(&path);
                            self.library_mode = false;
                            self.grab_applied = None;
                            self.apply_grab();
                            self.current_disc_path = Some(path.clone());
                            self.activate_nvram_for_selection();
                            self.boot_disc_art = library::load_disc_artwork(&path, &self.cd_art_path).or_else(|| library::load_disc_artwork(&path, &self.managed_cd_path));
                            self.current_game_name = opencdi::frontend::disc_display_name(&path);
                            self.add_recent(&path);
                            info!("Disc loaded successfully: {}", self.current_game_name);

                            if !self.dvc_disabled && !self.m.bus.dvc.is_fitted() {
                                if let Some(dvc_path) = self.dvc_arg.clone().or_else(find_dvc_rom) {
                                    if let Ok(_) = self.m.load_dvc_rom(&dvc_path) {
                                        info!("Loaded Digital Video Cartridge (DVC) extension from {:?}", dvc_path);
                                        #[cfg(target_os = "windows")]
                                        gui::update_dvc_menu(true);
                                    }
                                }
                            }

                            let name_upper = self.current_game_name.to_uppercase();
                            let is_ntsc = (name_upper.contains("(USA)") || name_upper.contains("(JAPAN)") || name_upper.contains("NTSC"))
                                && self.m.bus.cdrom.as_ref().map_or(true, |cd| cd.disc_type != 0x20);
                            let mode = if is_ntsc {
                                info!("Detected NTSC disc (USA/Japan): auto-configured NTSC 60 Hz video mode");
                                VideoMode::Ntsc
                            } else {
                                VideoMode::Pal
                            };
                            self.m.bus.set_video_mode(mode);
                            #[cfg(target_os = "windows")]
                            gui::update_video_menu(mode);

                            self.m.bus.reset_system();
                            self.m.cpu.reset(&mut self.m.bus);
                            self.reset_state();
                            self.mouse_captured = true;
                            self.m.bus.emulate_wait_states = true;
                            self.next_frame_time = Instant::now();
                            self.reset_telemetry();
                        }
                        Err(e) => {
                            error!("Failed to load disc {:?}: {:?}", path, e);
                            #[cfg(target_os = "windows")]
                            gui::show_disc_error(self.hwnd(), &format!("Errore nel caricamento del disco:\n\n{}", e));
                        }
                    }
                }
                GuiAction::EjectDisc => {
                    info!("Ejecting disc from CD-i drive");
                    self.save_active_disc_settings();
                    self.m.bus.slave.set_disc_inserted(false);
                    self.m.bus.cdrom = None;
                    self.current_disc_path = None;
                    self.active_disc_config_path = None;
                    self.bindings = self.global_bindings.clone();
                    self.input_device_mode = self.bindings.ui_shortcuts.input_device;
                    self.activate_nvram_for_selection();
                    self.disc_launched = false;
                    self.shell_ready_count = 0;
                    self.current_game_name = "No disc inserted (Standalone BIOS)".to_string();
                    self.pad_x = 384;
                    self.pad_y = 280;
                    self.pad_x_f = 384.0;
                    self.pad_y_f = 280.0;
                }
                GuiAction::StopToLibrary => {
                    self.stop_to_library();
                }
                GuiAction::Reset => {
                    info!("Console reset requested");
                    self.m.bus.reset_system();
                    self.m.cpu.reset(&mut self.m.bus);
                    self.reset_state();
                    self.mouse_captured = true;
                    self.m.bus.emulate_wait_states = true;
                    self.next_frame_time = Instant::now();
                    self.reset_telemetry();
                }
                GuiAction::Exit => {
                    info!("Exit requested from File menu");
                    return false;
                }
                GuiAction::SetProfile(profile) => {
                    self.bindings.set_profile(profile);
                    #[cfg(target_os = "windows")]
                    gui::update_profile_menu(self.bindings.profile);
                    info!("Control profile updated to: {:?}", self.bindings.profile);
                }
                GuiAction::SetInputDevice(mode) => {
                    self.input_device_mode = mode;
                    self.bindings.ui_shortcuts.input_device = mode;
                    self.save_input_settings();
                    #[cfg(target_os = "windows")]
                    gui::update_input_device_menu(mode);
                    info!("Input device mode set to: {:?}", mode);
                }
                GuiAction::ReloadControls => {
                    self.bindings.load_from_ini();
                    #[cfg(target_os = "windows")]
                    gui::update_profile_menu(self.bindings.profile);
                    info!("Controls reloaded from INI file. Profile: {:?}", self.bindings.profile);
                }
                GuiAction::OpenControllerConfig => {
                    self.controller_config_was_captured = self.mouse_captured;
                    self.controller_config = Some(ControllerConfigState::default());
                    self.mouse_captured = false;
                    self.m.bus.slave.update_mouse(self.pad_x, self.pad_y, 0);
                    self.prev_key_btn = 0;
                    self.apply_grab();
                    if let Some(window) = &self.window { window.request_redraw(); }
                }
                GuiAction::ToggleReleaseMouse => {
                    self.mouse_captured = !self.mouse_captured;
                    info!("Mouse capture: {}", if self.mouse_captured { "CAPTURED" } else { "RELEASED" });
                }
                GuiAction::ToggleMute => {
                    self.m.bus.cdic.audio.muted = !self.m.bus.cdic.audio.muted;
                    info!("Audio: {}", if self.m.bus.cdic.audio.muted { "MUTED" } else { "UNMUTED" });
                }
                GuiAction::VolumeUp => {
                    self.m.bus.cdic.audio.volume = (self.m.bus.cdic.audio.volume + 0.05).min(2.0);
                }
                GuiAction::VolumeDown => {
                    self.m.bus.cdic.audio.volume = (self.m.bus.cdic.audio.volume - 0.05).max(0.0);
                }
                GuiAction::ToggleFullscreen => {
                    self.toggle_fullscreen();
                }
                GuiAction::SetDisplayMode(mode) => self.set_display_mode(mode),
                GuiAction::ToggleSkipShell => {
                    self.skip_shell = !self.skip_shell;
                    self.bindings.ui_shortcuts.skip_shell = self.skip_shell;
                    self.bindings.save_to_ini();
                    if self.m.bus.cdrom.is_some() && !self.disc_launched {
                        self.mouse_captured = self.skip_shell;
                        self.grab_applied = None;
                        self.apply_grab();
                    }
                    #[cfg(target_os = "windows")]
                    gui::update_skip_shell_menu(self.skip_shell);
                    info!("Skip Philips shell: {}", if self.skip_shell { "enabled" } else { "disabled" });
                }
                GuiAction::SetNvramMode(mode) => {
                    if self.nvram_mode != mode {
                        self.nvram_mode = mode;
                        self.bindings.ui_shortcuts.nvram_mode = mode;
                        self.bindings.save_to_ini();
                        self.activate_nvram_for_selection();
                        #[cfg(target_os = "windows")]
                        gui::update_nvram_mode_menu(mode);
                        info!("NVRAM storage mode: {:?}", mode);
                    }
                }
                GuiAction::AddDiscPath(path) => {
                    let key = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
                    let exists = self.disc_paths.iter().any(|p| std::fs::canonicalize(p).unwrap_or_else(|_| p.clone()) == key);
                    if !exists { self.disc_paths.push(path); }
                    self.persist_paths();
                    self.refresh_library();
                    info!("Disc folders updated; {} titles found.", self.library_items.len());
                }
                GuiAction::SetArtworkPath(kind, path) => {
                    let resolved = resolve_config_path(&path);
                    match kind {
                        ArtworkPathKind::Covers => { self.covers_path = resolved.clone(); self.bindings.ui_shortcuts.covers_path = path; }
                        ArtworkPathKind::Cd => { self.cd_art_path = resolved.clone(); self.bindings.ui_shortcuts.cd_art_path = path; }
                        ArtworkPathKind::Bezels => { self.bezels_path = resolved.clone(); self.bindings.ui_shortcuts.bezels_path = path; }
                    }
                    if kind == ArtworkPathKind::Cd {
                        self.boot_disc_art = self.current_disc_path.as_deref().and_then(|p| library::load_disc_artwork(p, &self.cd_art_path).or_else(|| library::load_disc_artwork(p, &self.managed_cd_path)));
                    }
                    self.bindings.save_to_ini();
                    self.refresh_library();
                    info!("Artwork path updated: {:?}", resolved);
                }
                GuiAction::RescanLibrary => {
                    let settings = UiShortcuts::load();
                    self.disc_paths = settings.disc_paths.iter().map(|p| resolve_config_path(p)).collect();
                    self.covers_path = resolve_config_path(&settings.covers_path);
                    self.cd_art_path = resolve_config_path(&settings.cd_art_path);
                    self.bezels_path = resolve_config_path(&settings.bezels_path);
                    self.bindings.ui_shortcuts.disc_paths = settings.disc_paths;
                    self.bindings.ui_shortcuts.covers_path = settings.covers_path;
                    self.bindings.ui_shortcuts.cd_art_path = settings.cd_art_path;
                    self.bindings.ui_shortcuts.bezels_path = settings.bezels_path;
                    self.refresh_library();
                    self.boot_disc_art = self.current_disc_path.as_deref().and_then(|p| library::load_disc_artwork(p, &self.cd_art_path).or_else(|| library::load_disc_artwork(p, &self.managed_cd_path)));
                    info!("Disc library rescanned; {} titles found.", self.library_items.len());
                }
                GuiAction::TakeScreenshot => {
                    save_screenshot(self.frame_count, &self.m.bus.mcd212.framebuffer[..]);
                }
                GuiAction::SetVideoMode(mode) => {
                    self.m.bus.set_video_mode(mode);
                    #[cfg(target_os = "windows")]
                    gui::update_video_menu(mode);
                    info!("Video mode set to: {:?}", mode);
                    self.reset_telemetry();
                }
                GuiAction::ToggleWaitStates => {
                    self.m.bus.emulate_wait_states = !self.m.bus.emulate_wait_states;
                    #[cfg(target_os = "windows")]
                    gui::update_waitstates_menu(self.m.bus.emulate_wait_states);
                    info!("Authentic Hardware Speed (Wait-States): {}", if self.m.bus.emulate_wait_states { "ENABLED (Authentic CD-i 220)" } else { "DISABLED (Turbo Zero-Wait)" });
                }
                GuiAction::ToggleDvc => {
                    if self.m.bus.dvc.is_fitted() {
                        info!("Removing Digital Video Cartridge (Base CD-i mode)");
                        self.m.bus.dvc = opencdi::dvc::Dvc::new();
                        #[cfg(target_os = "windows")]
                        gui::update_dvc_menu(false);
                    } else {
                        if let Some(dvc_path) = self.dvc_arg.clone().or_else(find_dvc_rom) {
                            if let Ok(_) = self.m.load_dvc_rom(&dvc_path) {
                                info!("Inserted Digital Video Cartridge (DVC) from {:?}", dvc_path);
                                #[cfg(target_os = "windows")]
                                gui::update_dvc_menu(true);
                            }
                        } else {
                            error!("Cannot enable DVC: vmpega.rom not found in dvc/, bios/, or root directory");
                        }
                    }
                    self.m.bus.reset_system();
                    self.m.cpu.reset(&mut self.m.bus);
                    self.reset_state();
                }
                GuiAction::ShowHelp => {
                    #[cfg(target_os = "windows")]
                    gui::post_menu_command(self.hwnd(), gui::ID_HELP_CONTROLS);
                }
                GuiAction::ShowAbout => {
                    #[cfg(target_os = "windows")]
                    gui::post_menu_command(self.hwnd(), gui::ID_HELP_ABOUT);
                }
                GuiAction::QuickSaveState => {
                    let st_path = savestate_path_for(&self.current_game_name, 1);
                    match opencdi::savestate::save_state_to_file(&st_path, &self.m.cpu, &self.m.bus, self.disc_launched, self.shell_ready_count, self.pad_x, self.pad_y) {
                        Ok(_) => info!("Quick Save State (Slot 1) salvato con successo: {:?}", st_path),
                        Err(e) => error!("Errore salvataggio Quick Save State (Slot 1): {:?}", e),
                    }
                }
                GuiAction::QuickLoadState => {
                    let st_path = savestate_path_for(&self.current_game_name, 1);
                    match opencdi::savestate::load_state_from_file(&st_path, &mut self.m.cpu, &mut self.m.bus, &mut self.disc_launched, &mut self.shell_ready_count, &mut self.pad_x, &mut self.pad_y) {
                        Ok(_) => {
                            self.pad_x_f = self.pad_x as f32;
                            self.pad_y_f = self.pad_y as f32;
                            self.reset_telemetry();
                            info!("Quick Load State (Slot 1) caricato con successo: {:?}", st_path);
                        }
                        Err(e) => error!("Errore caricamento Quick Load State (Slot 1): {:?}", e),
                    }
                }
                GuiAction::SaveState(slot) => {
                    let st_path = savestate_path_for(&self.current_game_name, slot);
                    match opencdi::savestate::save_state_to_file(&st_path, &self.m.cpu, &self.m.bus, self.disc_launched, self.shell_ready_count, self.pad_x, self.pad_y) {
                        Ok(_) => info!("Save State (Slot {}) salvato con successo: {:?}", slot, st_path),
                        Err(e) => error!("Errore salvataggio Save State (Slot {}): {:?}", slot, e),
                    }
                }
                GuiAction::LoadState(slot) => {
                    let st_path = savestate_path_for(&self.current_game_name, slot);
                    match opencdi::savestate::load_state_from_file(&st_path, &mut self.m.cpu, &mut self.m.bus, &mut self.disc_launched, &mut self.shell_ready_count, &mut self.pad_x, &mut self.pad_y) {
                        Ok(_) => {
                            self.pad_x_f = self.pad_x as f32;
                            self.pad_y_f = self.pad_y as f32;
                            self.reset_telemetry();
                            info!("Load State (Slot {}) caricato con successo: {:?}", slot, st_path);
                        }
                        Err(e) => error!("Errore caricamento Load State (Slot {}): {:?}", slot, e),
                    }
                }
                GuiAction::ClearNvram => {
                    let _ = self.m.bus.nvram.clear();
                    if let Some(ref path) = self.nvram_path {
                        let _ = std::fs::remove_file(path);
                    }
                    self.m.bus.reset_system();
                    self.m.cpu.reset(&mut self.m.bus);
                    self.reset_state();
                    info!("Console memory (NVRAM) cancellata e console riavviata.");
                }
                GuiAction::OpenFeedback => {
                    #[cfg(target_os = "windows")]
                    gui::open_feedback_url();
                    #[cfg(not(target_os = "windows"))]
                    let _ = std::process::Command::new("xdg-open").arg("https://github.com/sigsantilio/opencdi/issues").spawn();
                }
            }
        }
        true
    }

    fn autoplay(&mut self) {
        // With a disc present, retry the shell's Play CD-i click until execution enters the title.
        let m = &mut self.m;
        if m.bus.cdrom.is_some() && !self.disc_launched {
            if self.skip_shell {
                let shell_state = if m.bus.dvc.is_fitted() {
                    let s_dvc = m.bus.peek_long(0x00D08090u32.wrapping_add(0xFFFF9172)).unwrap_or(0);
                    if s_dvc >= 2 { s_dvc }
                    else { m.bus.peek_long(0x0000C000u32.wrapping_add(0xFFFF9172)).unwrap_or(0) }
                } else {
                    m.bus.peek_long(0x0000C000u32.wrapping_add(0xFFFF9172)).unwrap_or(0)
                };

                if shell_state >= 2 {
                    self.shell_ready_count += 1;
                    let click_phase = self.shell_ready_count % 120;
                    if self.shell_ready_count == 1 {
                        info!("CD-i shell is ready; automatically selecting Play CD-i while the disc boots...");
                    }
                    if click_phase == 10 {
                        m.bus.slave.update_mouse(575, 335, 0);
                    } else if click_phase == 20 {
                        m.bus.slave.update_mouse(575, 335, 1);
                    } else if click_phase == 35 {
                        m.bus.slave.update_mouse(575, 335, 0);
                    }
                }
            }
            if self.skip_shell {
                if ((m.cpu.pc >= 0x00010000 && m.cpu.pc < 0x00300000) || (m.cpu.pc >= 0x00D00000 && m.cpu.pc < 0x00E00000)) && self.frame_count > 250 {
                    info!("Disc program started (PC=0x{:06X}); automatic boot complete.", m.cpu.pc);
                    self.disc_launched = true;
                    m.bus.emulate_wait_states = true;
                    self.pad_x = 384;
                    self.pad_y = 280;
                    self.pad_x_f = 384.0;
                    self.pad_y_f = 280.0;
                    m.bus.slave.update_mouse(384, 280, 0);
                }
            }
        }
    }

    // Keyboard, gamepad and host mouse into the slave controller, once per emulated frame
    fn update_input(&mut self) {
        let is_active = self.focused && self.host_input && self.controller_config.is_none();

        // Middle mouse button (rotellina/tasto centrale): toggle mouse capture / release focus
        if is_active && self.mouse_middle && !self.prev_middle {
            self.mouse_captured = !self.mouse_captured;
            info!("Controllo mouse: {}", if self.mouse_captured { "CATTURATO (focus console, puntatore host nascosto)" } else { "LIBERO (puntatore desktop visibile, rotellina o clic per ricatturare)" });
        }
        self.prev_middle = self.mouse_middle;

        // Left click inside window re-captures mouse if released
        if is_active && !self.mouse_captured && self.mouse_left && self.cursor.is_some() {
            self.mouse_captured = true;
            info!("Controllo mouse: CATTURATO (focus console, puntatore host nascosto)");
        }
        self.apply_grab();

        // Directional state from Keyboard (Arrow keys, WASD, or current profile)
        let (key_up, key_down, key_left, key_right, key_btn1, key_btn2, key_btn3) =
            (self.bindings.up, self.bindings.down, self.bindings.left, self.bindings.right, self.bindings.btn1, self.bindings.btn2, self.bindings.btn3);
        let (pad_up, pad_down, pad_left, pad_right, pad_btn1, pad_btn2, pad_btn3) =
            (self.bindings.pad_up, self.bindings.pad_down, self.bindings.pad_left, self.bindings.pad_right, self.bindings.pad_btn1, self.bindings.pad_btn2, self.bindings.pad_btn3);
        let mut dir_x: i32 = 0;
        let mut dir_y: i32 = 0;
        let mut key_btn = 0;

        if is_active {
            if self.key(key_left) {
                dir_x -= 1;
            }
            if self.key(key_right) {
                dir_x += 1;
            }
            if self.key(key_up) {
                dir_y -= 1;
            }
            if self.key(key_down) {
                dir_y += 1;
            }

            // Button 1 (Action / Sword / Start) and Button 2 (Jump / Secondary)
            // Space is fast-forward unless bound in the ini; Enter with Alt is fullscreen
            if self.key(key_btn1) {
                key_btn |= 1;
            }
            if self.key(key_btn2) {
                key_btn |= 2;
            }
            if self.key(key_btn3) {
                key_btn |= 3;
            }
        }

        // Gamepad handling via Gilrs (Xbox / PlayStation / USB)
        let mut menu_pad_down = false;
        let mut exit_pad_down = false;
        let mut pause_pad_down = false;
        let mut lib_left = false; let mut lib_right = false; let mut lib_up = false; let mut lib_down = false; let mut lib_confirm = false;
        let mut cfg_left = false; let mut cfg_right = false; let mut cfg_up = false; let mut cfg_down = false; let mut cfg_confirm = false;
        let mut captured_button = None;
        let mut currently_pressed_buttons = Vec::new();
        if let Some(ref mut g) = self.gilrs {
            while let Some(event) = g.next_event() {
                if captured_button.is_none() {
                    if let gilrs::EventType::ButtonPressed(button, _) = event.event { captured_button = Some(button); }
                }
            }
            for (_id, gamepad) in g.gamepads().filter(|_| self.host_input) {
                if gamepad.is_connected() {
                    for button in [gilrs::Button::South, gilrs::Button::East, gilrs::Button::West, gilrs::Button::North,
                        gilrs::Button::Start, gilrs::Button::Select, gilrs::Button::Mode, gilrs::Button::LeftTrigger,
                        gilrs::Button::RightTrigger, gilrs::Button::LeftTrigger2, gilrs::Button::RightTrigger2,
                        gilrs::Button::LeftThumb, gilrs::Button::RightThumb, gilrs::Button::DPadUp,
                        gilrs::Button::DPadDown, gilrs::Button::DPadLeft, gilrs::Button::DPadRight] {
                        if gamepad.is_pressed(button) && !currently_pressed_buttons.contains(&button) { currently_pressed_buttons.push(button); }
                    }
                    let menu_chord = &self.bindings.ui_shortcuts.menu_pad;
                    if !menu_chord.is_empty() && menu_chord.iter().all(|b| gamepad.is_pressed(*b)) { menu_pad_down = true; }
                    let exit_chord = &self.bindings.ui_shortcuts.exit_pad;
                    if !exit_chord.is_empty() && exit_chord.iter().all(|b| gamepad.is_pressed(*b)) { exit_pad_down = true; }
                    let pause_combo = &self.bindings.ui_shortcuts.pause_pad;
                    if !pause_combo.is_empty() && pause_combo.iter().all(|button| gamepad.is_pressed(*button)) { pause_pad_down = true; }
                    let mut pad_dir_x = 0;
                    let mut pad_dir_y = 0;

                    if gamepad.is_pressed(pad_up) { pad_dir_y -= 1; }
                    if gamepad.is_pressed(pad_down) { pad_dir_y += 1; }
                    if gamepad.is_pressed(pad_left) { pad_dir_x -= 1; }
                    if gamepad.is_pressed(pad_right) { pad_dir_x += 1; }

                    // The left analog stick remains available alongside the configurable digital buttons.
                    if pad_dir_x == 0 {
                        let lx = gamepad.value(gilrs::Axis::LeftStickX);
                        if lx < -0.35 { pad_dir_x = -1; }
                        else if lx > 0.35 { pad_dir_x = 1; }
                    }
                    if pad_dir_y == 0 {
                        let ly = gamepad.value(gilrs::Axis::LeftStickY);
                        if ly > 0.35 { pad_dir_y = -1; }
                        else if ly < -0.35 { pad_dir_y = 1; }
                    }

                    if self.library_mode {
                        lib_left |= pad_dir_x < 0; lib_right |= pad_dir_x > 0;
                        lib_up |= pad_dir_y < 0; lib_down |= pad_dir_y > 0;
                        lib_confirm |= gamepad.is_pressed(gilrs::Button::South);
                    }

                    if self.controller_config.as_ref().is_some_and(|s| !s.listening) {
                        cfg_left |= pad_dir_x < 0; cfg_right |= pad_dir_x > 0;
                        cfg_up |= pad_dir_y < 0; cfg_down |= pad_dir_y > 0;
                        cfg_confirm |= gamepad.is_pressed(gilrs::Button::South);
                    }

                    dir_x += pad_dir_x;
                    dir_y += pad_dir_y;

                    if gamepad.is_pressed(pad_btn1) {
                        key_btn |= 1;
                    }
                    if gamepad.is_pressed(pad_btn2) {
                        key_btn |= 2;
                    }
                    if gamepad.is_pressed(pad_btn3) {
                        key_btn |= 3;
                    }
                }
            }
        }
        pause_pad_down |= self.bindings.ui_shortcuts.pause_pad.contains(&gilrs::Button::Mode) && xbox_guide_pressed();
        if let Some(button) = captured_button {
            let mut save = false;
            if let Some(config) = self.controller_config.as_mut() {
                if config.listening && config.target == ControllerConfigTarget::Gamepad {
                    if config.row == 7 { self.bindings.ui_shortcuts.set_pause_combo(vec![button]); }
                    else { self.bindings.set_pad_binding(config.row, button); }
                    config.listening = false;
                    save = true;
                }
            }
            if save { self.save_input_settings(); }
        }
        let mut save_combo = false;
        if let Some(config) = self.controller_config.as_mut() {
            if config.listening && matches!(config.target, ControllerConfigTarget::PausePad | ControllerConfigTarget::ExitPad) {
                if !currently_pressed_buttons.is_empty() { config.pending_combo = currently_pressed_buttons.clone(); }
                else if !config.pending_combo.is_empty() {
                    let combo = std::mem::take(&mut config.pending_combo);
                    match config.target {
                        ControllerConfigTarget::PausePad => self.bindings.ui_shortcuts.set_pause_combo(combo),
                        ControllerConfigTarget::ExitPad => self.bindings.ui_shortcuts.set_exit_combo(combo),
                        _ => {}
                    }
                    config.listening = false;
                    save_combo = true;
                }
            }
        }
        if save_combo { self.save_input_settings(); }
        if let Some(config) = self.controller_config.as_mut() {
            if !config.listening {
                if cfg_up && !self.config_prev_up { config.row = (config.row + controller_config::ROWS - 1) % controller_config::ROWS; }
                if cfg_down && !self.config_prev_down { config.row = (config.row + 1) % controller_config::ROWS; }
                if (cfg_left && !self.config_prev_left) || (cfg_right && !self.config_prev_right) {
                    config.target = if config.target == ControllerConfigTarget::Keyboard { ControllerConfigTarget::Gamepad } else { ControllerConfigTarget::Keyboard };
                }
                if cfg_confirm && !self.config_prev_confirm { config.listening = true; }
            }
        }
        self.config_prev_up = cfg_up; self.config_prev_down = cfg_down;
        self.config_prev_left = cfg_left; self.config_prev_right = cfg_right; self.config_prev_confirm = cfg_confirm;
        if self.controller_config.is_some() { return; }
        if menu_pad_down && !self.prev_menu_pad { self.toggle_menu(); }
        self.prev_menu_pad = menu_pad_down;
        if pause_pad_down && !self.prev_pause_pad && self.pause_is_available() { self.toggle_pause_menu(); }
        self.prev_pause_pad = pause_pad_down;
        if exit_pad_down { self.exit_requested = true; }
        if self.library_mode {
            if lib_left && !self.library_prev_left { self.move_library_selection(-1); }
            if lib_right && !self.library_prev_right { self.move_library_selection(1); }
            let columns = library::layout(self.bindings.ui_shortcuts.artwork_size).columns as isize;
            if lib_up && !self.library_prev_up { self.move_library_selection(-columns); }
            if lib_down && !self.library_prev_down { self.move_library_selection(columns); }
            if lib_confirm && !self.library_prev_confirm { self.launch_library_selection(); }
        }
        self.library_prev_left = lib_left; self.library_prev_right = lib_right;
        self.library_prev_up = lib_up; self.library_prev_down = lib_down; self.library_prev_confirm = lib_confirm;

        // Mouse pointer: host cursor mapped through the letterbox onto the 768x560 picture
        let mut mouse_btn = 0;
        let mut mouse_x = self.pad_x;
        let mut mouse_y = self.pad_y;

        if is_active && self.mouse_captured {
            if let Some((x, y)) = self.rel_pointer {
                (mouse_x, mouse_y) = (x.round() as i32, y.round() as i32);
            } else if let Some(pos) = self.cursor {
                (mouse_x, mouse_y) = cursor_to_cdi(pos, self.image_rect);
            }
            if self.rel_pointer.is_some() || self.cursor.is_some() {
                if self.mouse_left { mouse_btn |= 1; }
                if self.mouse_right { mouse_btn |= 2; }
            }
        }

        let m = &mut self.m;
        if m.bus.slave.device_mouse_updated {
            self.pad_x = m.bus.slave.device_mouse_x;
            self.pad_y = m.bus.slave.device_mouse_y;
            self.pad_x_f = self.pad_x as f32;
            self.pad_y_f = self.pad_y as f32;
            m.bus.slave.device_mouse_updated = false;
            if self.rel_pointer.is_some() {
                self.rel_pointer = Some((self.pad_x as f64, self.pad_y as f64));
            }
        }

        if !self.disc_launched && m.bus.cdrom.is_some() && self.skip_shell {
            // Keep host input from interfering with the automatic Play CD-i selection.
        } else {
            match self.input_device_mode {
                InputDeviceMode::Gamepad => {
                    const PAD_SPEED: f32 = 2.5;
                    let mut moved = false;
                    if dir_x != 0 {
                        self.pad_x_f = (self.pad_x_f + (dir_x.clamp(-1, 1) as f32) * PAD_SPEED).clamp(0.0, 767.0);
                        self.pad_x = self.pad_x_f.round() as i32;
                        moved = true;
                    }
                    if dir_y != 0 {
                        self.pad_y_f = (self.pad_y_f + (dir_y.clamp(-1, 1) as f32) * PAD_SPEED).clamp(0.0, 559.0);
                        self.pad_y = self.pad_y_f.round() as i32;
                        moved = true;
                    }
                    if moved || key_btn != self.prev_key_btn {
                        m.bus.slave.update_mouse(self.pad_x, self.pad_y, key_btn);
                    }
                    self.prev_key_btn = key_btn;
                }
                InputDeviceMode::Mouse => {
                    self.pad_x = mouse_x;
                    self.pad_y = mouse_y;
                    self.pad_x_f = mouse_x as f32;
                    self.pad_y_f = mouse_y as f32;
                    m.bus.slave.update_mouse(self.pad_x, self.pad_y, mouse_btn | key_btn);
                }
            }
        }

        // Audio hotkeys (+ / - = Volume, held)
        let audio = &mut self.m.bus.cdic.audio;
        if self.keys.contains(&Key::Equal) || self.keys.contains(&Key::NumpadAdd) {
            audio.volume = (audio.volume + 0.02).min(2.0);
        }
        if self.keys.contains(&Key::Minus) || self.keys.contains(&Key::NumpadSubtract) {
            audio.volume = (audio.volume - 0.02).max(0.0);
        }
    }

    // Start a fresh speed window (pause, video mode switch, reset, fast-forward toggle)
    fn reset_telemetry(&mut self) {
        self.fps_timer = Instant::now();
        self.telemetry_frames = 0;
        self.prev_instructions = self.m.instructions;
        self.prev_cd_sectors = self.m.bus.cdic.curr_lba;
        self.buffer_flips_interval = 0;
    }

    fn update_telemetry(&mut self) {
        self.telemetry_frames += 1;
        let m = &self.m;
        let fps_interval = if m.bus.mcd212.video_mode == VideoMode::Pal { 25 } else { 30 };
        if self.telemetry_frames < fps_interval {
            return;
        }
        let elapsed_secs = self.fps_timer.elapsed().as_secs_f64();
        if elapsed_secs > 0.0 {
            self.measured_fps = (self.telemetry_frames as f64) / elapsed_secs;
            let inst_delta = m.instructions.saturating_sub(self.prev_instructions);
            self.measured_mips = (inst_delta as f64 / elapsed_secs) / 1_000_000.0;
            self.prev_instructions = m.instructions;

            let curr_sectors = m.bus.cdic.curr_lba;
            let sec_delta = curr_sectors.saturating_sub(self.prev_cd_sectors);
            self.measured_cd_rate = sec_delta as f64 / elapsed_secs;
            self.prev_cd_sectors = curr_sectors;

            self.measured_render_fps = (self.buffer_flips_interval as f64) / elapsed_secs;
            self.buffer_flips_interval = 0;
        }
        self.fps_timer = Instant::now();
        self.telemetry_frames = 0;

        let target_fps = m.bus.mcd212.video_mode.fps();
        let speed_pct = (self.measured_fps / target_fps) * 100.0;

        // Real-Time Anomaly Detector
        if self.frame_count > 100 && !self.fast_forward && (speed_pct > 108.0 || (speed_pct < 92.0 && speed_pct > 20.0)) {
            log::warn!(
                "[TELEMETRY SPEED ANOMALY] Speed: {:.1}% ({:.1} FPS on target {:.0} Hz) | MIPS: {:.2} | CD: {:.1} sec/s",
                speed_pct, self.measured_fps, target_fps, self.measured_mips, self.measured_cd_rate
            );
        }
        if self.measured_mips > 4.5 && m.bus.emulate_wait_states {
            log::warn!(
                "[TELEMETRY CPU ANOMALY] Unexpected CPU throughput: {:.2} MIPS (expected ~3.2-3.5 MIPS for SCC68070 at 15.5MHz)",
                self.measured_mips
            );
        }

        self.refresh_title();
    }

    // Title bar from the last measurements; shows PAUSED instead of the speed while paused
    fn refresh_title(&self) {
        let m = &self.m;
        let video_mode = match m.bus.mcd212.video_mode {
            VideoMode::Pal => "50Hz (PAL)",
            VideoMode::Ntsc => "60Hz (NTSC)",
        };
        let wait_status = if m.bus.emulate_wait_states { "ON" } else { "TURBO" };
        let cd_speed_x = self.measured_cd_rate / 75.0;
        let cd_str = if self.measured_cd_rate > 0.5 {
            format!("{:.1} sec/s ({:.1}x)", self.measured_cd_rate, cd_speed_x)
        } else {
            "IDLE".to_string()
        };
        let render_str = if self.measured_render_fps > 0.5 {
            format!("{:.0} FPS", self.measured_render_fps)
        } else {
            format!("{:.0} FPS", self.measured_fps)
        };
        let speed = if self.paused {
            format!("PAUSED ({})", video_mode)
        } else {
            let speed_pct = (self.measured_fps / m.bus.mcd212.video_mode.fps()) * 100.0;
            format!("{:.0}% ({:.1} FPS {})", speed_pct, self.measured_fps, video_mode)
        };
        if let Some(window) = &self.window {
            window.set_title(&format!(
                "OpenCDi – {} | {} | CPU: {:.2} MIPS | CD: {} | Render: {} | WaitStates: {}",
                self.current_game_name, speed, self.measured_mips, cd_str, render_str, wait_status
            ));
        }
    }

    // One emulated frame plus host input; returns false when the loop must stop
    fn tick(&mut self) -> bool {
        if !self.handle_actions() {
            return false;
        }
        if self.restore_check.is_some_and(|t| Instant::now() >= t) {
            self.restore_windowed();
            self.restore_check = None;
            self.windowed = None;
        }
        if self.paused {
            if self.controller_config.is_some() {
                self.update_input();
                if let Some(window) = &self.window { window.request_redraw(); }
            } else {
                self.update_pause_input();
            }
            if self.exit_requested { return false; }
            if !self.paused { return true; }
            // Keep the title live (disc, wait states) at about two updates a second
            self.paused_ticks += 1;
            if self.paused_ticks % 25 == 0 {
                self.refresh_title();
            }
            return true;
        }
        if self.library_mode {
            self.update_input();
            if self.exit_requested { return false; }
            if let Some(window) = &self.window { window.request_redraw(); }
            return true;
        }
        self.autoplay();
        self.m.run_frame();
        if self.frame_count % 300 == 0 {
            self.m.bus.nvram.flush_if_dirty();
        }

        let curr_vsr = (self.m.bus.mcd212.get_vsr::<0>(), self.m.bus.mcd212.get_vsr::<1>());
        if curr_vsr != self.prev_vsr {
            self.prev_vsr = curr_vsr;
            self.buffer_flips_interval += 1;
        }

        self.update_input();
        if self.exit_requested { return false; }
        // Emulation runs on the timer; only the picture is skipped while the window is hidden
        if self.fast_forward {
            // Drop fast-forward audio so the queues never hold sped-up backlog
            let audio = &self.m.bus.cdic.audio;
            for q in [&audio.sample_queue, &audio.sfx_queue] {
                if let Ok(mut q) = q.lock() {
                    q.clear();
                }
            }
        }
        // While fast-forwarding, present at most about 60 times a second
        let due = !self.in_modal_pump && (!self.fast_forward || self.last_present.elapsed() >= Duration::from_millis(16));
        if let Some(window) = self.window.as_ref().filter(|_| !self.occluded && due) {
            window.request_redraw();
        }

        self.frame_count += 1;
        self.update_telemetry();

        self.frames_run += 1;
        if self.max_frames.is_some_and(|n| self.frames_run >= n) {
            info!("Stopping after {} frames (--frames)", self.frames_run);
            return false;
        }
        true
    }

    // Precision host frame pacing: frames start on a fixed schedule, a late frame drops the backlog
    fn schedule_next(&mut self) {
        let target_frame_duration = self.m.bus.mcd212.video_mode.frame_duration();
        let now = Instant::now();
        if now < self.next_frame_time {
            self.run_at = self.next_frame_time;
            self.next_frame_time += target_frame_duration;
        } else {
            // Drop backlog immediately so emulation never bursts at host CPU speed
            self.run_at = now;
            self.next_frame_time = now + target_frame_duration;
        }
    }

    // --run-in-menu: steps due frames from a Win32 modal loop, where about_to_wait does not run; true if a frame ran
    #[cfg(target_os = "windows")]
    fn modal_pump(&mut self, el: &ActiveEventLoop) -> bool {
        let mut ran = false;
        for _ in 0..4 {
            if el.exiting() || Instant::now() < self.run_at {
                break;
            }
            self.in_modal_pump = true;
            let go = self.tick();
            self.in_modal_pump = false;
            ran = true;
            if !go {
                el.exit();
                gui::end_menu();
                break;
            }
            self.schedule_next();
        }
        ran
    }

    fn present(&mut self) {
        let render_pause_menu = self.paused && self.disc_launched && self.m.bus.cdrom.is_some();
        let (Some(window), Some(surface)) = (&self.window, &mut self.surface) else { return };
        let size = window.inner_size();
        let (Some(w), Some(h)) = (std::num::NonZeroU32::new(size.width), std::num::NonZeroU32::new(size.height)) else { return };
        if surface.resize(w, h).is_err() {
            return;
        }
        self.last_present = Instant::now();
        let (dw, dh) = (size.width as usize, size.height as usize);
        self.image_rect = if self.library_mode || self.controller_config.is_some() { (0,0,dw,dh) } else { image_rect(dw, dh) };
        if let Ok(mut buffer) = surface.buffer_mut() {
            if let Some(state) = &self.controller_config {
                controller_config::render_native(&mut buffer,dw,dh,self.image_rect,&self.bindings,&self.bindings.ui_shortcuts,self.input_device_mode,state,self.controller_art.as_ref(),self.mouse_art.as_ref());
            } else if self.library_mode {
                library::render_native(&mut buffer,dw,dh,self.image_rect,&self.library_items,self.library_selected,self.bindings.ui_shortcuts.artwork_size,self.menu_visible,self.display_mode,self.toolbar_popup);
            } else {
                let frame = if self.skip_shell && self.m.bus.cdrom.is_some() && !self.disc_launched {
                let mut frame = self.m.bus.mcd212.framebuffer.to_vec();
                library::render_boot(&mut frame, &self.current_game_name, self.boot_disc_art.as_ref(), self.frame_count);
                frame
                } else {
                    self.m.bus.mcd212.framebuffer.to_vec()
                };
                blit_aspect(&mut buffer, dw, dh, &frame, self.image_rect);
            }
            if render_pause_menu && self.controller_config.is_none() { library::render_pause_menu_full(&mut buffer, dw, dh, &self.current_game_name, self.pause_menu_selected); }
            // Wayland: ties the next redraw to a frame callback, so a hidden window is never presented to
            window.pre_present_notify();
            let _ = buffer.present();
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("OpenCDi - Philips CD-i")
            .with_inner_size(LogicalSize::new(SCREEN_WIDTH as f64, SCREEN_HEIGHT as f64))
            .with_decorations(false)
            .with_fullscreen(Some(Fullscreen::Borderless(None)))
            .with_resizable(true)
            .with_window_icon(window_icon());
        let window = match el.create_window(attrs) {
            Ok(w) => Rc::new(w),
            Err(e) => {
                error!("Cannot create the window: {}", e);
                el.exit();
                return;
            }
        };
        let surface = softbuffer::Context::new(window.clone()).and_then(|ctx| softbuffer::Surface::new(&ctx, window.clone()));
        match surface {
            Ok(s) => self.surface = Some(s),
            Err(e) => {
                error!("Cannot create the drawing surface: {}", e);
                el.exit();
                return;
            }
        }
        let size = window.inner_size();
        self.image_rect = if self.library_mode || self.controller_config.is_some() { (0,0,size.width as usize,size.height as usize) } else { image_rect(size.width as usize, size.height as usize) };
        self.focused = window.has_focus();
        self.window = Some(window);
        self.apply_grab();

        // Attach Native Win32 Menu GUI
        #[cfg(target_os = "windows")]
        {
            gui::set_run_in_modal(self.run_in_menu);
            gui::attach_gui_menu(self.hwnd(), self.bindings.profile, self.input_device_mode, self.m.bus.mcd212.video_mode);
            gui::set_menu_visible(self.hwnd(), false);
            gui::update_display_mode_menu(self.display_mode);
            gui::update_skip_shell_menu(self.skip_shell);
            gui::update_nvram_mode_menu(self.nvram_mode);
            gui::update_dvc_menu(self.m.bus.dvc.is_fitted());
        }
        let now = Instant::now();
        self.run_at = now;
        self.next_frame_time = now;
        self.fps_timer = now;
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::RedrawRequested => {
                #[cfg(target_os = "windows")]
                if self.run_in_menu && gui::modal_active() {
                    let timer = gui::take_timer_paint();
                    if !self.modal_pump(el) && timer {
                        return;
                    }
                }
                self.present();
            }
            WindowEvent::Resized(size) => {
                self.image_rect = if self.library_mode || self.controller_config.is_some() { (0,0,size.width as usize,size.height as usize) } else { image_rect(size.width as usize, size.height as usize) };
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::Focused(f) => {
                self.focused = f;
                if !f {
                    self.keys.clear();
                    self.mouse_left = false;
                    self.mouse_right = false;
                    self.mouse_middle = false;
                }
                self.apply_grab();
            }
            WindowEvent::Occluded(o) => self.occluded = o,
            // --no-host-input: keyboard and mouse never reach the emulator or the shortcuts
            WindowEvent::KeyboardInput { .. } | WindowEvent::MouseInput { .. } if !self.host_input => {}
            WindowEvent::ModifiersChanged(mods) => self.mods = mods.state(),
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    if event.state == ElementState::Pressed {
                        self.keys.insert(code);
                        if !event.repeat {
                            self.on_key_pressed(el, code);
                        }
                    } else {
                        self.keys.remove(&code);
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = Some(position);
                if self.library_size_dragging {
                    let (x, y) = cursor_to_cdi(position, self.image_rect);
                    if (library::H as i32-58..library::H as i32-12).contains(&y) {
                        self.set_artwork_size(library::size_from_slider_x(x.max(0) as usize));
                    }
                }
            }
            WindowEvent::CursorLeft { .. } => self.cursor = None,
            WindowEvent::MouseWheel { .. } if !self.host_input => {}
            WindowEvent::MouseWheel { delta, .. } => {
                if self.library_mode && self.controller_config.is_none() {
                    let amount = match delta {
                        MouseScrollDelta::LineDelta(_, y) => y as f64,
                        MouseScrollDelta::PixelDelta(pos) => pos.y / 60.0,
                    };
                    self.library_scroll_accum += amount;
                    let rows = self.library_scroll_accum.trunc() as isize;
                    if rows != 0 {
                        let columns = library::layout(self.bindings.ui_shortcuts.artwork_size).columns as isize;
                        self.move_library_selection(-rows * columns);
                        self.library_scroll_accum -= rows as f64;
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let down = state == ElementState::Pressed;
                if down && button == MouseButton::Left && self.paused && self.controller_config.is_none() {
                    if let Some(pos) = self.cursor {
                        let (width, height) = self.window.as_ref().map(|w| { let s = w.inner_size(); (s.width as usize, s.height as usize) }).unwrap_or((0, 0));
                        if let Some(row) = library::pause_menu_hit_full(pos.x.max(0.0) as usize, pos.y.max(0.0) as usize, width, height) {
                            self.pause_menu_selected = row;
                            self.choose_pause_action();
                            self.mouse_left = true;
                            return;
                        }
                    }
                }
                if down && button == MouseButton::Left && self.library_mode && self.controller_config.is_none() && self.menu_visible {
                    if let Some(pos) = self.cursor {
                        let (x, y) = cursor_to_cdi(pos, self.image_rect);
                        let px = x.max(0) as usize;
                        let py = y.max(0) as usize;
                        let on_toolbar = (0..library::TOOLBAR_HEIGHT as i32).contains(&y);
                        let on_popup = match self.toolbar_popup {
                            Some(library::ToolbarPopup::Paths) => (229..457).contains(&px) && (library::TOOLBAR_HEIGHT..library::TOOLBAR_HEIGHT + 148).contains(&py),
                            Some(library::ToolbarPopup::Display) => (427..633).contains(&px) && (library::TOOLBAR_HEIGHT..library::TOOLBAR_HEIGHT + 82).contains(&py),
                            None => false,
                        };
                        if on_toolbar || on_popup {
                            if let Some(action) = library::toolbar_hit(px, py, self.toolbar_popup) { self.handle_toolbar_action(action); }
                            else { self.toolbar_popup = None; }
                            self.mouse_left = true;
                            return;
                        }
                    }
                }
                if self.controller_config.is_some() {
                    if down && button == MouseButton::Left {
                        if let Some(pos) = self.cursor {
                            let (x, y) = cursor_to_cdi(pos, self.image_rect);
                            if let Some(hit) = controller_config::hit_test(x as usize, y as usize) {
                                match hit {
                                    ControllerConfigHit::Binding(_, _) | ControllerConfigHit::Hotkey(_) => {
                                        if let Some(config) = self.controller_config.as_mut() { config.click(hit); }
                                    }
                                    ControllerConfigHit::Device(mode) => {
                                        self.input_device_mode = mode;
                                        self.bindings.ui_shortcuts.input_device = mode;
                                        self.save_input_settings();
                                    }
                                    ControllerConfigHit::Close => {
                                        if let Some(config) = self.controller_config.as_mut() { config.click(hit); }
                                        self.controller_config = None;
                                        self.mouse_captured = self.controller_config_was_captured;
                                        self.apply_grab();
                                    }
                                }
                            }
                        }
                    }
                    self.mouse_left = down;
                    return;
                }
                if self.library_mode && button == MouseButton::Left {
                    if !down && self.library_size_dragging {
                        self.library_size_dragging = false;
                        self.bindings.save_to_ini();
                        self.mouse_left = false;
                        return;
                    }
                    if down {
                        if let Some(pos) = self.cursor {
                            let (x, y) = cursor_to_cdi(pos, self.image_rect);
                            if library::size_slider_hit(x.max(0) as usize, y.max(0) as usize) {
                                self.library_size_dragging = true;
                                self.set_artwork_size(library::size_from_slider_x(x.max(0) as usize));
                                self.mouse_left = true;
                                return;
                            }
                        }
                    }
                }
                if self.library_mode && down && button == MouseButton::Left {
                    if let Some(pos) = self.cursor {
                        let (x, y) = cursor_to_cdi(pos, self.image_rect);
                        if let Some(slot) = library::slot_at(x.max(0) as usize, y.max(0) as usize, self.bindings.ui_shortcuts.artwork_size) {
                            let per_page = library::layout(self.bindings.ui_shortcuts.artwork_size).visible_count();
                            let index = (self.library_selected / per_page) * per_page + slot;
                            if index < self.library_items.len() {
                                if self.library_selected == index && self.library_click.is_some_and(|(prev, t)| prev == index && t.elapsed() < Duration::from_millis(500)) {
                                    self.library_selected = index;
                                    self.launch_library_selection();
                                } else { self.library_selected = index; }
                                self.library_click = Some((index, Instant::now()));
                            }
                        }
                    }
                }
                if down && self.bindings.ui_shortcuts.menu_mouse == Some(button) { self.toggle_menu(); }
                match button {
                    MouseButton::Left => self.mouse_left = down,
                    MouseButton::Right => self.mouse_right = down,
                    MouseButton::Middle => self.mouse_middle = down,
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn device_event(&mut self, _el: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
        // Relative motion in host pixels, scaled like the letterboxed picture
        if let (DeviceEvent::MouseMotion { delta: (dx, dy) }, Some((x, y))) = (event, self.rel_pointer) {
            let (_, _, w, h) = self.image_rect;
            if w > 0 && h > 0 {
                let x = (x + dx * SCREEN_WIDTH as f64 / w as f64).clamp(0.0, (SCREEN_WIDTH - 1) as f64);
                let y = (y + dy * SCREEN_HEIGHT as f64 / h as f64).clamp(0.0, (SCREEN_HEIGHT - 1) as f64);
                self.rel_pointer = Some((x, y));
            }
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        if self.window.is_none() || el.exiting() {
            return;
        }
        let artwork_result = self.artwork_fetch_rx.as_ref().and_then(|rx| rx.try_recv().ok());
        if let Some(count) = artwork_result {
            self.artwork_fetch_rx = None;
            if count > 0 {
                info!("Downloaded {} missing artwork image(s) from LaunchBox", count);
                let mut roots = library_roots();
                roots.extend(self.disc_paths.iter().cloned());
                self.library_items = library::scan(&roots, &self.covers_path, &self.cd_art_path, &self.managed_covers_path, &self.managed_cd_path);
                if let Some(window) = &self.window { window.request_redraw(); }
            }
        }
        let ff = self.host_input && self.focused && self.key(Key::Space) && !self.bound(Key::Space);
        if ff != self.fast_forward {
            self.fast_forward = ff;
            info!("Fast-forward: {}", if ff { "ON" } else { "OFF" });
            self.reset_telemetry();
        }
        let now = Instant::now();
        if !ff && now < self.run_at {
            let remaining = self.run_at - now;
            if remaining > Duration::from_millis(2) {
                el.set_control_flow(ControlFlow::WaitUntil(self.run_at - Duration::from_millis(1)));
                return;
            }
            while Instant::now() < self.run_at {
                std::hint::spin_loop();
            }
        }
        if !self.tick() {
            el.exit();
            return;
        }
        if ff {
            // Uncapped: next frame as soon as pending events are handled
            let now = Instant::now();
            self.run_at = now;
            self.next_frame_time = now + self.m.bus.mcd212.video_mode.frame_duration();
            el.set_control_flow(ControlFlow::Poll);
        } else {
            self.schedule_next();
            el.set_control_flow(ControlFlow::WaitUntil(self.run_at));
        }
    }

    fn exiting(&mut self, _el: &ActiveEventLoop) {
        // Restore host cursor and release mouse confinement on exit
        self.mouse_captured = false;
        self.apply_grab();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letterbox_matches_aspect_stretch() {
        assert_eq!(image_rect(768, 560), (0, 0, 768, 560));
        assert_eq!(image_rect(1920, 1080), (219, 0, 1481, 1080));
        assert_eq!(image_rect(768, 1000), (0, 220, 768, 560));
        let src: Vec<u32> = (0..(SCREEN_WIDTH * SCREEN_HEIGHT) as u32).collect();
        let mut dst = vec![1u32; SCREEN_WIDTH * SCREEN_HEIGHT];
        blit_aspect(&mut dst, SCREEN_WIDTH, SCREEN_HEIGHT, &src, image_rect(SCREEN_WIDTH, SCREEN_HEIGHT));
        assert_eq!(dst, src);
        let mut big = vec![1u32; 1536 * 1120];
        blit_aspect(&mut big, 1536, 1120, &src, image_rect(1536, 1120));
        assert_eq!(big[1536 * 3 + 5], src[SCREEN_WIDTH + 2]);
        assert_eq!(cursor_to_cdi(PhysicalPosition::new(100.0, 200.0), image_rect(768, 560)), (100, 200));
        assert_eq!(cursor_to_cdi(PhysicalPosition::new(219.0 + 1481.0, 1080.0), image_rect(1920, 1080)), (767, 559));
        assert_eq!(cursor_to_cdi(PhysicalPosition::new(0.0, 0.0), image_rect(1920, 1080)), (0, 0));
    }
}
