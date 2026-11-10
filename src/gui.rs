use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use log::info;

use crate::frontend::{push_action, DisplayMode, GuiAction, InputDeviceMode, InputProfile, NvramMode};

pub const ID_FILE_INSERT: usize = 1000;
pub const ID_FILE_OPEN: usize = 1001;
pub const ID_FILE_RESET: usize = 1002;
pub const ID_FILE_EJECT: usize = 1004;
pub const ID_FILE_EXIT: usize = 1003;

pub const ID_PHYSICAL_DRIVE_BOOT_BASE: usize = 1100;
pub const ID_PHYSICAL_DRIVE_SWAP_BASE: usize = 1130;

pub const ID_PROFILE_STANDARD: usize = 1010;
pub const ID_PROFILE_WASD: usize = 1011;
pub const ID_PROFILE_NUMPAD: usize = 1012;
pub const ID_PROFILE_CUSTOM: usize = 1015;
pub const ID_CONTROLS_EDIT_INI: usize = 1016;
pub const ID_CONTROLS_RELOAD_INI: usize = 1017;
pub const ID_CONTROLS_CONFIGURE: usize = 1018;
pub const ID_CONTROLS_INFO: usize = 1013;
pub const ID_CONTROLS_RELEASE_MOUSE: usize = 1014;
pub const ID_INPUT_DEVICE_GAMEPAD: usize = 1060;
pub const ID_INPUT_DEVICE_MOUSE: usize = 1061;

pub const ID_AUDIO_MUTE: usize = 1020;
pub const ID_AUDIO_VOL_UP: usize = 1021;
pub const ID_AUDIO_VOL_DOWN: usize = 1022;

pub const ID_VIDEO_FULLSCREEN: usize = 1030;
pub const ID_VIDEO_SCREENSHOT: usize = 1031;
pub const ID_VIDEO_PAL: usize = 1032;
pub const ID_VIDEO_NTSC: usize = 1033;
pub const ID_DISPLAY_BORDERLESS: usize = 1034;
pub const ID_DISPLAY_DECORATED: usize = 1035;
pub const ID_DISPLAY_WINDOWED: usize = 1036;
pub const ID_SKIP_SHELL: usize = 1037;
pub const ID_EMU_WAITSTATES: usize = 1050;
pub const ID_EMU_DVC: usize = 1051;

pub const ID_HELP_CONTROLS: usize = 1040;
pub const ID_HELP_ABOUT: usize = 1041;
pub const ID_HELP_FEEDBACK: usize = 1042;
pub const ID_FILE_CLEAR_NVRAM: usize = 1005;
pub const ID_NVRAM_GLOBAL: usize = 1006;
pub const ID_NVRAM_PER_DISC: usize = 1007;
pub const ID_PATH_ADD_DISCS: usize = 1220;
pub const ID_SWITCH_BIOS: usize = 1225;

pub const ID_STATE_QUICK_SAVE: usize = 1070;
pub const ID_STATE_QUICK_LOAD: usize = 1071;
pub const ID_STATE_SAVE_BASE: usize = 1072; // 1072..1076 (Slots 1..5)
pub const ID_STATE_LOAD_BASE: usize = 1080; // 1080..1084 (Slots 1..5)

pub const ID_RECENT_IMAGE_BASE: usize = 1200;
pub const ID_RECENT_CLEAR: usize = 1210;
pub use crate::frontend::MAX_RECENT_IMAGES;

// Win32 Constants
const MF_STRING: u32 = 0x0000_0000;
const MF_POPUP: u32 = 0x0000_0010;
const MF_SEPARATOR: u32 = 0x0000_0800;
const MF_BYCOMMAND: u32 = 0x0000_0000;
const MF_BYPOSITION: u32 = 0x0000_0400;
const MF_CHECKED: u32 = 0x0000_0008;
const MF_UNCHECKED: u32 = 0x0000_0000;
const MF_GRAYED: u32 = 0x0000_0001;

const WM_COMMAND: u32 = 0x0111;
const WM_TIMER: u32 = 0x0113;
const WM_ENTERMENULOOP: u32 = 0x0211;
const WM_EXITMENULOOP: u32 = 0x0212;
const WM_ENTERSIZEMOVE: u32 = 0x0231;
const WM_EXITSIZEMOVE: u32 = 0x0232;
const RDW_INTERNALPAINT: u32 = 0x0002;
const MODAL_TIMER_ID: usize = 0x0CD1;
const MODAL_TIMER_MS: u32 = 10;
const WM_SETICON: u32 = 0x0080;
const ICON_SMALL: usize = 0;
const ICON_BIG: usize = 1;
const IMAGE_ICON: u32 = 1;
const LR_LOADFROMFILE: u32 = 0x0000_0010;
const LR_SHARED: u32 = 0x0000_8000;
const GWLP_WNDPROC: i32 = -4;

const MB_OK: u32 = 0x0000_0000;
const MB_YESNO: u32 = 0x0000_0004;
const MB_YESNOCANCEL: u32 = 0x0000_0003;
const MB_ICONINFORMATION: u32 = 0x0000_0040;
const MB_ICONQUESTION: u32 = 0x0000_0020;
const MB_ICONWARNING: u32 = 0x0000_0030;
const IDYES: i32 = 6;
const IDNO: i32 = 7;

#[repr(C)]
#[allow(non_snake_case)]
pub struct OPENFILENAMEW {
    pub lStructSize: u32,
    pub hwndOwner: *mut std::ffi::c_void,
    pub hInstance: *mut std::ffi::c_void,
    pub lpstrFilter: *const u16,
    pub lpstrCustomFilter: *mut u16,
    pub nMaxCustFilter: u32,
    pub nFilterIndex: u32,
    pub lpstrFile: *mut u16,
    pub nMaxFile: u32,
    pub lpstrFileTitle: *mut u16,
    pub nMaxFileTitle: u32,
    pub lpstrInitialDir: *const u16,
    pub lpstrTitle: *const u16,
    pub Flags: u32,
    pub nFileOffset: u16,
    pub nFileExtension: u16,
    pub lpstrDefExt: *const u16,
    pub lCustData: usize,
    pub lpfnHook: *mut std::ffi::c_void,
    pub lpTemplateName: *const u16,
    pub pvReserved: *mut std::ffi::c_void,
    pub dwReserved: u32,
    pub FlagsEx: u32,
}

#[repr(C)]
#[allow(non_snake_case)]
struct BROWSEINFOW {
    hwndOwner: *mut std::ffi::c_void,
    pidlRoot: *mut std::ffi::c_void,
    pszDisplayName: *mut u16,
    lpszTitle: *const u16,
    ulFlags: u32,
    lpfn: *mut std::ffi::c_void,
    lParam: isize,
    iImage: i32,
}

