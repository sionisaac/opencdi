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

use opencdi::audio::{self, Frame, HostAudioOutput};
use opencdi::savestate::{Reader, State, StateError};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, atomic::{AtomicU32, Ordering}};
use opencdi::frontend::{app_data_dir, app_data_file, migrate_app_data_dir, pop_action, push_action, DisplayMode, GuiAction, InputDeviceMode, KeyBindings, NvramMode, UiShortcuts};
use crate::controller_config::{Action as ControllerConfigAction, Hit as ControllerConfigHit, State as ControllerConfigState, Target as ControllerConfigTarget};
#[cfg(target_os = "windows")]
use opencdi::gui;
use opencdi::machine::Machine;
use opencdi::mcd212::{SCREEN_WIDTH, SCREEN_HEIGHT, VideoMode};
mod library;
mod library_catalogue;
mod launchbox;
mod screenscraper;
mod synopsis;
mod world_of_cdi;
mod missing_media;
mod boxart;
mod disc_art;
#[cfg(windows)]
mod updater;
mod controller_config;
mod wiimote;

const LIBRARY_SCAN_INTERVAL: Duration = Duration::from_secs(3);
const ARTWORK_RETRY_INTERVAL: Duration = Duration::from_secs(10 * 60);

struct DownloadedArtwork {
    title: String,
    kind: launchbox::ArtworkKind,
    image: library::Artwork,
    preferred: bool,
}

impl DownloadedArtwork {
    fn apply(&self, items: &mut [library::Item]) -> bool {
        let mut changed = false;
        for item in items.iter_mut().filter(|item| item.title == self.title) {
            let slot = match self.kind {
                launchbox::ArtworkKind::Cover => &mut item.cover,
                launchbox::ArtworkKind::Disc => &mut item.disc,
                launchbox::ArtworkKind::Logo => &mut item.logo,
                _ => continue,
            };
            if slot.is_none() || (self.preferred && self.kind == launchbox::ArtworkKind::Logo) {
                *slot = Some(self.image.clone());
                changed = true;
            }
        }
        changed
    }
}

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
  --mouse             Select the mouse settings page (all controls stay enabled)
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


fn write_state_file(path: &Path, m: &Machine, frontend: &[u8]) -> std::io::Result<usize> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut buf = Vec::new();
    m.save_state(frontend, &mut buf);
    let tmp = std::path::PathBuf::from(format!("{}.tmp", path.display()));
    std::fs::write(&tmp, &buf)?;
    std::fs::rename(&tmp, path)?;
    Ok(buf.len())
}

// Returns the frontend bytes; the machine is unchanged on any error
fn read_state_file(path: &Path, m: &mut Machine) -> Result<Vec<u8>, String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    m.load_state(&data).map(|fe| fe.to_vec()).map_err(|e: StateError| e.to_string())
}

fn board_nvram_path(machine: &Machine, custom: Option<&Path>) -> PathBuf {
    if let Some(path) = custom { return path.to_path_buf(); }
    let path = default_nvram_save_path();
    if machine.bus.mono4.is_some() { path.with_file_name("nvram_mono4.bin") } else { path }
}

fn flush_nvram(bus: &mut opencdi::bus::CdiBus) {
    bus.nvram.flush_if_dirty();
    if let Some(m4) = bus.mono4.as_deref_mut() {
        m4.nvram.flush_if_dirty();
    }
}

fn load_nvram(bus: &mut opencdi::bus::CdiBus, path: &Path) -> std::io::Result<()> {
    if let Some(board)=bus.mono4.as_deref_mut(){let mut nvram=opencdi::mono4::nvram::Nvram32::default();nvram.attach_file(path)?;board.nvram=nvram;}
    else{bus.nvram.load_or_initialize(path)?;}
    if let Some(now)=opencdi::rtc::local_now(){opencdi::rtc::set(bus,now);}Ok(())
}

fn send_pointer(bus: &mut opencdi::bus::CdiBus, x: i32, y: i32, buttons: u8) {
    opencdi::boot::send_shell_pointer(bus,x,y,buttons);
}

fn disc_program_started(pc: u32, frames: u64, shell_state: u32, shell_click_frames: u32) -> bool {
    opencdi::boot::disc_program_started(pc, frames, shell_state, shell_click_frames)
}