const BFFM_INITIALIZED: u32 = 1;
unsafe extern "system" fn activate_folder_picker(dialog: *mut std::ffi::c_void, message: u32, _lparam: isize, _data: isize) -> i32 {
    if message == BFFM_INITIALIZED {
        unsafe {
            ShowWindow(dialog, 5);
            BringWindowToTop(dialog);
            SetForegroundWindow(dialog);
            SetActiveWindow(dialog);
        }
    }
    0
}

#[link(name = "user32")]
unsafe extern "system" {
    fn CreateMenu() -> *mut std::ffi::c_void;
    fn CreatePopupMenu() -> *mut std::ffi::c_void;
    fn AppendMenuW(hmenu: *mut std::ffi::c_void, uflags: u32, uid: usize, lpitem: *const u16) -> i32;
    fn SetMenu(hwnd: *mut std::ffi::c_void, hmenu: *mut std::ffi::c_void) -> i32;
    fn DrawMenuBar(hwnd: *mut std::ffi::c_void) -> i32;
    fn CheckMenuItem(hmenu: *mut std::ffi::c_void, uid: u32, ucheck: u32) -> u32;
    fn MessageBoxW(hwnd: *mut std::ffi::c_void, lptext: *const u16, lpcaption: *const u16, utype: u32) -> i32;
    fn SetWindowLongPtrW(hwnd: *mut std::ffi::c_void, nindex: i32, dwnewlong: isize) -> isize;
    fn CallWindowProcW(prev: isize, hwnd: *mut std::ffi::c_void, msg: u32, wparam: usize, lparam: isize) -> isize;
    fn PostMessageW(hwnd: *mut std::ffi::c_void, msg: u32, wparam: usize, lparam: isize) -> i32;
    fn SetTimer(hwnd: *mut std::ffi::c_void, id: usize, elapse: u32, func: *mut std::ffi::c_void) -> usize;
    fn KillTimer(hwnd: *mut std::ffi::c_void, id: usize) -> i32;
    fn RedrawWindow(hwnd: *mut std::ffi::c_void, rect: *const std::ffi::c_void, region: *mut std::ffi::c_void, flags: u32) -> i32;
    fn EndMenu() -> i32;
    fn SendMessageW(hwnd: *mut std::ffi::c_void, msg: u32, wparam: usize, lparam: isize) -> isize;
    fn LoadImageW(hinst: *mut std::ffi::c_void, name: *const u16, utype: u32, cx: i32, cy: i32, fu_load: u32) -> *mut std::ffi::c_void;
    fn DeleteMenu(hmenu: *mut std::ffi::c_void, uposition: u32, uflags: u32) -> i32;
    fn GetMenuItemCount(hmenu: *mut std::ffi::c_void) -> i32;
    fn ShowWindow(hwnd: *mut std::ffi::c_void, command: i32) -> i32;
    fn BringWindowToTop(hwnd: *mut std::ffi::c_void) -> i32;
    fn SetForegroundWindow(hwnd: *mut std::ffi::c_void) -> i32;
    fn SetActiveWindow(hwnd: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
}

#[link(name = "comdlg32")]
unsafe extern "system" {
    fn GetOpenFileNameW(lpofn: *mut OPENFILENAMEW) -> i32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetLogicalDrives() -> u32;
    fn GetDriveTypeW(lpRootPathName: *const u16) -> u32;
    fn GetModuleHandleW(lpmodulename: *const u16) -> *mut std::ffi::c_void;
}

#[link(name = "shell32")]
unsafe extern "system" {
    fn ShellExecuteW(
        hwnd: *mut std::ffi::c_void,
        lpoperation: *const u16,
        lpfile: *const u16,
        lpparameters: *const u16,
        lpdirectory: *const u16,
        nshowcmd: i32,
    ) -> *mut std::ffi::c_void;
    fn SHBrowseForFolderW(info: *mut BROWSEINFOW) -> *mut std::ffi::c_void;
    fn SHGetPathFromIDListW(pidl: *mut std::ffi::c_void, path: *mut u16) -> i32;
}

#[link(name = "ole32")]
unsafe extern "system" { fn CoTaskMemFree(memory: *mut std::ffi::c_void); }

pub fn to_wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

pub fn get_optical_drives() -> Vec<char> {
    let mut drives = Vec::new();
    #[cfg(target_os = "windows")]
    unsafe {
        let mask = GetLogicalDrives();
        for i in 0..26 {
            if (mask & (1 << i)) != 0 {
                let letter = (b'A' + i as u8) as char;
                let root = format!("{}:\\", letter);
                let wide_root = to_wide(&root);
                if GetDriveTypeW(wide_root.as_ptr()) == 5 { // 5 = DRIVE_CDROM
                    drives.push(letter);
                }
            }
        }
    }
    drives
}

// Records an opened image and refreshes the Recent Disc Images menu
pub fn add_recent_image(path: &std::path::Path) {
    crate::frontend::add_recent_image(path);
    refresh_recent_menu();
}

pub fn refresh_recent_menu() {
    unsafe {
        if !RECENT_MENU_HANDLE.is_null() {
            rebuild_recent_menu_items(RECENT_MENU_HANDLE);
            if !OWNER_HWND.is_null() {
                DrawMenuBar(OWNER_HWND);
            }
        }
    }
}

pub fn set_window_icon(hwnd: *mut std::ffi::c_void) {
    if hwnd.is_null() { return; }
    unsafe {
        let hinstance = GetModuleHandleW(std::ptr::null());
        // Load embedded resource icon (ID 1 compiled by winres)
        let mut hicon_big = LoadImageW(hinstance, 1 as *const u16, IMAGE_ICON, 32, 32, LR_SHARED);
        let mut hicon_small = LoadImageW(hinstance, 1 as *const u16, IMAGE_ICON, 16, 16, LR_SHARED);

        // Fallback to loading icon.ico from current directory if needed
        if (hicon_big.is_null() || hicon_small.is_null()) && std::path::Path::new("icon.ico").exists() {
            let wide_ico = to_wide("icon.ico");
            if hicon_big.is_null() {
                hicon_big = LoadImageW(std::ptr::null_mut(), wide_ico.as_ptr(), IMAGE_ICON, 32, 32, LR_LOADFROMFILE | LR_SHARED);
            }
            if hicon_small.is_null() {
                hicon_small = LoadImageW(std::ptr::null_mut(), wide_ico.as_ptr(), IMAGE_ICON, 16, 16, LR_LOADFROMFILE | LR_SHARED);
            }
        }

        if !hicon_big.is_null() {
            SendMessageW(hwnd, WM_SETICON, ICON_BIG, hicon_big as isize);
        }
        if !hicon_small.is_null() {
            SendMessageW(hwnd, WM_SETICON, ICON_SMALL, hicon_small as isize);
        }
    }
}

static mut PREV_WNDPROC: isize = 0;
static mut MENU_HANDLE: *mut std::ffi::c_void = std::ptr::null_mut();
static mut OWNER_HWND: *mut std::ffi::c_void = std::ptr::null_mut();
static mut RECENT_MENU_HANDLE: *mut std::ffi::c_void = std::ptr::null_mut();

pub fn rebuild_recent_menu_items(menu: *mut std::ffi::c_void) {
    if menu.is_null() { return; }
    unsafe {
        let count = GetMenuItemCount(menu);
        for _ in 0..count {
            DeleteMenu(menu, 0, MF_BYPOSITION);
        }
        let list = crate::frontend::recent_images();
        if list.is_empty() {
            AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, to_wide("(No recent disc images)").as_ptr());
        } else {
            for (i, p) in list.iter().enumerate().take(MAX_RECENT_IMAGES) {
                let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.to_string_lossy().to_string());
                let label = format!("&{} {}", (i + 1) % 10, name);
                AppendMenuW(menu, MF_STRING, ID_RECENT_IMAGE_BASE + i, to_wide(&label).as_ptr());
            }
            AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
            AppendMenuW(menu, MF_STRING, ID_RECENT_CLEAR, to_wide("Clear Recent Images History").as_ptr());
        }
    }
}

// --run-in-menu: menus, dialogs and move/size loops block winit's loop, so a timer drives emulation through WM_PAINT
static RUN_IN_MODAL: AtomicBool = AtomicBool::new(false);
static MODAL_DEPTH: AtomicU32 = AtomicU32::new(0);
static TIMER_PAINT: AtomicBool = AtomicBool::new(false);

pub fn set_run_in_modal(on: bool) {
    RUN_IN_MODAL.store(on, Ordering::Relaxed);
}

// True while a Win32 modal loop (menu, dialog, move/size) is running
pub fn modal_active() -> bool {
    MODAL_DEPTH.load(Ordering::Relaxed) > 0
}

// True once per WM_PAINT that the modal timer asked for
pub fn take_timer_paint() -> bool {
    TIMER_PAINT.swap(false, Ordering::Relaxed)
}

// Closes an open menu so a quit requested during the menu loop takes effect
pub fn end_menu() {
    unsafe {
        EndMenu();
    }
}

fn modal_begin(hwnd: *mut std::ffi::c_void) {
    if MODAL_DEPTH.fetch_add(1, Ordering::Relaxed) == 0 && RUN_IN_MODAL.load(Ordering::Relaxed) {
        unsafe {
            SetTimer(hwnd, MODAL_TIMER_ID, MODAL_TIMER_MS, std::ptr::null_mut());
        }
    }
}

fn modal_end(hwnd: *mut std::ffi::c_void) {
    let prev = MODAL_DEPTH.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |d| Some(d.saturating_sub(1))).unwrap_or(0);
    if prev == 1 {
        unsafe {
            KillTimer(hwnd, MODAL_TIMER_ID);
        }
    }
}

// Runs a modal dialog with the modal timer active
pub fn modal<R>(hwnd: *mut std::ffi::c_void, f: impl FnOnce() -> R) -> R {
    modal_begin(hwnd);
    let r = f();
    modal_end(hwnd);
    r
}

pub fn update_input_device_menu(mode: InputDeviceMode) {
    unsafe {
        if !MENU_HANDLE.is_null() {
            CheckMenuItem(
                MENU_HANDLE,
                ID_INPUT_DEVICE_GAMEPAD as u32,
                MF_BYCOMMAND | if mode == InputDeviceMode::Gamepad { MF_CHECKED } else { MF_UNCHECKED },
            );
            CheckMenuItem(
                MENU_HANDLE,
                ID_INPUT_DEVICE_MOUSE as u32,
                MF_BYCOMMAND | if mode == InputDeviceMode::Mouse { MF_CHECKED } else { MF_UNCHECKED },
            );
            if !OWNER_HWND.is_null() {
                DrawMenuBar(OWNER_HWND);
            }
        }
    }
}

pub fn update_profile_menu(profile: InputProfile) {
    unsafe {
        if !MENU_HANDLE.is_null() {
            CheckMenuItem(
                MENU_HANDLE,
                ID_PROFILE_STANDARD as u32,
                MF_BYCOMMAND | if profile == InputProfile::Standard { MF_CHECKED } else { MF_UNCHECKED },
            );
            CheckMenuItem(
                MENU_HANDLE,
                ID_PROFILE_WASD as u32,
                MF_BYCOMMAND | if profile == InputProfile::Wasd { MF_CHECKED } else { MF_UNCHECKED },
            );
            CheckMenuItem(
                MENU_HANDLE,
                ID_PROFILE_NUMPAD as u32,
                MF_BYCOMMAND | if profile == InputProfile::Numpad { MF_CHECKED } else { MF_UNCHECKED },
            );
            CheckMenuItem(
                MENU_HANDLE,
                ID_PROFILE_CUSTOM as u32,
                MF_BYCOMMAND | if profile == InputProfile::Custom { MF_CHECKED } else { MF_UNCHECKED },
            );
            if !OWNER_HWND.is_null() {
                DrawMenuBar(OWNER_HWND);
            }
        }
    }
}

pub fn update_video_menu(mode: crate::mcd212::VideoMode) {
    unsafe {
        if !MENU_HANDLE.is_null() {
            CheckMenuItem(
                MENU_HANDLE,
                ID_VIDEO_PAL as u32,
                MF_BYCOMMAND | if mode == crate::mcd212::VideoMode::Pal { MF_CHECKED } else { MF_UNCHECKED },
            );
            CheckMenuItem(
                MENU_HANDLE,
                ID_VIDEO_NTSC as u32,
                MF_BYCOMMAND | if mode == crate::mcd212::VideoMode::Ntsc { MF_CHECKED } else { MF_UNCHECKED },
            );
            if !OWNER_HWND.is_null() {
                DrawMenuBar(OWNER_HWND);
            }
        }
    }
}