fn savestate_path_for(game_name: &str, slot: usize, bios: &str) -> PathBuf {
    let safe_name: String = game_name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' || c == '(' || c == ')' { c } else { '_' })
        .collect();
    let trimmed = safe_name.trim();
    let base = if trimmed.is_empty() { "cdi_bios" } else { trimmed };
    let dir = app_data_dir().join("Savestates").join(bios);
    dir.join(format!("{}_slot{}_v{}.st", base, slot, opencdi::savestate::VERSION))
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

fn per_disc_nvram_save_path(global_path: &Path, disc_path: &Path, bios: &str) -> PathBuf {
    let legacy = legacy_per_disc_nvram_save_path(global_path, disc_path);
    let mut name = disc_path.file_stem().unwrap_or_else(|| std::ffi::OsStr::new("disc")).to_os_string();
    name.push(".bin");
    let base=global_path.parent().filter(|p|!p.as_os_str().is_empty()).unwrap_or_else(||Path::new("."));
    let target=base.join("NVRAM").join(bios).join(&name);
    if target.is_file(){return target;}
    // The old shared file belongs only to the board whose RAM size matches.
    let size=if bios=="490"{32768}else{8192};
    for old in [legacy.with_file_name(&name),legacy] {
        if let Ok(bytes)=std::fs::read(&old) {
            if bytes.len()==size {
                if let Err(e)=copy_save_without_overwrite(&target,&bytes){log::warn!("Cannot migrate save {old:?}: {e}");return old;}
                break;
            }
        }
    }
    target
}
fn copy_save_without_overwrite(target:&Path,bytes:&[u8])->std::io::Result<()> {
    use std::io::Write;
    if let Some(parent)=target.parent(){std::fs::create_dir_all(parent)?;}
    let mut file=match std::fs::OpenOptions::new().write(true).create_new(true).open(target){Ok(f)=>f,Err(e) if e.kind()==std::io::ErrorKind::AlreadyExists=>return Ok(()),Err(e)=>return Err(e)};
    if let Err(e)=file.write_all(bytes).and_then(|()|file.sync_all()){drop(file);let _=std::fs::remove_file(target);return Err(e);}Ok(())
}
fn bios_storage_key(machine:&Machine)->&'static str {if machine.bus.mono4.is_some(){"490"}else{"220"}}

fn legacy_per_disc_nvram_save_path(global_path: &Path, disc_path: &Path) -> PathBuf {
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
    let canonical=relative_subpath.strip_prefix("bios/").map(|name|data_dir.join("BIOS").join(name));
    if let Some(path)=canonical.filter(|p|p.is_file()){return Some(path);}
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

fn find_dvc_rom(board: opencdi::bus::BoardKind) -> Option<PathBuf> {
    let names = if board == opencdi::bus::BoardKind::Mono4 {
        ["dvc/impega.rom", "bios/impega.rom", "impega.rom"]
    } else {
        ["dvc/vmpega.rom", "bios/vmpega.rom", "vmpega.rom"]
    };
    names.into_iter().find_map(|name| find_rom_file(name).map(|path| store_rom_in_app_data(path, name)))
}

fn store_rom_in_app_data(found: PathBuf, relative_name: &str) -> PathBuf {
    let data_dir = app_data_dir();
    if found.starts_with(&data_dir) { return found; }
    let relative_name=if let Some(rest)=relative_name.strip_prefix("bios/"){format!("BIOS/{rest}")}else{relative_name.to_owned()};
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
    #[cfg(windows)]
    if updater::run_helper() { return Ok(()); }
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
    if let Err(error)=opencdi::frontend::prepare_data_folders_at(&app_data_dir()){log::warn!("Cannot prepare BIOS folder: {error}");}
    let preferences = UiShortcuts::load();
    let saved_bios = preferences.bios_path.as_deref().map(|path| app_data_dir().join(path))
        .filter(|path| Machine::read_bios(path).is_ok_and(|(_, image)| image.report.status != opencdi::bios::Status::Error));
    let bios_path = match rom_arg.or(saved_bios).or_else(find_bios_rom) {
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
    let bios_model = match m.load_bios(&bios_path) {
        Ok(report) => {
            info!("Loaded system BIOS from {:?}: {}", bios_path, report.message);
            report.model().unwrap_or("Unknown CD-i BIOS").to_owned()
        }
        Err(e) => {
            error!("Cannot use system BIOS {:?}: {}", bios_path, e);
            eprintln!("Cannot use system BIOS {:?}: {}", bios_path, e);
            std::process::exit(1);
        }
    };

    // 2. Load NVRAM if present
    let nvram_arg = args.iter().position(|a| a == "--nvram").and_then(|i| args.get(i + 1)).map(PathBuf::from);
    let nvram_mode = preferences.nvram_mode;
    let resolved_nvram_path = board_nvram_path(&m, nvram_arg.as_deref());
    let mut current_nvram_path = resolved_nvram_path.clone();
    if let Err(error) = load_nvram(&mut m.bus, &resolved_nvram_path) {
        log::warn!("Could not load NVRAM from {resolved_nvram_path:?}: {error}");
    }

    // 2b. Load DVC ROM (Digital Video Cartridge) by default if present
    let dvc_disabled = has_flag("--no-dvc") || has_flag("--nodvc");
    let dvc_arg = flag_value("--dvc").map(PathBuf::from);
    if !dvc_disabled {
        if let Some(dvc_path) = dvc_arg.clone().or_else(|| find_dvc_rom(m.bus.board)) {
            if let Ok(_) = m.load_dvc(&dvc_path) {
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
                    current_nvram_path = per_disc_nvram_save_path(&resolved_nvram_path, &p, bios_storage_key(&m));
                    if let Err(e) = load_nvram(&mut m.bus, &current_nvram_path) {
                        log::warn!("Cannot load per-disc NVRAM {:?}: {}", current_nvram_path, e);
                    }
                }
                opencdi::frontend::add_recent_image(&p);
                current_game_name = opencdi::frontend::disc_display_name(&p);
                info!("Loaded game disc at startup: {}", current_game_name);

                let is_ntsc = m.bus.cdrom.as_ref().is_some_and(|cd| cd.is_ntsc_hint(&current_game_name).unwrap_or(false));
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
    if let (Some(path), Some(disc)) = (&active_disc_config_path, &current_disc_path) {
        bindings.load_disc_config_for(disc, path);
        bindings.save_disc_config(path);
    }
    let skip_shell = bindings.ui_shortcuts.skip_shell;
    let disc_paths = bindings.ui_shortcuts.disc_paths.iter().map(|p| resolve_config_path(p)).collect::<Vec<_>>();
    let managed_artwork_root = app_data_dir().join("Artwork");
    let synopsis_root = app_data_dir().join("Synopses");
    let boot_synopsis = current_disc_path.as_deref().and_then(|p| synopsis::load(&synopsis_root, p));
    let managed_covers_path = managed_artwork_root.join("Covers");
    let managed_cd_path = managed_artwork_root.join("CD");
    // Open the window first; the existing library worker discovers discs and
    // progressively fills artwork without decoding the entire collection here.
    if let Some(now)=opencdi::rtc::local_now(){opencdi::rtc::set(&mut m.bus,now);}
    let library_items = Vec::new();
    let mut library_search = library::Search::default();
    library_search.active_bios = Some((bios_model,bios_path.file_name().unwrap_or_default().to_string_lossy().into_owned()));
    library_search.collection.refresh(&library_items);
    let disc_details = current_disc_path.as_deref().map(|path| library::DiscDetails::load(path, &managed_artwork_root)).unwrap_or_default();
    let boot_disc_art = current_disc_path.as_deref().and_then(|p| library::load_disc_artwork(p, &managed_cd_path));
    let input_device_mode = if has_flag("--mouse") { InputDeviceMode::Mouse } else { bindings.ui_shortcuts.input_device };

    // 6. Initialize Host Audio Playback
    let audio_ring = Arc::new(Mutex::new(VecDeque::with_capacity(audio::RING_CAP)));
    let audio_gain = Arc::new(AtomicU32::new(1.0f32.to_bits()));
    let host_audio = HostAudioOutput::try_start(audio_ring.clone(), audio_gain.clone());

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
        peacekeeper_art: controller_config::load_peacekeeper_art(),
        wiimote: None,
        wiimote_auto: wiimote::AutoControl::default(),
        wiimote_cursor: None,
        wiimote_mouse_buttons: 0,
        wiimote_sample: None,
        wiimote_buttons: 0,
        wiimote_pressed: 0,
        wiimote_trigger_ready: false,
        gun_calibration: wiimote::Calibration::load(),
        gun_calibrating: None,
        gun_notice: String::new(),
        controller_config_was_captured: false,
        config_prev_up: false,
        config_prev_down: false,
        config_prev_left: false,
        config_prev_right: false,
        config_prev_confirm: false,
        skip_shell,
        input_device_mode,
        input_arbiter: opencdi::host_input::PointerArbiter::default(),
        physical_mouse_buttons: 0,
        mouse_activity: false,
        dvc_disabled,
        dvc_arg,
        max_frames,
        current_game_name,
        nvram_path: Some(current_nvram_path),
        global_nvram_path: resolved_nvram_path,
        custom_nvram_path: nvram_arg,
        current_disc_path,
        preview_started: None,
        boot_details_until: None,
        boot_fast_until: None,
        animation_clock: Instant::now(),
        rtc_last: Instant::now(),
        boot_scene: library::BootScene::default(),
        previous_play_pad: false,
        nvram_mode,
        window: None,
        surface: None,
        image_rect: (0, 0, SCREEN_WIDTH, SCREEN_HEIGHT),
        windowed: None,
        display_mode: DisplayMode::BorderlessFullscreen,
        restore_check: None,
        _host_audio: host_audio,
        audio_ring,
        audio_gain,
        audio_buf: Vec::new(),
        volume: 1.0,
        muted: false,
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
        artwork_progress_rx: None,
        artwork_progress: None,
        artwork_progress_at: Instant::now(),
        #[cfg(windows)]
        startup_update: updater::Startup::default(),
        artwork_during_scan: Vec::new(),
        artwork_retry_at: Instant::now(),
        library_scan_rx: None,
        optical_disc: None,
        library_scan_at: Instant::now(),
        library_reload_art: library_mode,
        library_selected: 0,
        library_search,
        disc_paths,
        managed_artwork_root,
        synopsis_root,
        boot_synopsis,
        next_synopsis_check: Instant::now(),
        managed_covers_path,
        managed_cd_path,
        boot_disc_art,
        disc_details,
        pause_info: false,
        previous_back_pad: false,
        library_click: None,
        library_scroll_accum: 0.0,
        library_size_dragging: false,
        library_prev_left: false,
        library_prev_right: false,
        library_prev_up: false,
        library_prev_down: false,
        library_prev_confirm: false,
    };

    info!("Emulation loop started. Shortcuts: [F11]/[Alt+Enter] Fullscreen, [F5]/[H] Reset, [P] Pause, [Space] hold Fast-forward, [F8] Screenshot (PPM), [F12] Screenshot (BMP), [M] Mute, [+]/[-] Volume, [Ctrl+I] Insert Disc, [Ctrl+O] Open & Reset, [Middle Click] Pause Menu, [Esc] Quit");

    let event_loop = EventLoop::new()?;
    event_loop.run_app(&mut app)?;

    #[cfg(target_os = "windows")]
    unsafe {
        timeEndPeriod(1);
    }

    flush_nvram(&mut app.m.bus);
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

fn resolve_config_path(path: &Path) -> PathBuf {
    if path.is_absolute() { return path.to_path_buf(); }
    std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join(path)))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default().join(path))
}

const BEZEL_WIDTH: usize = 1672;
const BEZEL_HEIGHT: usize = 941;

fn bezel_rect(width: usize, height: usize) -> (usize, usize, usize, usize) {
    let fitted_width = width.min(height.saturating_mul(BEZEL_WIDTH) / BEZEL_HEIGHT);
    let fitted_height = height.min(width.saturating_mul(BEZEL_HEIGHT) / BEZEL_WIDTH);
    ((width - fitted_width) / 2, (height - fitted_height) / 2, fitted_width, fitted_height)
}

fn bezel_game_rect(width: usize, height: usize) -> (usize, usize, usize, usize) {
    let (x, y, w, h) = bezel_rect(width, height);
    let opening_left = 226 * w / BEZEL_WIDTH;
    let opening_top = 28 * h / BEZEL_HEIGHT;
    let opening_width = 1446 * w / BEZEL_WIDTH - opening_left;
    let opening_height = 914 * h / BEZEL_HEIGHT - opening_top;
    let (gx, gy, gw, gh) = image_rect(opening_width, opening_height);
    (x + opening_left + gx, y + opening_top + gy, gw, gh)
}

fn draw_bezel(dst: &mut [u32], width: usize, height: usize, game: (usize, usize, usize, usize)) {
    static BEZEL: std::sync::LazyLock<library::Artwork> = std::sync::LazyLock::new(|| {
        library::load_png_bytes(include_bytes!("../assets/emulation_bezel.png"))
            .expect("embedded emulation bezel")
    });
    let (left, top, w, h) = bezel_rect(width, height);
    let (gx, gy, gw, gh) = game;
    for y in top..top + h {
        let row = &mut dst[y * width + left..y * width + left + w];
        let source_y = (y - top) * BEZEL.h / h;
        let draw_range = |row: &mut [u32], start: usize, end: usize| {
            for x in start..end {
                let pixel = BEZEL.pixels[source_y * BEZEL.w + x * BEZEL.w / w];
                let alpha = pixel >> 24;
                if alpha == 255 {
                    row[x] = pixel;
                } else if alpha != 0 {
                    row[x] = 0xff000000
                        | ((((pixel >> 16) & 255) * alpha / 255) << 16)
                        | ((((pixel >> 8) & 255) * alpha / 255) << 8)
                        | ((pixel & 255) * alpha / 255);
                }
            }
        };
        if (gy..gy + gh).contains(&y) {
            draw_range(row, 0, gx.saturating_sub(left).min(w));
            draw_range(row, (gx + gw).saturating_sub(left).min(w), w);
        } else {
            draw_range(row, 0, w);
        }
    }
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

struct LibraryScan {
    items: Option<Vec<library::Item>>,
    optical_disc: Option<char>,
    complete: bool,
}

struct App {
    m: Machine,
    bindings: KeyBindings,
    global_bindings: KeyBindings,
    active_disc_config_path: Option<PathBuf>,
    controller_config: Option<ControllerConfigState>,
    controller_art: Option<library::Artwork>,
    mouse_art: Option<library::Artwork>,
    peacekeeper_art: Option<library::Artwork>,
    wiimote: Option<wiimote::Wiimote>,
    wiimote_auto: wiimote::AutoControl,
    wiimote_cursor: Option<PhysicalPosition<f64>>,
    wiimote_mouse_buttons: u8,
    wiimote_sample: Option<wiimote::Sample>,
    wiimote_buttons: u16,
    wiimote_pressed: u16,
    wiimote_trigger_ready: bool,
    gun_calibration: wiimote::Calibration,
    gun_calibrating: Option<Option<(f64, f64)>>,
    gun_notice: String,
    controller_config_was_captured: bool,
    config_prev_up: bool,
    config_prev_down: bool,
    config_prev_left: bool,
    config_prev_right: bool,
    config_prev_confirm: bool,
    skip_shell: bool,
    input_device_mode: InputDeviceMode,
    input_arbiter: opencdi::host_input::PointerArbiter,
    physical_mouse_buttons: u8,
    mouse_activity: bool,
    dvc_disabled: bool,
    dvc_arg: Option<PathBuf>,
    max_frames: Option<u64>,
    current_game_name: String,
    nvram_path: Option<PathBuf>,
    global_nvram_path: PathBuf,
    custom_nvram_path: Option<PathBuf>,
    current_disc_path: Option<PathBuf>,
    preview_started: Option<Instant>,
    boot_details_until: Option<Instant>,
    boot_fast_until: Option<Instant>,
    animation_clock: Instant,
    rtc_last: Instant,
    boot_scene: library::BootScene,
    previous_play_pad: bool,
    nvram_mode: NvramMode,
    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    image_rect: (usize, usize, usize, usize),
    windowed: Option<(Option<PhysicalPosition<i32>>, PhysicalSize<u32>)>,
    display_mode: DisplayMode,
    restore_check: Option<Instant>,
    _host_audio: Option<HostAudioOutput>,
    audio_ring: Arc<Mutex<VecDeque<Frame>>>,
    audio_gain: Arc<AtomicU32>,
    audio_buf: Vec<Frame>,
    volume: f32,
    muted: bool,
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
    artwork_fetch_rx: Option<std::sync::mpsc::Receiver<DownloadedArtwork>>,
    artwork_progress_rx: Option<std::sync::mpsc::Receiver<launchbox::LookupProgress>>,
    artwork_progress: Option<launchbox::LookupProgress>,
    artwork_progress_at: Instant,
    #[cfg(windows)]
    startup_update: updater::Startup,
    artwork_during_scan: Vec<DownloadedArtwork>,
    artwork_retry_at: Instant,
    library_scan_rx: Option<std::sync::mpsc::Receiver<LibraryScan>>,
    optical_disc: Option<char>,
    library_scan_at: Instant,
    library_reload_art: bool,
    library_selected: usize,
    library_search: library::Search,
    disc_paths: Vec<PathBuf>,
    managed_artwork_root: PathBuf,
    synopsis_root: PathBuf,
    boot_synopsis: Option<String>,
    next_synopsis_check: Instant,
    managed_covers_path: PathBuf,
    managed_cd_path: PathBuf,
    boot_disc_art: Option<library::Artwork>,
    disc_details: library::DiscDetails,
    pause_info: bool,
    previous_back_pad: bool,
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
    fn frontend_state(&self) -> Vec<u8> {
        let mut w = Vec::new();
        (self.frame_count, self.disc_launched, self.shell_ready_count).save(&mut w);
        (self.pad_x, self.pad_y, self.prev_key_btn).save(&mut w);
        (self.pad_x_f, self.pad_y_f).save(&mut w);
        w
    }

    fn set_frontend_state(&mut self, data: &[u8]) -> Result<(), StateError> {
        let mut r = Reader::new(data);
        let mut a = (self.frame_count, self.disc_launched, self.shell_ready_count);
        let mut b = (self.pad_x, self.pad_y, self.prev_key_btn);
        let mut c = (self.pad_x_f, self.pad_y_f);
        a.load(&mut r)?;
        b.load(&mut r)?;
        c.load(&mut r)?;
        (self.frame_count, self.disc_launched, self.shell_ready_count) = a;
        (self.pad_x, self.pad_y, self.prev_key_btn) = b;
        (self.pad_x_f, self.pad_y_f) = c;
        Ok(())
    }

    fn state_slot(&mut self, slot: usize, load: bool) {
        let path = savestate_path_for(&self.current_game_name, slot, bios_storage_key(&self.m));
        if load {
            let canonical_old=app_data_dir().join("Savestates").join(path.file_name().unwrap());
            let old=if canonical_old.is_file(){canonical_old}else{migrate_app_data_dir("savestates").join(path.file_name().unwrap())};
            let source=if path.is_file(){path.clone()}else{old};
            match read_state_file(&source, &mut self.m) {
                Ok(frontend) => {
                    if let Err(error) = self.set_frontend_state(&frontend) {
                        error!("Could not restore frontend state: {error}");
                    }
                    if let Ok(mut ring) = self.audio_ring.lock() { ring.clear(); }
                    if source!=path {if let Ok(bytes)=std::fs::read(&source){let _=copy_save_without_overwrite(&path,&bytes);}}
                    self.rtc_last=Instant::now();
                    self.reset_telemetry();
                    info!("Loaded state slot {slot} from {path:?}");
                }
                Err(error) => error!("Could not load state slot {slot}: {error}"),
            }
        } else {
            if let Some(parent)=path.parent(){let _=std::fs::create_dir_all(parent);}
            match write_state_file(&path, &self.m, &self.frontend_state()) {
                Ok(_) => info!("Saved state slot {slot} to {path:?}"),
                Err(error) => error!("Could not save state slot {slot}: {error}"),
            }
        }
    }

    fn showing_disc_details(&self) -> bool {
        self.pause_info || self.preview_started.is_some()
            || self.boot_details_until.is_some_and(|until| Instant::now() < until)
            || (self.skip_shell && self.m.bus.cdrom.is_some() && !self.disc_launched)
    }

    fn content_rect(&self, width: usize, height: usize) -> (usize, usize, usize, usize) {
        if self.library_mode || self.controller_config.is_some() {
            (0, 0, width, height)
        } else if self.showing_disc_details() {
            image_rect(width, height)
        } else {
            bezel_game_rect(width, height)
        }
    }

    fn toggle_menu(&mut self) {
        if !self.library_mode || self.controller_config.is_some() { return; }
        self.menu_visible = !self.menu_visible;
        if let Some(window) = &self.window { window.request_redraw(); }
    }

    fn pause_is_available(&self) -> bool {
        opencdi::host_input::pause_action(self.library_mode,self.preview_started.is_some(),self.current_disc_path.is_some(),self.disc_launched)
            != opencdi::host_input::PauseAction::None
    }

    fn toggle_pause_menu(&mut self) {
        if opencdi::host_input::pause_action(self.library_mode,self.preview_started.is_some(),self.current_disc_path.is_some(),self.disc_launched)
            == opencdi::host_input::PauseAction::ReturnToLibrary {
            self.stop_to_library();
            return;
        }
        if !self.pause_is_available() && !self.paused { return; }
        self.paused = !self.paused;
        self.pause_info = false;
        self.apply_grab();
        if self.paused { self.pause_menu_selected = 0; }
        self.prev_pause_confirm = false;
        self.reset_telemetry();
        self.refresh_title();
        if let Some(window) = &self.window { window.request_redraw(); }
    }

    fn move_pause_selection(&mut self, delta: isize) {
        self.pause_menu_selected = (self.pause_menu_selected as isize + delta).rem_euclid(library::PAUSE_ACTIONS.len() as isize) as usize;
        if let Some(window) = &self.window { window.request_redraw(); }
    }

    fn choose_pause_action(&mut self) {
        match self.pause_menu_selected {
            0 => self.toggle_pause_menu(),
            1 => self.pause_info = true,
            2 => { push_action(GuiAction::QuickSaveState); self.paused = false; },
            3 => { push_action(GuiAction::QuickLoadState); self.paused = false; },
            4 => {
                self.controller_config_was_captured = self.mouse_captured;
                self.controller_config = Some(controller_config::State::default());
                self.mouse_captured = false;
                send_pointer(&mut self.m.bus, self.pad_x, self.pad_y, 0);
                self.prev_key_btn = 0;
                self.apply_grab();
            }
            5 => self.stop_to_library(),
            6 => self.exit_requested = true,
            _ => {}
        }
        if !self.paused && !self.library_mode {
            // A menu confirmation belongs to the menu, not the first game frame.
            self.wiimote_auto.suppress_held(self.wiimote_buttons);
            self.wiimote_buttons = 0;
            self.wiimote_pressed = 0;
            self.wiimote_trigger_ready = false;
            self.mouse_left = false;
            self.mouse_right = false;
        }
        self.reset_telemetry();
        if let Some(window) = &self.window { window.request_redraw(); }
    }

    fn stop_to_library(&mut self) {
        self.boot_fast_until = None;
        self.save_active_disc_settings();
        self.paused = false;
        self.pause_info = false;
        self.toolbar_popup = None;
        self.library_mode = true;
        self.preview_started = None;
        self.boot_details_until = None;
        self.library_scan_at = Instant::now();
        // The South/A button used to confirm this action may still be held on the
        // next library tick. Consume that edge so it cannot relaunch the selected disc.
        self.library_prev_confirm = true;
        self.mouse_captured = false;
        self.m.bus.slave.set_disc_inserted(false, false);
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

    fn go_back(&mut self) -> bool {
        if let Some(config) = &self.controller_config {
            // A button being assigned must still be captured by the mapping editor.
            if config.listening { return false; }
            self.controller_config = None;
            self.mouse_captured = self.controller_config_was_captured;
            self.apply_grab();
        } else if self.pause_info {
            self.pause_info = false;
        } else if self.paused {
            self.toggle_pause_menu();
        } else if self.preview_started.is_some() {
            self.stop_to_library();
        } else if !self.library_mode && self.current_disc_path.is_none() {
            self.stop_to_library();
        } else if !self.library_mode {
            return false;
        } else if self.toolbar_popup.take().is_none() {
            if self.library_search.collection.missing_offset.take().is_some() {
                // Closing the list restores the current category and selection.
            } else if self.library_search.focused {
                self.set_search_focus(false);
            } else if !self.library_search.query.is_empty() {
                self.library_search.clear();
                self.search_changed();
            } else {
                return false;
            }
        }
        if let Some(window) = &self.window { window.request_redraw(); }
        true
    }

    fn on_cursor_moved(&mut self, position: PhysicalPosition<f64>) {
                self.cursor = Some(position);
                self.library_search.pointer = Some(cursor_to_cdi(position, self.image_rect));
                if self.paused && !self.pause_info && self.controller_config.is_none()
                    && let Some(window) = &self.window {
                    let size=window.inner_size();
                    if let Some(row)=library::pause_menu_hit_full(position.x.max(0.0) as usize,position.y.max(0.0) as usize,size.width as usize,size.height as usize) {
                        self.pause_menu_selected=row;
                        window.request_redraw();
                    }
                }
                if self.library_size_dragging {
                    let (x, y) = cursor_to_cdi(position, self.image_rect);
                    if (library::H as i32-58..library::H as i32-12).contains(&y) {
                        self.set_artwork_size(library::size_from_slider_x(x.max(0) as usize));
                    }
                }
    }

    fn on_mouse_input(&mut self, state: ElementState, button: MouseButton) {
        let mask = match button { MouseButton::Left => 1, MouseButton::Right => 2, _ => 0 };
        let before = self.physical_mouse_buttons | self.wiimote_mouse_buttons;
        if state == ElementState::Pressed { self.physical_mouse_buttons |= mask; }
        else { self.physical_mouse_buttons &= !mask; }
        if mask != 0 { self.mouse_activity = true; }
        let after = self.physical_mouse_buttons | self.wiimote_mouse_buttons;
        if mask != 0 && (before ^ after) & mask == 0 { return; }
        self.on_pointer_input(state, button, true);
    }

    fn on_pointer_input(&mut self, state: ElementState, button: MouseButton, single_press_details: bool) {
                let down = state == ElementState::Pressed;
                if self.gun_calibrating.is_some() { return; }
                if self.controller_config.is_some() && self.input_device_mode == InputDeviceMode::Peacekeeper
                    && down && button == MouseButton::Left && let Some(pos) = self.cursor {
                    let (x, y) = cursor_to_cdi(pos, self.image_rect);
                    if (390..730).contains(&x) && (320..350).contains(&y) {
                        self.gun_calibrating = Some(None);
                        self.gun_notice.clear();
                        return;
                    }
                }
                if self.toolbar_popup == Some(library::ToolbarPopup::About) {
                    if down && button == MouseButton::Left && self.cursor.is_some_and(|pos| {
                        let (x, y) = cursor_to_cdi(pos, self.image_rect);
                        x >= 0 && y >= 0 && library::about_close_hit(x as usize, y as usize)
                    }) {
                        self.toolbar_popup = None;
                        if let Some(window) = &self.window { window.request_redraw(); }
                    }
                    self.mouse_left = false;
                    return;
                }
                if down && button == MouseButton::Middle && self.controller_config.is_none()
                    && self.pause_is_available() && !self.paused {
                    self.toggle_pause_menu();
                    return;
                }
                if down && button == MouseButton::Right && self.paused && self.controller_config.is_none() {
                    if self.pause_info { self.pause_info = false; }
                    self.toggle_pause_menu();
                    return;
                }
                if self.pause_info { return; }
                if self.preview_started.is_some() && self.controller_config.is_none() {
                    if down && button == MouseButton::Right {
                        self.stop_to_library();
                    } else if down && button == MouseButton::Left && let Some(pos) = self.cursor {
                        let (x, y) = cursor_to_cdi(pos, self.image_rect);
                        match library::preview_hit(x, y) {
                            Some(library::PreviewAction::Play) => push_action(GuiAction::PlaySelectedDisc),
                            Some(library::PreviewAction::Back) => self.stop_to_library(),
                            None => {}
                        }
                    }
                    return;
                }
                if down && button == MouseButton::Left && self.paused && self.controller_config.is_none() {
                    if let Some(pos) = self.cursor {
                        let (width, height) = self.window.as_ref().map(|w| { let s = w.inner_size(); (s.width as usize, s.height as usize) }).unwrap_or((0, 0));
                        if let Some(row) = library::pause_menu_hit_full(pos.x.max(0.0) as usize, pos.y.max(0.0) as usize, width, height) {
                            self.pause_menu_selected = row;
                            self.choose_pause_action();
                            self.mouse_left = self.paused;
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
                        let on_popup = library::toolbar_popup_hit(px, py, self.toolbar_popup);
                        if on_toolbar || on_popup {
                            self.set_search_focus(false);
                            if let Some(action) = library::toolbar_hit(px, py, self.toolbar_popup, self.optical_disc) { self.handle_toolbar_action(action); }
                            else { self.toolbar_popup = None; }
                            self.mouse_left = true;
                            return;
                        }
                    }
                }
                if down && button == MouseButton::Left && self.library_mode && self.controller_config.is_none() {
                    if let Some(pos) = self.cursor {
                        let (x, y) = cursor_to_cdi(pos, self.image_rect);
                        if let Some(category) = library::category_hit(x, y) {
                            self.select_library_category(category);
                            self.mouse_left = true;
                            return;
                        }
                        if library::collection_count_hit(x, y) || (self.library_search.collection.missing_offset.is_some()
                            && library::missing_close_hit(x, y))
                        {
                            self.toggle_missing_titles();
                            self.mouse_left = true;
                            return;
                        }
                        #[cfg(windows)]
                        if let Some(offset) = self.library_search.collection.missing_offset
                            && let Some(row) = library::missing_row_hit(x, y)
                            && let Some(title) = self.library_search.collection.missing_title(offset + row)
                        {
                            gui::open_web_url(&title.url());
                            self.mouse_left = true;
                            return;
                        }
                    }
                    let hit = self.cursor.and_then(|pos| {
                        let (x, y) = cursor_to_cdi(pos, self.image_rect);
                        library::search_hit(x, y)
                    });
                    if let Some(clear) = hit {
                        self.toolbar_popup = None;
                        self.set_search_focus(true);
                        self.library_search.caret = self.library_search.query.len();
                        if clear && !self.library_search.query.is_empty() {
                            self.library_search.clear();
                            self.search_changed();
                        }
                        self.mouse_left = true;
                        return;
                    }
                    if self.library_search.focused { self.set_search_focus(false); }
                }
                if self.controller_config.is_some() {
                    if down && button == MouseButton::Left {
                        if let Some(pos) = self.cursor {
                            let (x, y) = cursor_to_cdi(pos, self.image_rect);
                            if let Some(hit) = controller_config::hit_test(x as usize, y as usize) {
                                if self.input_device_mode == InputDeviceMode::Peacekeeper
                                    && matches!(hit, ControllerConfigHit::Binding(_, _) | ControllerConfigHit::Hotkey(_)) { return; }
                                match hit {
                                    ControllerConfigHit::Binding(_, _) | ControllerConfigHit::Hotkey(_) => {
                                        if let Some(config) = self.controller_config.as_mut() { config.click(hit); }
                                    }
                                    ControllerConfigHit::Device(mode) => {
                                        if let Some(config) = self.controller_config.as_mut() { config.listening = false; }
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
                        if self.library_search.collection.missing_offset.is_none()
                            && let Some(slot) = library::slot_at(x.max(0) as usize, y.max(0) as usize, self.bindings.ui_shortcuts.artwork_size)
                        {
                            let per_page = library::layout(self.bindings.ui_shortcuts.artwork_size).visible_count();
                            let index = (self.library_selected / per_page) * per_page + slot;
                            if index < self.library_search.len(&self.library_items) {
                                if single_press_details || (self.library_selected == index && self.library_click.is_some_and(|(prev, t)| prev == index && t.elapsed() < Duration::from_millis(500))) {
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

    fn update_wiimote(&mut self) {
        let enabled = self.host_input;
        if enabled && self.wiimote.is_none() { self.wiimote = Some(wiimote::Wiimote::new()); }
        if !enabled {
            self.wiimote = None;
            self.gun_calibrating = None;
        }
        self.wiimote_sample = self.wiimote.as_ref().and_then(|w| w.sample());
        if !self.focused {
            self.wiimote_auto.suppress_held(self.wiimote_sample.map_or(0, |s| s.buttons));
        }
        let filtered = self.wiimote_auto.update(self.wiimote_sample, Instant::now());
        let buttons = if self.focused { filtered } else { 0 };
        self.wiimote_pressed = buttons & !self.wiimote_buttons;
        self.wiimote_buttons = buttons;
        if buttons & (wiimote::PLUS | wiimote::MINUS) == (wiimote::PLUS | wiimote::MINUS) {
            self.exit_requested = true;
            self.wiimote_buttons = 0;
            self.wiimote_pressed = 0;
            return;
        }
        let aim = self.wiimote_sample.and_then(|s| s.aim);
        let inside_picture = self.wiimote_position().is_some_and(|pos| {
            let (x,y,w,h) = self.image_rect;
            pos.x >= x as f64 && pos.y >= y as f64 && pos.x < (x+w) as f64 && pos.y < (y+h) as f64
        });
        if !self.focused || !inside_picture
            || self.controller_config.is_some() {
            self.wiimote_trigger_ready = false;
        } else if buttons & (wiimote::A | wiimote::B) == 0 {
            self.wiimote_trigger_ready = true;
        }
        if self.controller_config.is_none() { self.gun_calibrating = None; }
        let calibrating = self.gun_calibrating.is_some();
        if self.gun_calibrating.is_some() && self.wiimote_pressed & wiimote::B != 0
            && let Some(aim) = aim {
            match self.gun_calibrating {
                Some(None) => self.gun_calibrating = Some(Some(aim)),
                Some(Some(first)) => {
                    let calibration = wiimote::Calibration { first, second: aim };
                    if calibration.valid() {
                        self.gun_calibration = calibration;
                        self.gun_notice = match calibration.save() {
                            Ok(()) => "Calibration saved".into(),
                            Err(_) => "Calibration active; settings could not be saved".into(),
                        };
                        self.gun_calibrating = None;
                    } else {
                        self.gun_notice = "Targets too close. Press C to try again.".into();
                        self.gun_calibrating = None;
                    }
                }
                None => {}
            }
        }
        if calibrating {
            // A calibration press belongs only to the target capture, including
            // the tick that completes calibration and returns to the settings page.
            self.wiimote_auto.suppress_held(self.wiimote_buttons);
            self.wiimote_buttons = 0;
            self.wiimote_pressed = 0;
        }
    }

    fn wiimote_position(&self) -> Option<PhysicalPosition<f64>> {
        let aim = self.wiimote_sample?.aim.and_then(|p| self.gun_calibration.map(p))?;
        let size = self.window.as_ref()?.inner_size();
        Some(PhysicalPosition::new(aim.0 * size.width.saturating_sub(1) as f64,
            aim.1 * size.height.saturating_sub(1) as f64))
    }

    fn update_wiimote_mouse(&mut self) {
        self.input_arbiter.remote_mode(self.wiimote_auto.mode == wiimote::ControlMode::Pointer);
        let remote = (self.wiimote_auto.mode == wiimote::ControlMode::Pointer)
            .then(|| self.wiimote_position()).flatten()
            .map(|p| cursor_to_cdi(p,self.image_rect));
        self.input_arbiter.pointers(std::mem::take(&mut self.mouse_activity),remote,
            self.wiimote_pressed & (wiimote::A | wiimote::B) != 0);
        let ui = self.library_mode || self.preview_started.is_some() || self.controller_config.is_some() || self.paused;
        let previous_cursor = self.wiimote_cursor;
        self.wiimote_cursor = None;
        let mut buttons = 0;
        if self.focused && self.wiimote_auto.mode == wiimote::ControlMode::Pointer
            && self.input_arbiter.source() == opencdi::host_input::PointerSource::Wiimote
            && let Some(position) = self.wiimote_position() {
            self.wiimote_cursor = Some(position);
            if ui {
                self.on_cursor_moved(position);
                if self.wiimote_buttons & wiimote::A != 0 { buttons |= 1; }
                if self.wiimote_buttons & wiimote::B != 0 { buttons |= 2; }
            }
        }
        let previous = self.wiimote_mouse_buttons;
        #[cfg(windows)]
        if let Some(remote) = &self.wiimote {
            let target = (self.host_input && self.focused && self.wiimote_cursor.is_some())
                .then(|| self.hwnd() as isize);
            remote.set_pointer_target(target, self.gun_calibration);
        }
        self.wiimote_mouse_buttons = buttons;
        for (mask,button) in [(1,MouseButton::Left),(2,MouseButton::Right)] {
            if ((previous | self.physical_mouse_buttons) ^ (buttons | self.physical_mouse_buttons)) & mask != 0 {
                self.on_pointer_input(if buttons & mask != 0 { ElementState::Pressed } else { ElementState::Released }, button, true);
            }
        }
        self.apply_grab();
        if self.paused && previous_cursor != self.wiimote_cursor
            && let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn update_pause_input(&mut self) {
        let mut guide = self.bindings.ui_shortcuts.pause_pad.contains(&gilrs::Button::Mode) && xbox_guide_pressed();
        let mut exit = false;
        let mut up = false; let mut down = false; let mut confirm = false;
        let mut back = false;
        guide |= self.wiimote_buttons & wiimote::HOME != 0;
        let (_,remote_y) = self.wiimote_auto.directions(self.wiimote_buttons);
        up |= remote_y < 0;
        down |= remote_y > 0;
        confirm |= self.wiimote_buttons & wiimote::ONE != 0
            || self.wiimote_auto.mode == wiimote::ControlMode::Sideways && self.wiimote_buttons & wiimote::A != 0;
        back |= self.wiimote_buttons & (wiimote::B | wiimote::TWO | wiimote::MINUS) != 0;
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
                back |= pad.is_pressed(self.bindings.pad_btn2);
            }
        }
        let back_pressed = back && !self.previous_back_pad;
        self.previous_back_pad = back;
        let pause_pressed = guide && !self.prev_pause_pad;
        self.prev_pause_pad = guide;
        if back_pressed && self.go_back() { return; }
        if pause_pressed && !self.pause_info { self.toggle_pause_menu(); }
        if exit { self.exit_requested = true; }
        if !self.paused { return; }
        if !self.pause_info {
            if up && !self.prev_pause_up { self.move_pause_selection(-1); }
            if down && !self.prev_pause_down { self.move_pause_selection(1); }
            if confirm && !self.prev_pause_confirm { self.choose_pause_action(); }
        }
        self.prev_pause_up = up; self.prev_pause_down = down; self.prev_pause_confirm = confirm;
    }

    fn handle_toolbar_action(&mut self, action: library::ToolbarAction) {
        use library::ToolbarAction as T;
        match action {
            T::OpenDisc => self.open_dialog_action(true),
            T::PlayDisc(letter) => {
                self.toolbar_popup = None;
                push_action(GuiAction::OpenDisc(PathBuf::from(format!("{letter}:"))));
            }
            T::Start220 | T::Start490 => {
                let target=if action==T::Start490{"490"}else{"220"};
                self.handle_toolbar_action(if action==T::Start490{T::Bios490}else{T::Bios220});
                if bios_storage_key(&self.m)!=target{return;}
                self.stop_to_library();
                self.m.bus.reset_system();self.m.reset();self.reset_state();
                self.library_mode=false;self.mouse_captured=true;
                self.current_game_name=format!("CD-i {target} System");
                self.grab_applied=None;self.apply_grab();self.reset_telemetry();
            }
            T::Controller => push_action(GuiAction::OpenControllerConfig),
            T::About => self.toolbar_popup = Some(library::ToolbarPopup::About),
            T::Settings => self.toolbar_popup = if self.toolbar_popup == Some(library::ToolbarPopup::Settings) { None } else { Some(library::ToolbarPopup::Settings) },
            T::Exit => { self.toolbar_popup = None; push_action(GuiAction::Exit); }
            T::Borderless => { self.toolbar_popup = None; self.set_display_mode(DisplayMode::BorderlessFullscreen); }
            T::Decorated => { self.toolbar_popup = None; self.set_display_mode(DisplayMode::DecoratedFullscreen); }
            T::Windowed => { self.toolbar_popup = None; self.set_display_mode(DisplayMode::Windowed); }
            T::AddDiscFolder => {
                self.toolbar_popup = None;
                #[cfg(windows)]
                {
                    let hwnd = self.hwnd();
                    if let Some(path) = gui::modal(hwnd, || gui::select_folder_dialog(hwnd, "Choose a folder to scan for disc images")) {
                        push_action(GuiAction::AddDiscPath(path));
                    }
                }
            }
            T::Bios220 | T::Bios490 => {
                self.toolbar_popup = None;
                let model=if action==T::Bios490 { "490" } else { "220" };
                if self.library_search.active_bios.as_ref().is_some_and(|(current,_)| current.contains(model)) { return; }
                let names: &[&str] = if model=="490" { &["bios/cdi490a.rom","cdi490a.rom"] }
                    else { &["bios/cdi220b.rom","cdi220b.rom","bios/cdi220.rom","cdi220.rom"] };
                if let Some(path)=names.iter().find_map(|name| find_rom_file(name)) {
                    self.switch_bios(path);
                } else {
                    #[cfg(windows)]
                    gui::show_disc_error(self.hwnd(),&format!("BIOS {model} was not found in the BIOS folder."));
                }
            }
        }
        if let Some(window) = &self.window { window.request_redraw(); }
    }

    fn launch_library_selection(&mut self) {
        if self.library_search.collection.missing_offset.is_some() { return; }
        if let Some(item) = self.library_search.get(&self.library_items, self.library_selected) {
            push_action(GuiAction::OpenDisc(item.path.clone()));
        }
    }

    fn preview_disc(&mut self, path: PathBuf) {
        if !self.library_mode && self.preview_started.is_none() {
            self.stop_to_library();
        }
        self.set_search_focus(false);
        self.library_mode = false;
        self.toolbar_popup = None;
        if let Ok(mut ring) = self.audio_ring.lock() { ring.clear(); }
        self.preview_started = Some(Instant::now());
        self.boot_details_until = None;
        self.mouse_left = false;
        self.mouse_right = false;
        self.mouse_middle = false;
        self.mouse_captured = false;
        self.apply_grab();
        self.disc_details = library::DiscDetails::load(&path, &self.managed_artwork_root);
        self.boot_synopsis = synopsis::load(&self.synopsis_root, &path);
        self.boot_disc_art = library::load_disc_artwork(&path, &self.managed_cd_path);
        self.current_game_name = opencdi::frontend::disc_display_name(&path);
        self.current_disc_path = Some(path);
        self.next_synopsis_check = Instant::now();
        self.artwork_retry_at = self.next_synopsis_check;
        self.refresh_title();
        if let Some(window) = &self.window { window.request_redraw(); }
    }

    fn switch_bios(&mut self, path: PathBuf) {
        let result = Machine::read_bios(&path).and_then(|(_, image)| {
            if image.report.status == opencdi::bios::Status::Error {
                Err(std::io::Error::new(std::io::ErrorKind::InvalidData, image.report.message.clone()))
            } else {
                Ok(image)
            }
        });
        let image = match result {
            Ok(image) => image,
            Err(error) => {
                log::warn!("Cannot use BIOS {path:?}: {error}");
                #[cfg(windows)]
                gui::show_disc_error(self.hwnd(), &format!("Cannot use this system BIOS:\n\n{error}"));
                return;
            }
        };
        if !self.library_mode { self.stop_to_library(); }
        flush_nvram(&mut self.m.bus);
        let previous = self.m.bus.board;
        self.m.load_bios_image(&image);
        if previous != self.m.bus.board {
            self.m.bus.dvc = opencdi::dvc::Dvc::new();
            if !self.dvc_disabled
                && let Some(rom) = self.dvc_arg.clone().or_else(|| find_dvc_rom(self.m.bus.board))
            {
                if let Err(error) = self.m.load_dvc(rom) { log::warn!("Cannot load DVC for this BIOS: {error}"); }
            }
        }
        self.global_nvram_path = board_nvram_path(&self.m, self.custom_nvram_path.as_deref());
        self.nvram_path = None;
        self.activate_nvram_for_selection();
        self.m.reset();
        self.reset_state();
        let data_dir = app_data_dir();
        let saved = path.strip_prefix(&data_dir).unwrap_or(&path).to_path_buf();
        self.bindings.ui_shortcuts.bios_path = Some(saved.clone());
        self.global_bindings.ui_shortcuts.bios_path = Some(saved);
        self.library_search.active_bios = Some((image.report.model().unwrap_or("Unknown CD-i BIOS").to_owned(),path.file_name().unwrap_or_default().to_string_lossy().into_owned()));
        self.bindings.save_to_ini();
        #[cfg(windows)]
        gui::update_dvc_menu(self.m.bus.dvc.is_fitted());
        #[cfg(windows)]
        gui::update_bios_menu(image.report.model().unwrap_or(""));
        info!("Selected system BIOS {:?}: {}", path, image.report.message);
        if let Some(window) = &self.window { window.request_redraw(); }
    }

    fn refresh_library(&mut self) {
        self.library_scan_rx = None;
        self.artwork_during_scan.clear();
        self.library_reload_art = true;
        self.library_scan_at = Instant::now();
        self.artwork_retry_at = self.library_scan_at;
    }

    fn update_library(&mut self, now: Instant) {
        use std::sync::mpsc::TryRecvError;

        if let Some(rx) = &self.artwork_progress_rx {
            while let Ok(progress) = rx.try_recv() {
                self.artwork_progress = Some(progress);
                self.artwork_progress_at = now;
                if let Some(window) = &self.window { window.request_redraw(); }
            }
        }
        if self.artwork_progress.as_ref().is_some_and(|progress| progress.finished)
            && now.duration_since(self.artwork_progress_at) > Duration::from_secs(15)
        {
            self.artwork_progress = None;
            if let Some(window) = &self.window { window.request_redraw(); }
        }
        let mut artwork_changed = false;
        while let Some(rx) = &self.artwork_fetch_rx {
            match rx.try_recv() {
                Ok(update) => {
                    artwork_changed |= update.apply(&mut self.library_items);
                    if update.preferred && update.kind == launchbox::ArtworkKind::Cover {
                        for item in self.library_items.iter_mut().filter(|item| item.title == update.title) {
                            item.cover = library::load_cover_artwork(&item.path, &self.managed_covers_path);
                            artwork_changed = true;
                        }
                    }
                    if self.current_disc_path.as_deref().is_some_and(|path| library::disc_title(path) == update.title) {
                        use launchbox::ArtworkKind::*;
                        let slot = match update.kind {
                            Disc => None,
                            Logo => Some(&mut self.disc_details.logo),
                            Boxart => Some(&mut self.disc_details.boxart),
                            Back => Some(&mut self.disc_details.back),
                            Cover => None,
                        };
                        if update.kind == Disc && let Some(path) = &self.current_disc_path {
                            self.boot_disc_art = library::load_disc_artwork(path, &self.managed_cd_path)
                                .or_else(|| Some(update.image.clone()));
                            artwork_changed = true;
                        } else if let Some(slot) = slot.filter(|slot| slot.is_none() || update.preferred || update.kind == Boxart) {
                            *slot = Some(update.image.clone());
                            artwork_changed = true;
                        }
                    }
                    if self.library_scan_rx.is_some() {
                        self.artwork_during_scan.push(update);
                    }
                }
                Err(TryRecvError::Disconnected) => self.artwork_fetch_rx = None,
                Err(TryRecvError::Empty) => break,
            }
        }
        if artwork_changed && let Some(window) = &self.window {
            window.request_redraw();
        }
        if let Some(rx) = &self.library_scan_rx {
            match rx.try_recv() {
                Ok(result) => {
                    if result.complete { self.library_scan_rx = None; }
                    if self.optical_disc != result.optical_disc {
                        self.optical_disc = result.optical_disc;
                        if let Some(window) = &self.window { window.request_redraw(); }
                    }
                    if let Some(mut items) = result.items {
                        for update in &self.artwork_during_scan {
                            update.apply(&mut items);
                        }
                        let selected_path = self.library_search.get(&self.library_items, self.library_selected)
                            .map(|item| item.path.clone());
                        let paths_changed = !self.library_items.iter().map(|item| &item.path).eq(items.iter().map(|item| &item.path));
                        if paths_changed {
                            self.artwork_retry_at = now;
                            self.library_click = None;
                        }
                        self.library_items = items;
                        if paths_changed {
                            self.library_search.collection.refresh(&self.library_items);
                            self.library_search.rebuild(&self.library_items);
                        }
                        self.library_selected = selected_path.and_then(|path| self.library_search.position(&self.library_items, &path))
                            .unwrap_or_else(|| self.library_selected.min(self.library_search.len(&self.library_items).saturating_sub(1)));
                        if let Some(window) = &self.window { window.request_redraw(); }
                    }
                    if result.complete { self.artwork_during_scan.clear(); }
                }
                Err(TryRecvError::Disconnected) => {
                    self.library_scan_rx = None;
                    self.artwork_during_scan.clear();
                    self.library_reload_art = true;
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        if !self.library_mode && !self.disc_launched && self.boot_synopsis.is_none()
            && now >= self.next_synopsis_check
        {
            self.next_synopsis_check = now + Duration::from_millis(500);
            self.boot_synopsis = self.current_disc_path.as_deref().and_then(|path| synopsis::load(&self.synopsis_root, path));
        }
        if now >= self.artwork_retry_at && self.artwork_fetch_rx.is_none()
            && (self.library_mode || self.current_disc_path.is_some())
            && self.library_scan_rx.is_none() && !self.library_reload_art
        {
            self.artwork_retry_at = now + ARTWORK_RETRY_INTERVAL;
            self.start_artwork_lookup();
        }
        if (self.library_mode || !self.library_items.is_empty())
            && now >= self.library_scan_at && self.library_scan_rx.is_none()
        {
            self.library_scan_at = now + LIBRARY_SCAN_INTERVAL;
            let roots = self.disc_paths.clone();
            let previous: Vec<_> = self.library_items.iter().map(|item| item.path.clone()).collect();
            let previous_items = self.library_items.clone();
            let previous_optical = self.optical_disc;
            let covers = self.managed_covers_path.clone();
            let discs = self.managed_cd_path.clone();
            let managed_artwork = self.managed_artwork_root.clone();
            let missing_logos: Vec<_> = self.library_items.iter().filter(|item| item.logo.is_none())
                .map(|item| item.path.clone()).collect();
            let force_reload = self.library_reload_art;
            let synopsis_root = self.synopsis_root.clone();
            let report_path = std::env::current_exe().ok().map(|exe| exe.with_file_name("Missing Media.txt"));
            let lookup_running = self.artwork_fetch_rx.is_some();
            let (tx, rx) = std::sync::mpsc::channel();
            if std::thread::Builder::new().name("disc-library-scan".into()).spawn(move || {
                let paths = library::scan_paths(&roots);
                library::migrate_artwork_names(&paths, &managed_artwork);
                let logo_added = missing_logos.iter().any(|path| {
                    library::managed_art_path(path, &managed_artwork, launchbox::ArtworkKind::Logo).is_some()
                });
                if force_reload || paths != previous || logo_added {
                    let mut items: Vec<_> = paths.iter().map(|path| {
                        previous_items.iter().find(|item| item.path == *path).cloned().unwrap_or_else(|| library::Item {
                            path: path.clone(), title: library::disc_title(path), cover: None, disc: None, logo: None,
                        })
                    }).collect();
                    // Titles are immediately usable, then covers arrive a page at a time.
                    if tx.send(LibraryScan { items: Some(items.clone()), optical_disc: previous_optical, complete: false }).is_err() { return; }
                    for start in (0..paths.len()).step_by(24) {
                        let end = (start + 24).min(paths.len());
                        let loaded = library::load_items(paths[start..end].to_vec(), &covers, &discs, &managed_artwork);
                        items[start..end].clone_from_slice(&loaded);
                        if tx.send(LibraryScan { items: Some(items.clone()), optical_disc: previous_optical, complete: false }).is_err() { return; }
                    }
                }
                if let Some(path) = report_path {
                    if let Err(error) = missing_media::refresh(&path, &paths, &managed_artwork, &synopsis_root, lookup_running) {
                        log::debug!("Could not write Missing Media.txt: {error}");
                    }
                }
                #[cfg(target_os = "windows")]
                let optical_disc = gui::get_optical_drives().into_iter().find(|&letter| opencdi::cdrom::optical_drive_has_disc(letter));
                #[cfg(not(target_os = "windows"))]
                let optical_disc = None;
                let _ = tx.send(LibraryScan { items: None, optical_disc, complete: true });
            }).is_ok() {
                self.library_scan_rx = Some(rx);
                self.library_reload_art = false;
            }
        }

    }

    fn start_artwork_lookup(&mut self) {
        let cache = self.managed_artwork_root.join("Cache");
        let needs_lookup = |target: &launchbox::FetchTarget| {
            !target.missing.is_empty() || target.synopsis_path.is_some()
                || !cache.join("LaunchBox-Media").join(format!("{}.done", launchbox::safe_name(&target.title))).is_file()
        };
        let target_for = |path: &Path, cover_missing: bool, disc_missing: bool, logo_missing: bool| {
            use launchbox::ArtworkKind::{self, *};
            let missing = ArtworkKind::ALL.into_iter().filter(|kind| match kind {
                Cover => cover_missing,
                Disc => disc_missing,
                Logo => logo_missing,
                Boxart | Back => library::managed_art_path(path, &self.managed_artwork_root, *kind).is_none(),
            }).collect();
            launchbox::FetchTarget {
                title: library::disc_title(path),
                missing,
                download_all: false,
                synopsis_path: synopsis::path(&self.synopsis_root, path).filter(|path| !path.exists()),
            }
        };
        let mut targets: Vec<_> = if self.library_mode {
            self.library_items.iter()
                .map(|item| target_for(&item.path, item.cover.is_none(), item.disc.is_none(), item.logo.is_none()))
                .filter(needs_lookup)
                .collect()
        } else if let Some(path) = &self.current_disc_path {
            if opencdi::cdrom::parse_drive_letter(path).is_some() { return; }
            let cover_missing = library::load_cover_artwork(path, &self.managed_covers_path).is_none();
            let target = target_for(path, cover_missing, self.boot_disc_art.is_none(), self.disc_details.logo.is_none());
            if !needs_lookup(&target) { return; }
            vec![target]
        } else {
            return;
        };
        if targets.is_empty() { return; }
        let mut seen_titles = HashSet::new();
        targets.retain(|target| seen_titles.insert(target.title.clone()));
        if self.library_mode {
            let per_page = library::layout(self.bindings.ui_shortcuts.artwork_size).visible_count();
            let start = self.library_selected / per_page * per_page;
            let visible: HashSet<_> = (start..start + per_page)
                .filter_map(|index| self.library_search.get(&self.library_items, index))
                .map(|item| item.title.clone()).collect();
            targets.sort_by_key(|target| !visible.contains(&target.title));
        }
        let (progress_tx, progress_rx) = std::sync::mpsc::channel();
        self.artwork_progress_rx = Some(progress_rx);
        self.artwork_progress = Some(launchbox::LookupProgress {
            phase: "Preparing artwork lookup", total: targets.len(), ..Default::default()
        });
        let existing_covers: Vec<_> = targets.iter().filter_map(|target| {
            let disc = self.library_items.iter().find(|item| item.title == target.title).map(|item| item.path.as_path())
                .or(self.current_disc_path.as_deref())?;
            library::cover_artwork_path(disc, &self.managed_covers_path)
                .map(|cover| (target.title.clone(), cover))
        }).collect();
        let artwork_root = self.managed_artwork_root.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.artwork_fetch_rx = Some(rx);
        let result = std::thread::Builder::new().name("launchbox-artwork".into()).spawn(move || {
            let send_image = |title: &str, kind, path: &Path| {
                if let Some(image) = library::load_artwork(path) {
                    let preferred = path.file_stem().is_some_and(|stem| stem.to_string_lossy().ends_with("--screenscraper")
                        || stem.to_string_lossy().ends_with("--screenscraper-disc"));
                    let _ = tx.send(DownloadedArtwork { title: title.to_owned(), kind, image, preferred });
                }
            };
            for (title, cover) in existing_covers {
                match boxart::ensure(&cover, &title, &artwork_root) {
                    Ok(Some(path)) => {
                        send_image(&title, launchbox::ArtworkKind::Cover, &cover);
                        send_image(&title, launchbox::ArtworkKind::Boxart, &path);
                    }
                    Err(error) => log::warn!("Could not generate box art for {title}: {error}"),
                    _ => {}
                }
            }
            let image_ready = |title: &str, kind, path: &Path| {
                send_image(title, kind, path);
                if kind == launchbox::ArtworkKind::Cover {
                    match boxart::ensure(path, title, &artwork_root) {
                        Ok(Some(path)) => send_image(title, launchbox::ArtworkKind::Boxart, &path),
                        Err(error) => log::warn!("Could not generate box art for {title}: {error}"),
                        _ => {}
                    }
                }
            };
            launchbox::fetch_missing_artwork(&targets, &artwork_root, &cache, image_ready, |pending, kinds, disc_region, notify| {
                let mut supplied = HashSet::new();
                screenscraper::fetch_missing_for_kinds(pending, &artwork_root, kinds, disc_region, |title, kind, path| {
                    supplied.insert((title.to_owned(), kind));
                    notify(title, kind, path);
                }, synopsis::save);
                for target in pending {
                    target.missing.retain(|&kind| !supplied.contains(&(target.title.clone(), kind)));
                }
            }, |progress| { let _ = progress_tx.send(progress); });
        });
        if result.is_err() {
            self.artwork_fetch_rx = None;
            self.artwork_progress_rx = None;
            self.artwork_progress = None;
        }
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
        self.global_bindings.ui_shortcuts.disc_paths = self.disc_paths.clone();
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

    fn save_global_shortcuts(&mut self) {
        let input_device = self.global_bindings.ui_shortcuts.input_device;
        self.global_bindings.ui_shortcuts = self.bindings.ui_shortcuts.clone();
        self.global_bindings.ui_shortcuts.input_device = input_device;
        self.global_bindings.save_to_ini();
    }

    fn save_active_disc_settings(&self) {
        if let Some(path) = &self.active_disc_config_path { self.bindings.save_disc_config(path); }
    }

    fn activate_disc_settings(&mut self, path: &Path) {
        self.bindings = self.global_bindings.clone();
        let config = disc_config_path(path);
        self.bindings.load_disc_config_for(path, &config);
        self.bindings.save_disc_config(&config);
        self.input_device_mode = self.bindings.ui_shortcuts.input_device;
        self.active_disc_config_path = Some(config);
    }

    fn move_library_selection(&mut self, delta: isize) {
        if self.library_search.collection.missing_offset.is_some() {
            self.library_search.collection.scroll_missing(delta);
            if let Some(window) = &self.window { window.request_redraw(); }
            return;
        }
        let n = self.library_search.len(&self.library_items) as isize;
        if n == 0 { return; }
        self.library_selected = (self.library_selected as isize + delta).rem_euclid(n) as usize;
    }

    fn activate_nvram_for_selection(&mut self) {
        let target = if self.nvram_mode == NvramMode::PerDisc {
            self.current_disc_path.as_deref().map(|p| per_disc_nvram_save_path(&self.global_nvram_path, p, bios_storage_key(&self.m)))
                .unwrap_or_else(|| self.global_nvram_path.clone())
        } else { self.global_nvram_path.clone() };
        if self.nvram_path.as_ref() == Some(&target) { return; }
        flush_nvram(&mut self.m.bus);
        match load_nvram(&mut self.m.bus, &target) {
            Ok(()) => {
                self.nvram_path = Some(target.clone());
                info!("NVRAM loaded from {:?}", target);
            }
            Err(e) => error!("Unable to activate NVRAM {:?}: {}", target, e),
        }
    }

    fn set_search_focus(&mut self, focused: bool) {
        if focused { self.library_search.collection.missing_offset = None; }
        self.library_search.focused = focused;
        self.library_search.select_all = false;
        if let Some(window) = &self.window {
            window.set_ime_allowed(focused);
            window.request_redraw();
        }
    }

    fn select_library_category(&mut self, category: library_catalogue::Category) {
        self.set_search_focus(false);
        if self.library_search.collection.category == category { return; }
        self.library_search.collection.category = category;
        if let Some(offset) = &mut self.library_search.collection.missing_offset { *offset = 0; }
        self.search_changed();
    }

    fn toggle_missing_titles(&mut self) {
        let collection = &mut self.library_search.collection;
        collection.missing_offset = collection.missing_offset.is_none().then_some(0);
        self.library_click = None;
        self.library_scroll_accum = 0.0;
        self.set_search_focus(false);
    }

    fn search_changed(&mut self) {
        self.library_search.rebuild(&self.library_items);
        self.library_selected = 0;
        self.library_click = None;
        self.library_scroll_accum = 0.0;
        if let Some(window) = &self.window { window.request_redraw(); }
    }

    fn search_key_event(&mut self, event: &winit::event::KeyEvent) -> bool {
        if !self.library_mode || self.controller_config.is_some() || self.toolbar_popup == Some(library::ToolbarPopup::About) {
            return false;
        }
        let code = match event.physical_key {
            PhysicalKey::Code(code) => Some(code),
            _ => None,
        };
        if event.state != ElementState::Pressed { return false; }
        if self.mods.control_key() && code == Some(Key::KeyF) {
            self.toolbar_popup = None;
            self.set_search_focus(true);
            self.library_search.caret = self.library_search.query.len();
            self.library_search.select_all = !self.library_search.query.is_empty();
            return true;
        }
        if !self.library_search.focused { return false; }
        let changed = match code {
            Some(Key::Escape) => {
                if self.library_search.query.is_empty() { self.set_search_focus(false); return true; }
                self.library_search.clear();
                true
            }
            Some(Key::Enter | Key::NumpadEnter | Key::Tab | Key::ArrowDown) => {
                self.set_search_focus(false);
                return true;
            }
            Some(Key::KeyA) if self.mods.control_key() => {
                self.library_search.select_all = !self.library_search.query.is_empty();
                false
            }
            Some(Key::Backspace) => self.library_search.delete(true),
            Some(Key::Delete) => self.library_search.delete(false),
            Some(Key::ArrowLeft | Key::ArrowRight) => {
                self.library_search.move_caret(code == Some(Key::ArrowRight));
                false
            }
            Some(Key::Home | Key::End) => {
                self.library_search.caret = if code == Some(Key::Home) { 0 } else { self.library_search.query.len() };
                self.library_search.select_all = false;
                false
            }
            _ if !self.mods.super_key() => event.text.as_deref().is_some_and(|text| self.library_search.insert(text)),
            _ => false,
        };
        if changed {
            self.search_changed();
        } else if let Some(window) = &self.window {
            window.request_redraw();
        }
        true
    }

    fn library_key(&mut self, el: &ActiveEventLoop, code: Key) -> bool {
        if !self.library_mode { return false; }
        if self.mods.control_key() {
            match code {
                Key::KeyM => { self.toggle_missing_titles(); return true; }
                Key::ArrowLeft | Key::ArrowRight => {
                    let direction = if code == Key::ArrowLeft { -1 } else { 1 };
                    self.select_library_category(self.library_search.collection.category.step(direction));
                    return true;
                }
                _ => {}
            }
        }
        if self.library_search.collection.missing_offset.is_some() {
            match code {
                Key::Escape => self.toggle_missing_titles(),
                Key::ArrowUp => self.move_library_selection(-1),
                Key::ArrowDown => self.move_library_selection(1),
                Key::PageUp => self.move_library_selection(-(library_catalogue::MISSING_ROWS as isize)),
                Key::PageDown => self.move_library_selection(library_catalogue::MISSING_ROWS as isize),
                Key::Home => self.move_library_selection(isize::MIN),
                Key::End => self.move_library_selection(isize::MAX),
                _ => {}
            }
            return true;
        }
        match code {
            Key::ArrowLeft => self.move_library_selection(-1),
            Key::ArrowRight => self.move_library_selection(1),
            Key::ArrowUp => self.move_library_selection(-(library::layout(self.bindings.ui_shortcuts.artwork_size).columns as isize)),
            Key::ArrowDown => self.move_library_selection(library::layout(self.bindings.ui_shortcuts.artwork_size).columns as isize),
            Key::Enter | Key::NumpadEnter | Key::Space => self.launch_library_selection(),
            Key::Escape if !self.library_search.query.is_empty() => {
                self.library_search.clear();
                self.search_changed();
            }
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
        self.boot_fast_until = None;
        if let Some(now)=opencdi::rtc::local_now(){opencdi::rtc::set(&mut self.m.bus,now);}
        self.rtc_last=Instant::now();
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
        let pointing = self.host_input && self.focused && self.wiimote_cursor.is_some();
        let ui = self.library_mode || self.paused || self.pause_info || self.preview_started.is_some() || self.controller_config.is_some();
        let want = self.host_input && self.focused && self.mouse_captured && !ui && !pointing;
        window.set_cursor_visible(!(want || (pointing && !ui)));
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
        if self.controller_config.is_some() {
            if self.gun_calibrating.is_some() {
                if code == Key::Escape { self.gun_calibrating = None; }
                return;
            }
            if !self.controller_config.as_ref().is_some_and(|s| s.listening) {
                let mode = match code {
                    Key::Digit1 => Some(InputDeviceMode::Gamepad),
                    Key::Digit2 => Some(InputDeviceMode::Mouse),
                    Key::Digit3 => Some(InputDeviceMode::Peacekeeper),
                    _ => None,
                };
                if let Some(mode) = mode {
                    self.input_device_mode = mode;
                    self.save_input_settings();
                    return;
                }
                if self.input_device_mode == InputDeviceMode::Peacekeeper && code == Key::KeyC {
                    self.gun_calibrating = Some(None);
                    self.gun_notice.clear();
                    return;
                }
            }
        }
        if code == self.bindings.btn2 && self.go_back() { return; }
        if self.preview_started.is_some() && self.controller_config.is_none() {
            if code == Key::Escape {
                self.stop_to_library();
            } else if matches!(code, Key::Enter | Key::NumpadEnter | Key::Space) || code == self.bindings.btn1 {
                push_action(GuiAction::PlaySelectedDisc);
            }
            return;
        }
        if self.pause_info { return; }
        if self.toolbar_popup == Some(library::ToolbarPopup::About) {
            if matches!(code, Key::Escape | Key::Enter | Key::NumpadEnter | Key::Space) {
                self.toolbar_popup = None;
                if let Some(window) = &self.window { window.request_redraw(); }
            }
            return;
        }
        if self.controller_config.is_some() && self.input_device_mode == InputDeviceMode::Peacekeeper {
            if code == Key::Escape {
                self.controller_config = None;
                self.mouse_captured = self.controller_config_was_captured;
                self.apply_grab();
            }
            return;
        }
        let mut save_input = false;
        let mut save_shortcuts = false;
        let mut closed_controller_config = false;
        if let Some(state) = self.controller_config.as_mut() {
            match state.key_pressed(code) {
                ControllerConfigAction::AssignedKey(row, key) => {
                    if row == 7 {
                        self.bindings.ui_shortcuts.set_pause_key(key);
                        save_shortcuts = true;
                    } else {
                        self.bindings.set_key_binding(row, key);
                        save_input = true;
                    }
                }
                ControllerConfigAction::AssignedPauseKey(key) => { self.bindings.ui_shortcuts.set_pause_key(key); save_shortcuts = true; }
                ControllerConfigAction::AssignedExitKey(key) => { self.bindings.ui_shortcuts.set_exit_key(key); save_shortcuts = true; }
                ControllerConfigAction::Close => {
                    self.controller_config = None;
                    closed_controller_config = true;
                    self.mouse_captured = self.controller_config_was_captured;
                    self.apply_grab();
                }
                ControllerConfigAction::Cancelled | ControllerConfigAction::Handled => {}
            }
            if save_input { self.save_input_settings(); }
            if save_shortcuts { self.save_global_shortcuts(); }
            if self.controller_config.is_some() { return; }
            if closed_controller_config {
                if let Some(window) = &self.window { window.request_redraw(); }
                return;
            }
        }
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
        if code==Key::Escape && !self.library_mode && self.current_disc_path.is_none(){self.stop_to_library();return;}
        if self.bound(code) { return; }
        let ctrl = self.mods.control_key();
        let alt = self.mods.alt_key();
        match code {
            Key::Escape => el.exit(),
            Key::F1 => push_action(GuiAction::OpenFeedback),
            Key::F6 => push_action(GuiAction::QuickSaveState),
            Key::F7 => push_action(GuiAction::QuickLoadState),
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
                self.muted = !self.muted;
                    self.publish_gain();
                info!("Audio: {}", if self.muted { "MUTED" } else { "UNMUTED" });
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
                            self.disc_details = library::DiscDetails::load(&path, &self.managed_artwork_root);
                            self.boot_synopsis = synopsis::load(&self.synopsis_root, &path);
                            self.next_synopsis_check = Instant::now();
                            self.artwork_retry_at = self.next_synopsis_check;
                            self.activate_nvram_for_selection();
                            self.boot_disc_art = library::load_disc_artwork(&path, &self.managed_cd_path);
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
                GuiAction::OpenDisc(path) => self.preview_disc(path),
                GuiAction::PlaySelectedDisc => {
                    if self.preview_started.is_none() { continue; }
                    let Some(path) = self.current_disc_path.clone() else { continue; };
                    info!("Loading and booting new disc (Reset): {:?}", path);
                    match self.m.bus.load_cdrom(&path) {
                        Ok(_) => {
                            self.save_active_disc_settings();
                            self.activate_disc_settings(&path);
                            self.set_search_focus(false);
                            self.preview_started = None;
                            self.animation_clock = Instant::now();
                            self.grab_applied = None;
                            self.activate_nvram_for_selection();
                            self.add_recent(&path);
                            info!("Disc loaded successfully: {}", self.current_game_name);

                            if !self.dvc_disabled && !self.m.bus.dvc.is_fitted() {
                                if let Some(dvc_path) = self.dvc_arg.clone().or_else(|| find_dvc_rom(self.m.bus.board)) {
                                    if let Ok(_) = self.m.load_dvc(&dvc_path) {
                                        info!("Loaded Digital Video Cartridge (DVC) extension from {:?}", dvc_path);
                                        #[cfg(target_os = "windows")]
                                        gui::update_dvc_menu(true);
                                    }
                                }
                            }

                            let is_ntsc = self.m.bus.cdrom.as_ref().is_some_and(|cd| cd.is_ntsc_hint(&self.current_game_name).unwrap_or(false));
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
                            self.apply_grab();
                            self.m.bus.emulate_wait_states = true;
                            self.next_frame_time = Instant::now();
                            self.boot_details_until = Some(self.next_frame_time + Duration::from_millis(750));
                            if self.skip_shell {
                                self.boot_fast_until = Some(self.next_frame_time + Duration::from_secs(30));
                            }
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
                    self.m.bus.slave.set_disc_inserted(false, false);
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
                    info!("Controller settings page selected: {:?}; all input devices remain enabled", mode);
                }
                GuiAction::ReloadControls => {
                    self.bindings.load_from_ini();
                    #[cfg(target_os = "windows")]
                    gui::update_profile_menu(self.bindings.profile);
                    info!("Controls reloaded from INI file. Profile: {:?}", self.bindings.profile);
                }
                GuiAction::OpenControllerConfig => {
                    self.set_search_focus(false);
                    self.toolbar_popup = None;
                    self.controller_config_was_captured = self.mouse_captured;
                    self.controller_config = Some(ControllerConfigState::default());
                    self.mouse_captured = false;
                    send_pointer(&mut self.m.bus, self.pad_x, self.pad_y, 0);
                    self.prev_key_btn = 0;
                    self.apply_grab();
                    if let Some(window) = &self.window { window.request_redraw(); }
                }
                GuiAction::ToggleReleaseMouse => {
                    self.mouse_captured = !self.mouse_captured;
                    info!("Mouse capture: {}", if self.mouse_captured { "CAPTURED" } else { "RELEASED" });
                }
                GuiAction::ToggleMute => {
                    self.muted = !self.muted;
                    self.publish_gain();
                    info!("Audio: {}", if self.muted { "MUTED" } else { "UNMUTED" });
                }
                GuiAction::VolumeUp => {
                    self.volume = (self.volume + 0.05).min(2.0);
                    self.publish_gain();
                }
                GuiAction::VolumeDown => {
                    self.volume = (self.volume - 0.05).max(0.0);
                    self.publish_gain();
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
                    info!("Disc folders updated.");
                }
                GuiAction::SwitchBios(path) => self.switch_bios(path),
                GuiAction::SelectBios(model) => self.handle_toolbar_action(if model==490 { library::ToolbarAction::Bios490 } else { library::ToolbarAction::Bios220 }),
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
                        if let Some(dvc_path) = self.dvc_arg.clone().or_else(|| find_dvc_rom(self.m.bus.board)) {
                            if let Ok(_) = self.m.load_dvc(&dvc_path) {
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
                GuiAction::QuickSaveState => self.state_slot(1, false),
                GuiAction::QuickLoadState => self.state_slot(1, true),
                GuiAction::SaveState(slot) => self.state_slot(slot, false),
                GuiAction::LoadState(slot) => self.state_slot(slot, true),
                GuiAction::ClearNvram => {
                    if let Some(board) = self.m.bus.mono4.as_deref_mut() {
                        board.nvram.clear();
                    } else {
                        let _ = self.m.bus.nvram.clear();
                    }
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

                if let Some((x,y,b)) = opencdi::boot::shell_play_click(&m.bus, shell_state, &mut self.shell_ready_count) {
                    send_pointer(&mut m.bus,x,y,b);
                }
                let started = disc_program_started(m.cpu.pc, self.frame_count,
                    if m.bus.mono4.is_some() { 0 } else { shell_state }, self.shell_ready_count);
                if started {
                    info!("Disc program started (PC=0x{:06X}); automatic boot complete.", m.cpu.pc);
                    self.disc_launched = true;
                    self.boot_fast_until = None;
                    m.bus.emulate_wait_states = true;
                    self.pad_x = 384;
                    self.pad_y = 280;
                    self.pad_x_f = 384.0;
                    self.pad_y_f = 280.0;
                    send_pointer(&mut m.bus, 384, 280, 0);
                }
            }
        }
    }

    // Keyboard, gamepad and host mouse into the slave controller, once per emulated frame
    fn update_input(&mut self) {
        let is_active = self.focused && self.host_input && self.controller_config.is_none();

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
        let mut play_pad_down = false;
        let mut back_pad_down = false;
        let capturing_mapping = self.controller_config.as_ref().is_some_and(|config| config.listening);
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
                        play_pad_down = true;
                        key_btn |= 1;
                    }
                    if gamepad.is_pressed(pad_btn2) {
                        back_pad_down = true;
                        key_btn |= 2;
                    }
                    if gamepad.is_pressed(pad_btn3) {
                        key_btn |= 3;
                    }
                }
            }
        }
        {
            let buttons = self.wiimote_buttons;
            let (remote_x,remote_y) = self.wiimote_auto.directions(buttons);
            let sideways = self.wiimote_auto.mode == wiimote::ControlMode::Sideways;
            lib_left |= remote_x < 0; lib_right |= remote_x > 0;
            lib_up |= remote_y < 0; lib_down |= remote_y > 0;
            let confirm = buttons & wiimote::ONE != 0 || sideways && buttons & wiimote::A != 0;
            lib_confirm |= confirm;
            play_pad_down |= confirm;
            if is_active && sideways {
                dir_x += remote_x; dir_y += remote_y;
                if buttons & (wiimote::ONE | wiimote::A) != 0 { key_btn |= 1; }
                if buttons & (wiimote::TWO | wiimote::B) != 0 { key_btn |= 2; }
            }
            // B is Back in menus and previews, and remains the trigger during play.
            back_pad_down |= buttons & wiimote::MINUS != 0 || sideways && buttons & wiimote::TWO != 0
                || (self.library_mode || self.preview_started.is_some() || self.controller_config.is_some())
                    && buttons & wiimote::B != 0;
            pause_pad_down |= buttons & wiimote::HOME != 0;
        }
        pause_pad_down |= self.bindings.ui_shortcuts.pause_pad.contains(&gilrs::Button::Mode) && xbox_guide_pressed();
        if let Some(button) = captured_button {
            let mut assigned_row = None;
            if let Some(config) = self.controller_config.as_mut() {
                if config.listening && config.target == ControllerConfigTarget::Gamepad {
                    if config.row == 7 { self.bindings.ui_shortcuts.set_pause_combo(vec![button]); }
                    else { self.bindings.set_pad_binding(config.row, button); }
                    config.listening = false;
                    assigned_row = Some(config.row);
                }
            }
            match assigned_row {
                Some(7) => self.save_global_shortcuts(),
                Some(_) => self.save_input_settings(),
                None => {}
            }
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
        if save_combo { self.save_global_shortcuts(); }
        if let Some(config) = self.controller_config.as_mut() {
            if !config.listening {
                if cfg_up && !self.config_prev_up { config.row = (config.row + controller_config::ROWS - 1) % controller_config::ROWS; }
                if cfg_down && !self.config_prev_down { config.row = (config.row + 1) % controller_config::ROWS; }
                if (cfg_left && !self.config_prev_left) || (cfg_right && !self.config_prev_right) {
                    config.target = if config.target == ControllerConfigTarget::Keyboard { ControllerConfigTarget::Gamepad } else { ControllerConfigTarget::Keyboard };
                }
                config.confirm_gamepad(cfg_confirm, self.config_prev_confirm);
            }
        }
        self.config_prev_up = cfg_up; self.config_prev_down = cfg_down;
        self.config_prev_left = cfg_left; self.config_prev_right = cfg_right; self.config_prev_confirm = cfg_confirm;
        let back_pressed = back_pad_down && !self.previous_back_pad;
        self.previous_back_pad = back_pad_down;
        let play_pressed = play_pad_down && !self.previous_play_pad;
        self.previous_play_pad = play_pad_down;
        if !capturing_mapping && back_pressed && self.go_back() { return; }
        if self.preview_started.is_some() && self.controller_config.is_none() {
            if play_pressed { push_action(GuiAction::PlaySelectedDisc); }
            if exit_pad_down { self.exit_requested = true; }
            return;
        }
        if self.controller_config.is_some() { return; }
        if menu_pad_down && !self.prev_menu_pad { self.toggle_menu(); }
        self.prev_menu_pad = menu_pad_down;
        if pause_pad_down && !self.prev_pause_pad && self.pause_is_available() { self.toggle_pause_menu(); }
        self.prev_pause_pad = pause_pad_down;
        if exit_pad_down { self.exit_requested = true; }
        if self.library_mode && self.toolbar_popup != Some(library::ToolbarPopup::About) {
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
                if self.physical_mouse_buttons & 1 != 0 { mouse_btn |= 1; }
                if self.physical_mouse_buttons & 2 != 0 { mouse_btn |= 2; }
            }
        }

        let wiimote_raw_aim = self.wiimote_position().filter(|_| self.wiimote_auto.mode == wiimote::ControlMode::Pointer).filter(|pos| {
            let (x,y,w,h) = self.image_rect;
            pos.x >= x as f64 && pos.y >= y as f64 && pos.x < (x+w) as f64 && pos.y < (y+h) as f64
        }).map(|pos| cursor_to_cdi(pos,self.image_rect));
        let wiimote_aim = wiimote_raw_aim.map(|(x,y)| {
            wiimote::correct_game_aim(&self.current_game_name,self.m.bus.mcd212.cursor_control,x,y)
        });
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

        if self.boot_details_until.is_some_and(|until| Instant::now() < until)
            || (!self.disc_launched && m.bus.cdrom.is_some() && self.skip_shell)
        {
            // Keep host input from interfering with the automatic Play CD-i selection.
        } else {
            // All devices feed one CD-i pointer. Motion comes from the last
            // deliberately used device; held buttons are combined independently.
            if is_active && (dir_x != 0 || dir_y != 0) {
                self.input_arbiter.gamepad_motion(wiimote_raw_aim);
                const PAD_SPEED: f32 = 2.5;
                self.pad_x_f = (self.pad_x_f + dir_x.clamp(-1,1) as f32 * PAD_SPEED).clamp(0.0,767.0);
                self.pad_y_f = (self.pad_y_f + dir_y.clamp(-1,1) as f32 * PAD_SPEED).clamp(0.0,559.0);
                self.pad_x = self.pad_x_f.round() as i32;
                self.pad_y = self.pad_y_f.round() as i32;
            } else if is_active {
                match self.input_arbiter.source() {
                    opencdi::host_input::PointerSource::Mouse => {
                        self.pad_x = mouse_x;
                        self.pad_y = mouse_y;
                    }
                    opencdi::host_input::PointerSource::Wiimote => {
                        if let Some((x,y)) = wiimote_aim { self.pad_x = x; self.pad_y = y; }
                    }
                    opencdi::host_input::PointerSource::Gamepad => {}
                }
                self.pad_x_f = self.pad_x as f32;
                self.pad_y_f = self.pad_y as f32;
            }
            let mut remote_buttons = 0;
            if wiimote_aim.is_some() && self.wiimote_trigger_ready {
                if self.wiimote_buttons & wiimote::A != 0 { remote_buttons |= 1; }
                if self.wiimote_buttons & wiimote::B != 0 { remote_buttons |= 2; }
            }
            let buttons = opencdi::host_input::combined_buttons(is_active,mouse_btn,key_btn,remote_buttons);
            if m.bus.cdrom.is_none(){opencdi::boot::send_shell_pointer(&mut m.bus,self.pad_x,self.pad_y,buttons);}
            else{opencdi::boot::send_game_pointer(&mut m.bus,self.pad_x,self.pad_y,buttons);}
            self.prev_key_btn = key_btn;
        }

        // Audio hotkeys (+ / - = Volume, held)
        if self.keys.contains(&Key::Equal) || self.keys.contains(&Key::NumpadAdd) {
            self.volume = (self.volume + 0.02).min(2.0);
            self.publish_gain();
        }
        if self.keys.contains(&Key::Minus) || self.keys.contains(&Key::NumpadSubtract) {
            self.volume = (self.volume - 0.02).max(0.0);
            self.publish_gain();
        }
    }

    fn publish_gain(&self) {
        let g = if self.muted { 0.0f32 } else { self.volume };
        self.audio_gain.store(g.to_bits(), Ordering::Relaxed);
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
        if self.preview_started.is_some() {
            if let Some(window) = &self.window {
                window.set_title(&format!("OpenCDi – {} | Ready to play", self.current_game_name));
            }
            return;
        }
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
                "OpenCDi – {} | {} | CPU: {:.2} MIPS | CD: {} | Render: {} | WaitStates: {}{}",
                self.current_game_name, speed, self.measured_mips, cd_str, render_str, wait_status,
                if !self.disc_launched && self.skip_shell && m.bus.cdrom.is_some() { " | Starting disc" } else { "" }
            ));
        }
    }

    // One emulated frame plus host input; returns false when the loop must stop
    fn boot_accelerating(&self) -> bool {
        self.boot_fast_until.is_some_and(|until| Instant::now() < until)
            && !self.disc_launched && self.skip_shell && !self.paused
            && !self.library_mode && self.preview_started.is_none() && self.m.bus.cdrom.is_some()
    }

    fn tick(&mut self) -> bool {
        self.update_wiimote();
        if self.exit_requested { return false; }
        self.update_wiimote_mouse();
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
            if self.pause_info && let Some(window) = &self.window { window.request_redraw(); }
            if !self.paused { return true; }
            // Keep the title live (disc, wait states) at about two updates a second
            self.paused_ticks += 1;
            if self.paused_ticks % 25 == 0 {
                self.refresh_title();
            }
            return true;
        }
        if self.library_mode || self.preview_started.is_some() {
            self.update_input();
            if self.exit_requested { return false; }
            if let Some(window) = &self.window { window.request_redraw(); }
            return true;
        }
        self.autoplay();
        self.m.run_frame();
        if self.frame_count % 300 == 0 {
            flush_nvram(&mut self.m.bus);
        }

        let curr_vsr = (self.m.bus.mcd212.get_vsr::<0>(), self.m.bus.mcd212.get_vsr::<1>());
        if curr_vsr != self.prev_vsr {
            self.prev_vsr = curr_vsr;
            self.buffer_flips_interval += 1;
        }

        self.update_input();
        if self.exit_requested { return false; }
        // Emulation runs on the timer; only the picture is skipped while the window is hidden
        self.audio_buf.clear();
        self.m.drain_audio(&mut self.audio_buf);
        if self.fast_forward {
            // Drop fast-forward audio so the ring never holds sped-up backlog
            if let Ok(mut q) = self.audio_ring.lock() {
                q.clear();
            }
        } else {
            audio::feed_ring(&self.audio_ring, &self.audio_buf);
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
        let Some(window) = &self.window else { return };
        let size = window.inner_size();
        let (Some(w), Some(h)) = (std::num::NonZeroU32::new(size.width), std::num::NonZeroU32::new(size.height)) else { return };
        let (dw, dh) = (size.width as usize, size.height as usize);
        let show_details = self.showing_disc_details();
        self.image_rect = self.content_rect(dw, dh);
        let Some(surface) = &mut self.surface else { return };
        if surface.resize(w, h).is_err() {
            return;
        }
        self.last_present = Instant::now();
        if let Ok(mut buffer) = surface.buffer_mut() {
            if let Some(state) = &self.controller_config {
                controller_config::render_native(&mut buffer,dw,dh,self.image_rect,&self.bindings,&self.bindings.ui_shortcuts,self.input_device_mode,state,self.controller_art.as_ref(),self.mouse_art.as_ref(),self.peacekeeper_art.as_ref(),self.wiimote_sample,&self.gun_notice,self.gun_calibrating,self.wiimote_auto.mode);
            } else if self.library_mode {
                library::render_native(&mut buffer,dw,dh,self.image_rect,&self.library_items,self.library_selected,self.bindings.ui_shortcuts.artwork_size,self.menu_visible,self.display_mode,self.toolbar_popup,self.optical_disc,&self.library_search,self.artwork_progress.as_ref());
            } else {
                if show_details {
                    // The library preview is stationary until Play. Once started,
                    // use real time even in the in-game pause/Info screen.
                    let animation_frame = if self.preview_started.is_some() { 0 }
                        else { (self.animation_clock.elapsed().as_millis() / 20) as u64 };
                    self.boot_scene.render(&mut buffer, dw, dh, self.image_rect,
                        &self.current_game_name, &self.disc_details, self.boot_synopsis.as_deref(),
                        self.preview_started.is_some(), self.boot_disc_art.as_ref(), animation_frame);
                } else {
                    blit_aspect(&mut buffer, dw, dh, &self.m.bus.mcd212.framebuffer[..], self.image_rect);
                    draw_bezel(&mut buffer, dw, dh, self.image_rect);
                }
            }
            if render_pause_menu && self.controller_config.is_none() && !self.pause_info {
                library::render_pause_menu_full(&mut buffer, dw, dh, &self.current_game_name,
                    self.disc_details.logo.as_ref(), self.pause_menu_selected);
            }
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
        self.image_rect = self.content_rect(size.width as usize, size.height as usize);
        self.focused = window.has_focus();
        self.window = Some(window);
        #[cfg(windows)]
        self.startup_update.start();
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
            gui::update_bios_menu(self.library_search.active_bios.as_ref().map_or("",|(model,_)|model.as_str()));
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
                self.image_rect = self.content_rect(size.width as usize, size.height as usize);
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::Focused(f) => {
                self.focused = f;
                if !f {
                    self.physical_mouse_buttons = 0;
                    self.input_arbiter = Default::default();
                    self.mouse_activity = false;
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
            WindowEvent::Ime(winit::event::Ime::Commit(text)) if self.host_input && self.library_mode && self.library_search.focused => {
                if self.library_search.insert(&text) { self.search_changed(); }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if self.search_key_event(&event) { return; }
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
            WindowEvent::CursorMoved { position, .. } => self.on_cursor_moved(position),
            WindowEvent::CursorLeft { .. } => self.cursor = None,
            WindowEvent::MouseWheel { .. } if !self.host_input => {}
            WindowEvent::MouseWheel { delta, .. } => {
                if self.library_mode && self.controller_config.is_none() && self.toolbar_popup != Some(library::ToolbarPopup::About) {
                    let amount = match delta {
                        MouseScrollDelta::LineDelta(_, y) => y as f64,
                        MouseScrollDelta::PixelDelta(pos) => pos.y / 60.0,
                    };
                    self.library_scroll_accum += amount;
                    let rows = self.library_scroll_accum.trunc() as isize;
                    if rows != 0 {
                        let step = if self.library_search.collection.missing_offset.is_some() { 1 }
                            else { library::layout(self.bindings.ui_shortcuts.artwork_size).columns as isize };
                        self.move_library_selection(-rows * step);
                        self.library_scroll_accum -= rows as f64;
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => self.on_mouse_input(state, button),
            _ => {}
        }
    }

    fn device_event(&mut self, _el: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta: (dx,dy) } = &event {
            if self.focused && self.host_input && (*dx != 0.0 || *dy != 0.0) { self.mouse_activity = true; }
        }
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
        #[cfg(windows)]
        {
            let owner = self.hwnd();
            if self.startup_update.poll(owner) {
                el.exit();
                return;
            }
        }
        let seconds=self.rtc_last.elapsed().as_secs();
        if seconds>0 {opencdi::rtc::advance(&mut self.m.bus,seconds);self.rtc_last+=Duration::from_secs(seconds);}
        self.update_library(Instant::now());
        let boot_fast = self.boot_accelerating();
        let ff = boot_fast || (self.host_input && self.focused && self.key(Key::Space) && !self.bound(Key::Space));
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
        let burst_started = Instant::now();
        if !self.tick() {
            el.exit();
            return;
        }
        if boot_fast {
            // Windows presentation may wait for the monitor refresh. Run a short
            // batch between paints rather than limiting boot to one emulated frame
            // per presentation. Recheck the handover before every additional frame.
            for _ in 0..7 {
                if !self.boot_accelerating() || burst_started.elapsed() >= Duration::from_millis(12) { break; }
                if !self.tick() { el.exit(); return; }
            }
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
        self.wiimote_cursor = None;
        self.apply_grab();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "Writes the matching screen previews; requires OPENCDI_THEME_PREVIEWS"]
    fn render_matching_theme_previews() {
        let folder=PathBuf::from(std::env::var("OPENCDI_THEME_PREVIEWS").unwrap());
        std::fs::create_dir_all(&folder).unwrap();
        let save=|name:&str,pixels:&[u32]| {
            let bytes:Vec<_>=pixels.iter().flat_map(|p| [((p>>16)&255) as u8,((p>>8)&255) as u8,(p&255) as u8]).collect();
            let mut encoder=png::Encoder::new(std::fs::File::create(folder.join(name)).unwrap(),1920,1080);
            encoder.set_color(png::ColorType::Rgb);encoder.set_depth(png::BitDepth::Eight);
            encoder.write_header().unwrap().write_image_data(&bytes).unwrap();
        };
        let gamepad=controller_config::load_controller_art().unwrap();
        let mouse=controller_config::load_mouse_art().unwrap();
        let gun=controller_config::load_peacekeeper_art().unwrap();
        let bindings=KeyBindings::new();let shortcuts=UiShortcuts::default();
        for (name,mode) in [("controller-gamepad.png",InputDeviceMode::Gamepad),("controller-mouse.png",InputDeviceMode::Mouse),("controller-peacekeeper.png",InputDeviceMode::Peacekeeper)] {
            let mut pixels=vec![0;1920*1080];
            controller_config::render_native(&mut pixels,1920,1080,(0,0,1920,1080),&bindings,&shortcuts,mode,&controller_config::State::default(),Some(&gamepad),Some(&mouse),Some(&gun),None,"",None,wiimote::ControlMode::Pointer);
            save(name,&pixels);
        }
        let mut pixels=vec![0xFF152637;1920*1080];
        library::render_pause_menu_full(&mut pixels,1920,1080,"Mad Dog McCree (USA, Europe)",None,0);
        save("pause-menu.png",&pixels);
        let mut search=library::Search::default();
        let items=vec![library::Item{path:PathBuf::from("Mad Dog McCree (USA, Europe).chd"),title:"Mad Dog McCree (USA, Europe)".into(),cover:None,disc:None,logo:None}];
        search.collection.refresh(&items);
        library::render_native(&mut pixels,1920,1080,(0,0,1920,1080),&items,0,shortcuts.artwork_size,true,DisplayMode::Windowed,None,Some('V'),&search,None);
        save("category-tabs.png",&pixels);
        search.active_bios=Some(("Philips CD-i 490".into(),"cdi490a.rom".into()));
        library::render_native(&mut pixels,1920,1080,(0,0,1920,1080),&items,0,shortcuts.artwork_size,true,DisplayMode::Windowed,Some(library::ToolbarPopup::Settings),Some('V'),&search,None);
        save("settings-bios.png",&pixels);
        pixels.fill(0xFF080E17);
        library::render_preview_actions(&mut pixels,1920,1080,(0,0,1920,1080));
        save("disc-actions.png",&pixels);
        if let (Ok(path),Ok(root))=(std::env::var("OPENCDI_PREVIEW_DISC"),std::env::var("OPENCDI_PREVIEW_ART")) {
            let path=PathBuf::from(path);let root=PathBuf::from(root);
            let details=library::DiscDetails::load(&path,&root);
            let disc=library::load_disc_artwork(&path,&root.join("CD"));
            let synopsis=synopsis::load(&root.parent().unwrap_or(&root).join("Synopses"),&path);
            let mut scene=library::BootScene::default();
            scene.render(&mut pixels,1920,1080,(0,0,1920,1080),"The 7th Guest",&details,synopsis.as_deref(),true,disc.as_ref(),0);
            save("disc-info.png",&pixels);
            scene.render(&mut pixels,1920,1080,(0,0,1920,1080),"The 7th Guest",&details,synopsis.as_deref(),false,disc.as_ref(),30);
            save("disc-info-loading.png",&pixels);
        }

    }
    #[test]
    fn readable_nvram_names_are_separate_and_preserve_old_saves() {
        let root=std::env::temp_dir().join(format!("opencdi-nvram-split-{}",std::process::id()));
        let global=root.join("nvram.bin");let disc=root.join("Joker's Wild! Jr., The (USA).chd");
        let old=legacy_per_disc_nvram_save_path(&global,&disc);std::fs::create_dir_all(old.parent().unwrap()).unwrap();
        let bytes=vec![0x42;8192];std::fs::write(&old,&bytes).unwrap();
        let first=per_disc_nvram_save_path(&global,&disc,"220");let second=per_disc_nvram_save_path(&global,&disc,"490");
        assert_eq!(first,root.join("NVRAM/220/Joker's Wild! Jr., The (USA).bin"));
        assert_eq!(second,root.join("NVRAM/490/Joker's Wild! Jr., The (USA).bin"));
        assert_eq!(std::fs::read(&first).unwrap(),bytes);assert_eq!(std::fs::read(&old).unwrap(),bytes);assert!(!second.exists());
        std::fs::write(&first,vec![0x77;8192]).unwrap();assert_eq!(per_disc_nvram_save_path(&global,&disc,"220"),first);assert_eq!(std::fs::read(first).unwrap(),vec![0x77;8192]);
        // A shared 32 KB save must be imported only into the 490 folder.
        std::fs::write(&old,vec![0x33;32768]).unwrap();assert_eq!(std::fs::read(per_disc_nvram_save_path(&global,&disc,"490")).unwrap(),vec![0x33;32768]);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn save_state_folders_do_not_collide_between_bioses(){
        let a=savestate_path_for("Test Game",1,"220");let b=savestate_path_for("Test Game",1,"490");
        assert_ne!(a,b);assert_eq!(a.file_name(),b.file_name());assert!(a.parent().unwrap().ends_with("Savestates/220"));assert!(b.parent().unwrap().ends_with("Savestates/490"));
    }
    #[test]
    #[ignore = "Measures local library decoding; requires OPENCDI_BENCH_DISCS and OPENCDI_BENCH_ART"]
    fn profile_library_loading() {
        let root = PathBuf::from(std::env::var("OPENCDI_BENCH_DISCS").unwrap());
        let art = PathBuf::from(std::env::var("OPENCDI_BENCH_ART").unwrap());
        let started = Instant::now();
        let paths = library::scan_paths(&[root]);
        println!("Titles: {} in {:?}", paths.len(), started.elapsed());
        let page_started = Instant::now();
        let page = library::load_items(paths.iter().take(24).cloned().collect(), &art.join("Covers"), &art.join("CD"), &art);
        println!("First artwork page: {} in {:?}", page.len(), page_started.elapsed());
        let remaining = Instant::now();
        let rest = library::load_items(paths.into_iter().skip(24).collect(), &art.join("Covers"), &art.join("CD"), &art);
        println!("Remaining artwork: {} in {:?}; total {:?}", rest.len(), remaining.elapsed(), started.elapsed());
    }
    #[test]
    fn autoplay_does_not_release_input_while_the_shell_is_still_active() {
        // Observed with the saved Mad Dog McCree setup: DVC code is executing,
        // but shell state 2 still denotes its menu.
        assert!(!super::disc_program_started(0x00df4fce, 630, 2, 192));
        assert!(!super::disc_program_started(0x00df4fce, 630, 3, 192));
        assert!(!super::disc_program_started(0x00df4fce, 630, 0, 20));
        assert!(super::disc_program_started(0x00df4fce, 630, 0, 192));
        assert!(super::disc_program_started(0x00020000, 700, 0, 192));
        assert!(!super::disc_program_started(0x00408108, 700, 0, 192));
    }
    use super::*;

    #[test]
    fn individual_download_fills_only_missing_art_before_worker_finishes() {
        let image = library::Artwork { w: 1, h: 1, pixels: [0xFF112233].into() };
        let mut items = [
            library::Item { path: "one.chd".into(), title: "One".into(), cover: None, disc: None, logo: None },
            library::Item { path: "two.chd".into(), title: "Two".into(), cover: None, disc: None, logo: None },
        ];
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(DownloadedArtwork { title: "One".into(), kind: launchbox::ArtworkKind::Cover, image: image.clone(), preferred: false }).ok().unwrap();
        let cover = rx.try_recv().unwrap();
        assert!(cover.apply(&mut items));
        assert!(items[0].cover.is_some());
        assert!(items[0].disc.is_none() && items[1].cover.is_none());
        assert!(!cover.apply(&mut items));
        assert!(matches!(rx.try_recv(), Err(std::sync::mpsc::TryRecvError::Empty)));
        tx.send(DownloadedArtwork { title: "One".into(), kind: launchbox::ArtworkKind::Disc, image, preferred: false }).ok().unwrap();
        assert!(rx.try_recv().unwrap().apply(&mut items));
        assert!(items[0].disc.is_some());
    }

    #[test]
    fn bezel_keeps_game_picture_and_mouse_inside_its_opening() {
        let (w, h) = (1672, 941);
        let game = bezel_game_rect(w, h);
        assert!(game.0 >= 226 && game.1 >= 28);
        assert!(game.0 + game.2 <= 1446 && game.1 + game.3 <= 914);
        let mut pixels = vec![0xff000000; w * h];
        let frame = vec![0xff336699; SCREEN_WIDTH * SCREEN_HEIGHT];
        blit_aspect(&mut pixels, w, h, &frame, game);
        draw_bezel(&mut pixels, w, h, game);
        assert_eq!(pixels[game.1 * w + game.0], 0xff336699);
        assert_ne!(pixels[w * 100 + 100], 0xff000000);
        assert_eq!(cursor_to_cdi(PhysicalPosition::new(game.0 as f64, game.1 as f64), game), (0, 0));
    }

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