pub fn update_waitstates_menu(enabled: bool) {
    unsafe {
        if !MENU_HANDLE.is_null() {
            CheckMenuItem(
                MENU_HANDLE,
                ID_EMU_WAITSTATES as u32,
                MF_BYCOMMAND | if enabled { MF_CHECKED } else { MF_UNCHECKED },
            );
            if !OWNER_HWND.is_null() {
                DrawMenuBar(OWNER_HWND);
            }
        }
    }
}

pub fn update_dvc_menu(fitted: bool) {
    unsafe {
        if !MENU_HANDLE.is_null() {
            CheckMenuItem(
                MENU_HANDLE,
                ID_EMU_DVC as u32,
                MF_BYCOMMAND | if fitted { MF_CHECKED } else { MF_UNCHECKED },
            );
            if !OWNER_HWND.is_null() {
                DrawMenuBar(OWNER_HWND);
            }
        }
    }
}

unsafe extern "system" fn custom_wndproc(
    hwnd: *mut std::ffi::c_void,
    msg: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    match msg {
        WM_ENTERMENULOOP | WM_ENTERSIZEMOVE => modal_begin(hwnd),
        WM_EXITMENULOOP | WM_EXITSIZEMOVE => modal_end(hwnd),
        WM_TIMER if wparam == MODAL_TIMER_ID => {
            TIMER_PAINT.store(true, Ordering::Relaxed);
            unsafe {
                RedrawWindow(hwnd, std::ptr::null(), std::ptr::null_mut(), RDW_INTERNALPAINT);
            }
            return 0;
        }
        _ => {}
    }
    if msg == WM_COMMAND {
        let menu_id = (wparam & 0xFFFF) as usize;
        if menu_id >= ID_PHYSICAL_DRIVE_BOOT_BASE && menu_id < ID_PHYSICAL_DRIVE_BOOT_BASE + 26 {
            let letter = (b'A' + (menu_id - ID_PHYSICAL_DRIVE_BOOT_BASE) as u8) as char;
            push_action(GuiAction::OpenDisc(PathBuf::from(format!("{}:", letter))));
            return 0;
        }
        if menu_id >= ID_PHYSICAL_DRIVE_SWAP_BASE && menu_id < ID_PHYSICAL_DRIVE_SWAP_BASE + 26 {
            let letter = (b'A' + (menu_id - ID_PHYSICAL_DRIVE_SWAP_BASE) as u8) as char;
            push_action(GuiAction::InsertDisc(PathBuf::from(format!("{}:", letter))));
            return 0;
        }
        if menu_id >= ID_RECENT_IMAGE_BASE && menu_id < ID_RECENT_IMAGE_BASE + MAX_RECENT_IMAGES {
            let idx = menu_id - ID_RECENT_IMAGE_BASE;
            let list = crate::frontend::recent_images();
            if let Some(path) = list.get(idx) {
                push_action(GuiAction::OpenDisc(path.clone()));
            }
            return 0;
        }
        if menu_id == ID_RECENT_CLEAR {
            crate::frontend::clear_recent_images();
            refresh_recent_menu();
            return 0;
        }
        if menu_id >= ID_STATE_SAVE_BASE && menu_id < ID_STATE_SAVE_BASE + 5 {
            let slot = (menu_id - ID_STATE_SAVE_BASE) + 1;
            push_action(GuiAction::SaveState(slot));
            return 0;
        }
        if menu_id >= ID_STATE_LOAD_BASE && menu_id < ID_STATE_LOAD_BASE + 5 {
            let slot = (menu_id - ID_STATE_LOAD_BASE) + 1;
            push_action(GuiAction::LoadState(slot));
            return 0;
        }
        match menu_id {
            ID_STATE_QUICK_SAVE => {
                push_action(GuiAction::SaveState(1));
                return 0;
            }
            ID_STATE_QUICK_LOAD => {
                push_action(GuiAction::LoadState(1));
                return 0;
            }
            ID_FILE_INSERT => {
                if let Some(path) = modal(hwnd, || open_file_dialog(hwnd)) {
                    push_action(GuiAction::InsertDisc(path));
                }
                return 0;
            }
            ID_FILE_OPEN => {
                if let Some(path) = modal(hwnd, || open_file_dialog(hwnd)) {
                    push_action(GuiAction::OpenDisc(path));
                }
                return 0;
            }
            ID_FILE_EJECT => {
                push_action(GuiAction::EjectDisc);
                return 0;
            }
            ID_FILE_RESET => {
                push_action(GuiAction::Reset);
                return 0;
            }
            ID_FILE_EXIT => {
                push_action(GuiAction::Exit);
                return 0;
            }
            ID_PROFILE_STANDARD => {
                push_action(GuiAction::SetProfile(InputProfile::Standard));
                return 0;
            }
            ID_PROFILE_WASD => {
                push_action(GuiAction::SetProfile(InputProfile::Wasd));
                return 0;
            }
            ID_PROFILE_NUMPAD => {
                push_action(GuiAction::SetProfile(InputProfile::Numpad));
                return 0;
            }
            ID_PROFILE_CUSTOM => {
                push_action(GuiAction::SetProfile(InputProfile::Custom));
                return 0;
            }
            ID_INPUT_DEVICE_GAMEPAD => {
                push_action(GuiAction::SetInputDevice(InputDeviceMode::Gamepad));
                return 0;
            }
            ID_INPUT_DEVICE_MOUSE => {
                push_action(GuiAction::SetInputDevice(InputDeviceMode::Mouse));
                return 0;
            }
            ID_CONTROLS_EDIT_INI => {
                let _ = std::process::Command::new("notepad.exe").arg(crate::frontend::settings_file_path()).spawn();
                push_action(GuiAction::SetProfile(InputProfile::Custom));
                return 0;
            }
            ID_CONTROLS_RELOAD_INI => {
                push_action(GuiAction::ReloadControls);
                return 0;
            }
            ID_CONTROLS_CONFIGURE => {
                push_action(GuiAction::OpenControllerConfig);
                return 0;
            }
            ID_CONTROLS_INFO => {
                modal(hwnd, || show_controls_dialog(hwnd));
                return 0;
            }
            ID_CONTROLS_RELEASE_MOUSE => {
                push_action(GuiAction::ToggleReleaseMouse);
                return 0;
            }
            ID_AUDIO_MUTE => {
                push_action(GuiAction::ToggleMute);
                return 0;
            }
            ID_AUDIO_VOL_UP => {
                push_action(GuiAction::VolumeUp);
                return 0;
            }
            ID_AUDIO_VOL_DOWN => {
                push_action(GuiAction::VolumeDown);
                return 0;
            }
            ID_VIDEO_FULLSCREEN => {
                push_action(GuiAction::ToggleFullscreen);
                return 0;
            }
            ID_DISPLAY_BORDERLESS => { push_action(GuiAction::SetDisplayMode(DisplayMode::BorderlessFullscreen)); return 0; }
            ID_DISPLAY_DECORATED => { push_action(GuiAction::SetDisplayMode(DisplayMode::DecoratedFullscreen)); return 0; }
            ID_DISPLAY_WINDOWED => { push_action(GuiAction::SetDisplayMode(DisplayMode::Windowed)); return 0; }
            ID_SKIP_SHELL => { push_action(GuiAction::ToggleSkipShell); return 0; }
            ID_VIDEO_SCREENSHOT => {
                push_action(GuiAction::TakeScreenshot);
                return 0;
            }
            ID_VIDEO_PAL => {
                push_action(GuiAction::SetVideoMode(crate::mcd212::VideoMode::Pal));
                return 0;
            }
            ID_VIDEO_NTSC => {
                push_action(GuiAction::SetVideoMode(crate::mcd212::VideoMode::Ntsc));
                return 0;
            }
            ID_EMU_WAITSTATES => {
                push_action(GuiAction::ToggleWaitStates);
                return 0;
            }
            ID_EMU_DVC => {
                push_action(GuiAction::ToggleDvc);
                return 0;
            }
            ID_HELP_CONTROLS => {
                modal(hwnd, || show_controls_dialog(hwnd));
                return 0;
            }
            ID_FILE_CLEAR_NVRAM => {
                let confirm = unsafe {
                    MessageBoxW(
                        hwnd,
                        to_wide("Are you sure you want to clear console memory (NVRAM)?\n\nAll saved games in NVRAM will be erased and the console will be reset to factory defaults.").as_ptr(),
                        to_wide("OpenCDi - Clear Console Memory").as_ptr(),
                        MB_YESNO | MB_ICONQUESTION,
                    )
                };
                if confirm == IDYES {
                    push_action(GuiAction::ClearNvram);
                }
                return 0;
            }
            ID_NVRAM_GLOBAL => { push_action(GuiAction::SetNvramMode(NvramMode::Global)); return 0; }
            ID_NVRAM_PER_DISC => { push_action(GuiAction::SetNvramMode(NvramMode::PerDisc)); return 0; }
            ID_PATH_ADD_DISCS => {
                if let Some(path) = modal(hwnd, || select_folder_dialog(hwnd, "Choose a folder to scan for disc images")) { push_action(GuiAction::AddDiscPath(path)); }
                return 0;
            }
            ID_SWITCH_BIOS => {
                if let Some(path) = modal(hwnd, || open_bios_dialog(hwnd)) { push_action(GuiAction::SwitchBios(path)); }
                return 0;
            }
            ID_HELP_FEEDBACK => {
                open_feedback_url();
                return 0;
            }
            ID_HELP_ABOUT => {
                modal(hwnd, || show_about_dialog(hwnd));
                return 0;
            }
            _ => {}
        }
    }

    unsafe {
        if PREV_WNDPROC != 0 {
            CallWindowProcW(PREV_WNDPROC, hwnd, msg, wparam, lparam)
        } else {
            0
        }
    }
}

// Queue a menu command so its modal dialog runs from the window procedure, outside winit's event handler
pub fn post_menu_command(hwnd: *mut std::ffi::c_void, menu_id: usize) {
    if !hwnd.is_null() {
        unsafe {
            PostMessageW(hwnd, WM_COMMAND, menu_id, 0);
        }
    }
}

pub fn attach_gui_menu(
    hwnd: *mut std::ffi::c_void,
    current_profile: InputProfile,
    current_device_mode: InputDeviceMode,
    current_video_mode: crate::mcd212::VideoMode,
) {
    if hwnd.is_null() {
        return;
    }

    unsafe {
        OWNER_HWND = hwnd;

        let main_menu = CreateMenu();
        if main_menu.is_null() {
            return;
        }
        MENU_HANDLE = main_menu;

        // 1. File Menu
        let file_menu = CreatePopupMenu();
        AppendMenuW(file_menu, MF_STRING, ID_FILE_INSERT, to_wide("Insert Disc Image (Hot-swap)...\tCtrl+I").as_ptr());
        AppendMenuW(file_menu, MF_STRING, ID_FILE_OPEN, to_wide("Open Disc Image && Restart Console...\tCtrl+O").as_ptr());

        // Submenu: Recent Disc Images
        let recent_menu = CreatePopupMenu();
        RECENT_MENU_HANDLE = recent_menu;
        rebuild_recent_menu_items(recent_menu);
        AppendMenuW(file_menu, MF_POPUP, recent_menu as usize, to_wide("Recent Disc Images").as_ptr());

        AppendMenuW(file_menu, MF_STRING, ID_FILE_EJECT, to_wide("Eject Disc").as_ptr());

        let optical_drives = get_optical_drives();
        if !optical_drives.is_empty() {
            AppendMenuW(file_menu, MF_SEPARATOR, 0, std::ptr::null());
            let drives_submenu = CreatePopupMenu();
            for &letter in &optical_drives {
                let boot_id = ID_PHYSICAL_DRIVE_BOOT_BASE + (letter as usize - 'A' as usize);
                let swap_id = ID_PHYSICAL_DRIVE_SWAP_BASE + (letter as usize - 'A' as usize);

                let single_drive_menu = CreatePopupMenu();
                AppendMenuW(
                    single_drive_menu,
                    MF_STRING,
                    boot_id,
                    to_wide(&format!("Boot Real Disc ({}:) & Restart", letter)).as_ptr(),
                );
                AppendMenuW(
                    single_drive_menu,
                    MF_STRING,
                    swap_id,
                    to_wide(&format!("Hot-swap Real Disc ({}:)", letter)).as_ptr(),
                );

                AppendMenuW(
                    drives_submenu,
                    MF_POPUP,
                    single_drive_menu as usize,
                    to_wide(&format!("Optical Drive ({}:)", letter)).as_ptr(),
                );
            }
            AppendMenuW(file_menu, MF_POPUP, drives_submenu as usize, to_wide("Real Optical CD Drives").as_ptr());
        }

        AppendMenuW(file_menu, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(file_menu, MF_STRING, ID_STATE_QUICK_SAVE, to_wide("Quick Save State (Slot 1)\tF6").as_ptr());
        AppendMenuW(file_menu, MF_STRING, ID_STATE_QUICK_LOAD, to_wide("Quick Load State (Slot 1)\tF7").as_ptr());

        let save_state_menu = CreatePopupMenu();
        let load_state_menu = CreatePopupMenu();
        for slot in 1..=5 {
            let save_label = if slot == 1 {
                format!("Slot &{} (Quick Save)\tF6", slot)
            } else {
                format!("Slot &{}", slot)
            };
            let load_label = if slot == 1 {
                format!("Slot &{} (Quick Load)\tF7", slot)
            } else {
                format!("Slot &{}", slot)
            };
            AppendMenuW(save_state_menu, MF_STRING, ID_STATE_SAVE_BASE + (slot - 1), to_wide(&save_label).as_ptr());
            AppendMenuW(load_state_menu, MF_STRING, ID_STATE_LOAD_BASE + (slot - 1), to_wide(&load_label).as_ptr());
        }
        AppendMenuW(file_menu, MF_POPUP, save_state_menu as usize, to_wide("Save State to Slot").as_ptr());
        AppendMenuW(file_menu, MF_POPUP, load_state_menu as usize, to_wide("Load State from Slot").as_ptr());

        AppendMenuW(file_menu, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(file_menu, MF_STRING, ID_FILE_CLEAR_NVRAM, to_wide("Clear Console Memory (Reset NVRAM)...").as_ptr());
        let nvram_menu = CreatePopupMenu();
        AppendMenuW(nvram_menu, MF_STRING, ID_NVRAM_GLOBAL, to_wide("One Global NVRAM File").as_ptr());
        AppendMenuW(nvram_menu, MF_STRING, ID_NVRAM_PER_DISC, to_wide("Separate NVRAM per Disc").as_ptr());
        AppendMenuW(file_menu, MF_POPUP, nvram_menu as usize, to_wide("NVRAM Storage Mode").as_ptr());
        AppendMenuW(file_menu, MF_STRING, ID_FILE_RESET, to_wide("Restart Console (Reset)\tF5 / H").as_ptr());
        AppendMenuW(file_menu, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(file_menu, MF_STRING, ID_FILE_EXIT, to_wide("Exit\tEsc").as_ptr());
        AppendMenuW(main_menu, MF_POPUP, file_menu as usize, to_wide("File").as_ptr());

        let settings_menu = CreatePopupMenu();
        AppendMenuW(settings_menu, MF_STRING, ID_PATH_ADD_DISCS, to_wide("Add Disc Folder...").as_ptr());
        AppendMenuW(settings_menu, MF_STRING, ID_SWITCH_BIOS, to_wide("Switch BIOS...").as_ptr());
        AppendMenuW(main_menu, MF_POPUP, settings_menu as usize, to_wide("Settings").as_ptr());

        // 2. Controls Menu
        let controls_menu = CreatePopupMenu();

        let device_menu = CreatePopupMenu();
        AppendMenuW(device_menu, MF_STRING, ID_INPUT_DEVICE_GAMEPAD, to_wide("CD-i Gamepad (USB Controller / Keyboard)\tF9").as_ptr());
        AppendMenuW(device_menu, MF_STRING, ID_INPUT_DEVICE_MOUSE, to_wide("Philips CD-i Mouse (Host Cursor)\tF9").as_ptr());
        AppendMenuW(controls_menu, MF_POPUP, device_menu as usize, to_wide("Input Device (Exclusive)").as_ptr());
        AppendMenuW(controls_menu, MF_SEPARATOR, 0, std::ptr::null());

        AppendMenuW(controls_menu, MF_STRING, ID_PROFILE_STANDARD, to_wide("Key Profile: Standard (Arrow Keys + Z / X)").as_ptr());
        AppendMenuW(controls_menu, MF_STRING, ID_PROFILE_WASD, to_wide("Key Profile: WASD + J / K (PC Style)").as_ptr());
        AppendMenuW(controls_menu, MF_STRING, ID_PROFILE_NUMPAD, to_wide("Key Profile: Numpad (8 4 6 2 + 1 / 0)").as_ptr());
        AppendMenuW(controls_menu, MF_STRING, ID_PROFILE_CUSTOM, to_wide("Key Profile: Custom (opencdi_controls.ini)").as_ptr());
        AppendMenuW(controls_menu, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(controls_menu, MF_STRING, ID_CONTROLS_CONFIGURE, to_wide("Configure CD-i Controller...").as_ptr());
        AppendMenuW(controls_menu, MF_STRING, ID_CONTROLS_EDIT_INI, to_wide("Edit Controls in Notepad...").as_ptr());
        AppendMenuW(controls_menu, MF_STRING, ID_CONTROLS_RELOAD_INI, to_wide("Reload Controls from INI").as_ptr());
        AppendMenuW(controls_menu, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(controls_menu, MF_STRING, ID_CONTROLS_RELEASE_MOUSE, to_wide("Release / Capture Mouse\tMiddle Click").as_ptr());
        AppendMenuW(controls_menu, MF_STRING, ID_CONTROLS_INFO, to_wide("View Controls Configuration...").as_ptr());
        AppendMenuW(main_menu, MF_POPUP, controls_menu as usize, to_wide("Controls").as_ptr());

        // 3. Audio Menu
        let audio_menu = CreatePopupMenu();
        AppendMenuW(audio_menu, MF_STRING, ID_AUDIO_MUTE, to_wide("Mute / Unmute Audio\tM").as_ptr());
        AppendMenuW(audio_menu, MF_STRING, ID_AUDIO_VOL_UP, to_wide("Volume Up (+)\t+").as_ptr());
        AppendMenuW(audio_menu, MF_STRING, ID_AUDIO_VOL_DOWN, to_wide("Volume Down (-)\t-").as_ptr());
        AppendMenuW(main_menu, MF_POPUP, audio_menu as usize, to_wide("Audio").as_ptr());

        // 4. Video Menu
        let video_menu = CreatePopupMenu();
        AppendMenuW(video_menu, MF_STRING, ID_VIDEO_FULLSCREEN, to_wide("Cycle Display Mode\tF11 / Alt+Enter").as_ptr());
        let display_menu = CreatePopupMenu();
        AppendMenuW(display_menu, MF_STRING, ID_DISPLAY_BORDERLESS, to_wide("Borderless Fullscreen (Default)").as_ptr());
        AppendMenuW(display_menu, MF_STRING, ID_DISPLAY_DECORATED, to_wide("Fullscreen with Border and Menu").as_ptr());
        AppendMenuW(display_menu, MF_STRING, ID_DISPLAY_WINDOWED, to_wide("Windowed").as_ptr());
        AppendMenuW(video_menu, MF_POPUP, display_menu as usize, to_wide("Display Mode").as_ptr());
        AppendMenuW(video_menu, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(video_menu, MF_STRING | MF_CHECKED, ID_SKIP_SHELL, to_wide("Skip Philips Shell and Auto-start Disc").as_ptr());
        AppendMenuW(video_menu, MF_STRING, ID_VIDEO_SCREENSHOT, to_wide("Save Screenshot\tF12").as_ptr());
        AppendMenuW(video_menu, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(video_menu, MF_STRING, ID_VIDEO_PAL, to_wide("PAL (50 Hz) [CD-i 220 Standard]").as_ptr());
        AppendMenuW(video_menu, MF_STRING, ID_VIDEO_NTSC, to_wide("NTSC (60 Hz)").as_ptr());
        AppendMenuW(video_menu, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(video_menu, MF_STRING | MF_CHECKED, ID_EMU_WAITSTATES, to_wide("Authentic Hardware Speed (Wait-States)").as_ptr());
        AppendMenuW(video_menu, MF_STRING | MF_CHECKED, ID_EMU_DVC, to_wide("Digital Video Cartridge (DVC) [vmpega.rom]").as_ptr());
        AppendMenuW(main_menu, MF_POPUP, video_menu as usize, to_wide("Video").as_ptr());

        // 5. Help Menu
        let help_menu = CreatePopupMenu();
        AppendMenuW(help_menu, MF_STRING, ID_HELP_CONTROLS, to_wide("Controls Guide...").as_ptr());
        AppendMenuW(help_menu, MF_STRING, ID_HELP_ABOUT, to_wide("About OpenCDi...").as_ptr());
        AppendMenuW(main_menu, MF_POPUP, help_menu as usize, to_wide("Help").as_ptr());

        // 6. Prominent Top-Level Send Feedback Button
        AppendMenuW(main_menu, MF_STRING, ID_HELP_FEEDBACK, to_wide("Send Feedback (F1)").as_ptr());

        // The application draws its own toolbar; keep the native menu detached.
        SetMenu(hwnd, std::ptr::null_mut());
        DrawMenuBar(hwnd);
        set_window_icon(hwnd);

        update_input_device_menu(current_device_mode);
        update_profile_menu(current_profile);
        update_video_menu(current_video_mode);

        // Subclass window procedure
        let prev = SetWindowLongPtrW(hwnd, GWLP_WNDPROC, custom_wndproc as *const () as usize as isize);
        PREV_WNDPROC = prev;
        info!("Win32 Native Menu attached and window subclassed successfully.");
    }
}

pub fn set_menu_visible(hwnd: *mut std::ffi::c_void, _visible: bool) {
    if hwnd.is_null() { return; }
    unsafe {
        SetMenu(hwnd, std::ptr::null_mut());
        DrawMenuBar(hwnd);
    }
}

pub fn update_display_mode_menu(mode: DisplayMode) {
    unsafe {
        if MENU_HANDLE.is_null() { return; }
        CheckMenuItem(MENU_HANDLE, ID_DISPLAY_BORDERLESS as u32, MF_BYCOMMAND | if mode == DisplayMode::BorderlessFullscreen { MF_CHECKED } else { MF_UNCHECKED });
        CheckMenuItem(MENU_HANDLE, ID_DISPLAY_DECORATED as u32, MF_BYCOMMAND | if mode == DisplayMode::DecoratedFullscreen { MF_CHECKED } else { MF_UNCHECKED });
        CheckMenuItem(MENU_HANDLE, ID_DISPLAY_WINDOWED as u32, MF_BYCOMMAND | if mode == DisplayMode::Windowed { MF_CHECKED } else { MF_UNCHECKED });
    }
}

pub fn update_skip_shell_menu(enabled: bool) {
    unsafe {
        if !MENU_HANDLE.is_null() {
            CheckMenuItem(MENU_HANDLE, ID_SKIP_SHELL as u32, MF_BYCOMMAND | if enabled { MF_CHECKED } else { MF_UNCHECKED });
        }
    }
}

pub fn update_nvram_mode_menu(mode: NvramMode) {
    unsafe {
        if MENU_HANDLE.is_null() { return; }
        CheckMenuItem(MENU_HANDLE, ID_NVRAM_GLOBAL as u32, MF_BYCOMMAND | if mode == NvramMode::Global { MF_CHECKED } else { MF_UNCHECKED });
        CheckMenuItem(MENU_HANDLE, ID_NVRAM_PER_DISC as u32, MF_BYCOMMAND | if mode == NvramMode::PerDisc { MF_CHECKED } else { MF_UNCHECKED });
    }
}

pub fn open_file_dialog(owner: *mut std::ffi::c_void) -> Option<PathBuf> {
    choose_file(owner, "Select CD-i Disc Image (CHD / ISO / BIN / CUE)",
        "CD-i Disc Images", "*.chd;*.iso;*.bin;*.cue", None)
}

pub fn open_bios_dialog(owner: *mut std::ffi::c_void) -> Option<PathBuf> {
    let folder = crate::frontend::app_data_dir().join("bios");
    choose_file(owner, "Select a Philips CD-i System BIOS", "BIOS ROM files",
        "*.rom;*.bin;*.zip", Some(&folder))
}

fn choose_file(owner: *mut std::ffi::c_void, title: &str, label: &str,
    extensions: &str, folder: Option<&std::path::Path>) -> Option<PathBuf> {
    let mut file_buf = [0u16; 1024];
    let filter = to_wide(&format!("{label}\0{extensions}\0All Files (*.*)\0*.*\0"));
    let title = to_wide(title);
    let folder = folder.map(|path| path.as_os_str().encode_wide().chain(std::iter::once(0)).collect::<Vec<_>>());

    let mut ofn: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    ofn.hwndOwner = owner;
    ofn.lpstrFilter = filter.as_ptr();
    ofn.lpstrInitialDir = folder.as_ref().map_or(std::ptr::null(), |path| path.as_ptr());
    ofn.lpstrFile = file_buf.as_mut_ptr();
    ofn.nMaxFile = file_buf.len() as u32;
    ofn.lpstrTitle = title.as_ptr();
    ofn.Flags = 0x00000800 | 0x00001000 | 0x00000004 | 0x00000008; // OFN_PATHMUSTEXIST | OFN_FILEMUSTEXIST | OFN_HIDEREADONLY | OFN_NOCHANGEDIR

    unsafe {
        if GetOpenFileNameW(&mut ofn) != 0 {
            let len = file_buf.iter().position(|&c| c == 0).unwrap_or(file_buf.len());
            let os_string = OsString::from_wide(&file_buf[..len]);
            Some(PathBuf::from(os_string))
        } else {
            None
        }
    }
}

pub fn select_folder_dialog(owner: *mut std::ffi::c_void, title: &str) -> Option<PathBuf> {
    let title = to_wide(title);
    let mut display_name = [0u16; 260];
    unsafe { SetForegroundWindow(owner); }
    let mut info = BROWSEINFOW {
        hwndOwner: owner,
        pidlRoot: std::ptr::null_mut(),
        pszDisplayName: display_name.as_mut_ptr(),
        lpszTitle: title.as_ptr(),
        ulFlags: 0x0001 | 0x0010 | 0x0040, // filesystem folders, edit box, modern picker
        lpfn: activate_folder_picker as *const () as *mut std::ffi::c_void,
        lParam: 0,
        iImage: 0,
    };
    let pidl = unsafe { SHBrowseForFolderW(&mut info) };
    if pidl.is_null() { return None; }
    let mut path = vec![0u16; 32768];
    let ok = unsafe { SHGetPathFromIDListW(pidl, path.as_mut_ptr()) };
    unsafe { CoTaskMemFree(pidl); }
    if ok == 0 { return None; }
    let len = path.iter().position(|&c| c == 0).unwrap_or(path.len());
    Some(PathBuf::from(OsString::from_wide(&path[..len])))
}

pub fn show_controls_dialog(owner: *mut std::ffi::c_void) {
    let text = to_wide(
        "OpenCDi - Philips CD-i Controls Configuration\n\
        =================================================\n\n\
        Quick control profiles available under the 'Controls' menu:\n\n\
        1. STANDARD PROFILE (Default):\n\
           • Directional Movement / D-Pad: Arrow Keys\n\
           • Button 1 (Action / Confirm): [Z]\n\
           • Button 2 (Jump / Action 2): [X]\n\n\
        2. WASD PROFILE (PC Style):\n\
           • Movement: [W] [A] [S] [D]\n\
           • Button 1: [J]  |  Button 2: [K]\n\n\
        3. NUMPAD PROFILE:\n\
           • Movement: [8] [4] [6] [2]\n\
           • Button 1: [1]  |  Button 2: [0]\n\n\
        4. CUSTOM PROFILE:\n\
           • Configurable via 'Controls -> Edit Controls in Notepad...'\n\
           • Map any key (e.g. Space, Enter, Shift, etc.) in opencdi_controls.ini\n\n\
        MOUSE & TOUCH:\n\
           • Move host mouse to move the on-screen pointer or cursor.\n\
           • Left Click = Button 1  |  Right Click = Button 2\n\
           • Middle Click: Lock / release host mouse cursor to/from the window.\n\n\
        USB GAMEPAD / XBOX / PLAYSTATION:\n\
           • D-Pad or Left Analog Stick = Movement\n\
           • Button A / Cross = Button 1\n\
           • Button B / Circle / X = Button 2\n\
           • Start Button = Action / Pause\n\n\
        KEYBOARD SHORTCUTS:\n\
           • F11 or Alt+Enter: Cycle display modes (borderless, decorated, windowed)\n\
           • F5 or H: Restart console (Reset)\n\
           • F6: Quick Save State (Slot 1)  |  F7: Quick Load State (Slot 1)\n\
           • F9: Switch Active Input Device (Gamepad <-> Mouse)\n\
           • P: Pause  |  Space (hold): Fast-forward\n\
           • F8: Screenshot (PPM)  |  F12: Screenshot (BMP)\n\
           • Ctrl+I: Insert disc  |  Ctrl+O: Open disc image and restart\n\
           • M: Mute / Unmute audio  |  + / - : Adjust volume\n\
           • Esc: Quit"
    );
    let title = to_wide("OpenCDi - Controls & Key Mapping Guide");
    unsafe {
        MessageBoxW(owner, text.as_ptr(), title.as_ptr(), MB_OK | MB_ICONINFORMATION);
    }
}

// Warning box for a disc image or drive that failed to load
pub fn show_disc_error(owner: *mut std::ffi::c_void, text: &str) {
    let text = to_wide(text);
    let title = to_wide("OpenCDi - Errore Lettura Disco");
    modal(owner, || unsafe {
        MessageBoxW(owner, text.as_ptr(), title.as_ptr(), MB_OK | MB_ICONWARNING);
    });
}

pub fn show_about_dialog(owner: *mut std::ffi::c_void) {
    let text = to_wide(
        "OpenCDi - Philips CD-i Emulator\n\
        Version: 0.2.0 (High Performance Edition)\n\
        Emulator Core by sigsantilio and Arnold de Ruiter\n\
        Built with Rust, winit & softbuffer\n\n\
        Key Emulation Features:\n\
        • Philips SCC68070 Microprocessor & OS-9 CD-RTOS kernel\n\
        • Philips MCD-212 Video Display Controller (Planes A & B, ICA/DCA display lists)\n\
        • CDIC CD-i Controller & ADPCM Audio Decoder (Levels A, B, C stereo/mono)\n\
        • Motorola 68HC05 Slave Microcontroller (HLE keyboard, mouse, RTC, NVRAM)\n\
        • Digital Video Cartridge (DVC / MPEG-1) for Full Motion Video titles\n\
        • Win32 GUI menu with disc hot-swapping and configurable controls\n\
        • Direct USB gamepad (XInput / DirectInput) and precise mouse capture\n\n\
        Special Thanks & Acknowledgements:\n\
        • The MAME Team (BSD-3-Clause CD-i drivers: cdi, mcd212, cdicdic, scc68070)\n\
        • The CDi_MiSTer project (ash2020) for FPGA hardware research\n\
        • The CD-i preservation community"
    );
    let title = to_wide("About OpenCDi");
    unsafe {
        MessageBoxW(owner, text.as_ptr(), title.as_ptr(), MB_OK | MB_ICONINFORMATION);
    }
}

pub fn open_feedback_url() {
    open_web_url("https://cdi-tester-feedback.pages.dev/");
}

pub fn open_web_url(url: &str) {
    unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            to_wide("open").as_ptr(),
            to_wide(url).as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
        );
    }
}

pub fn show_nvram_cleared_dialog(owner: *mut std::ffi::c_void) {
    let text = to_wide(
        "Console memory (NVRAM) has been cleared successfully.\n\n\
        All saved games and preferences have been reset to factory defaults.\n\
        The console has been restarted."
    );
    let title = to_wide("OpenCDi - Memory Cleared");
    unsafe {
        MessageBoxW(owner, text.as_ptr(), title.as_ptr(), MB_OK | MB_ICONINFORMATION);
    }
}
