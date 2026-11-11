//! Native DolphinBar Mode 4 input. The transport runs away from the emulation thread.
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Default, Debug)]
pub struct Sample {
    pub buttons: u16,
    pub aim: Option<(f64, f64)>,
}

pub const B: u16 = 0x0004;
pub const A: u16 = 0x0008;
pub const HOME: u16 = 0x0080;
pub const LEFT: u16 = 0x0100;
pub const RIGHT: u16 = 0x0200;
pub const DOWN: u16 = 0x0400;
pub const UP: u16 = 0x0800;
pub const MINUS: u16 = 0x0010;
pub const PLUS: u16 = 0x1000;
pub const ONE: u16 = 0x0002;
pub const TWO: u16 = 0x0001;

/// Chaos Control's shooting reticle is centred seven low-resolution pixels
/// inside its hardware cursor. Its menus use a different cursor and hotspot.
pub fn correct_game_aim(title: &str, cursor_control: u32, x: i32, y: i32) -> (i32, i32) {
    let title = title.to_ascii_lowercase();
    let chaos = title.strip_prefix("chaos control").is_some_and(|rest|
        rest.is_empty() || rest.starts_with(" ("));
    let shooting_cursor = cursor_control & opencdi::mcd212::CURCNT_EN != 0
        && matches!(cursor_control & opencdi::mcd212::CURCNT_COLOR, 0x0a | 0x0c);
    if chaos && shooting_cursor {
        ((x - 14).clamp(0,767), (y - 14).clamp(0,559))
    } else { (x,y) }
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub enum ControlMode {
    Pointer,
    #[default]
    Sideways,
}

#[derive(Default)]
pub struct AutoControl {
    pub mode: ControlMode,
    last_aim: Option<Instant>,
    reacquired: Option<Instant>,
    blocked: u16,
}

impl AutoControl {
    pub fn suppress_held(&mut self, buttons: u16) { self.blocked |= buttons; }

    pub fn update(&mut self, sample: Option<Sample>, now: Instant) -> u16 {
        let Some(sample) = sample else {
            *self = Self::default();
            return 0;
        };
        let previous = self.mode;
        if sample.aim.is_some() {
            self.last_aim = Some(now);
            let since = *self.reacquired.get_or_insert(now);
            if now.saturating_duration_since(since) >= Duration::from_millis(100) {
                self.mode = ControlMode::Pointer;
            }
        } else {
            self.reacquired = None;
            if self.last_aim.is_none_or(|time| now.saturating_duration_since(time) >= Duration::from_millis(500)) {
                self.mode = ControlMode::Sideways;
            }
        }
        if self.mode != previous { self.blocked |= sample.buttons; }
        self.blocked &= sample.buttons;
        sample.buttons & !self.blocked
    }

    pub fn directions(&self, buttons: u16) -> (i32, i32) {
        let (left,right,up,down) = match self.mode {
            ControlMode::Pointer => (LEFT,RIGHT,UP,DOWN),
            ControlMode::Sideways => (UP,DOWN,RIGHT,LEFT),
        };
        (i32::from(buttons & right != 0) - i32::from(buttons & left != 0),
         i32::from(buttons & down != 0) - i32::from(buttons & up != 0))
    }
}

pub fn decode(report: &[u8]) -> Option<Sample> {
    let id = *report.first()?;
    if !(0x30..=0x37).contains(&id) || report.len() < 3 { return None; }
    if id == 0x33 && report.len() < 18 { return None; }
    // Accelerometer low bits share the button bytes and must be masked out.
    let buttons = u16::from_be_bytes([report[1] & 0x1f, report[2] & 0x9f]);
    if id != 0x33 { return Some(Sample { buttons, aim: None }); }
    let points: Vec<_> = report[6..18].chunks_exact(3).filter_map(|p| {
        let x = u16::from(p[0]) | (u16::from(p[2] & 0x30) << 4);
        let y = u16::from(p[1]) | (u16::from(p[2] & 0xc0) << 2);
        (x < 1023 && y < 1023).then_some((f64::from(x), f64::from(y)))
    }).collect();
    // Both ends of the sensor bar must be visible. A lone point cannot reliably
    // identify the centre of the bar and would move the aim when tracking is lost.
    let aim = (points.len() == 2).then(|| {
        ((points[0].0 + points[1].0) / 2.0, (points[0].1 + points[1].1) / 2.0)
    });
    Some(Sample { buttons, aim })
}

#[derive(Clone, Copy, Debug)]
pub struct Calibration { pub first: (f64, f64), pub second: (f64, f64) }

impl Default for Calibration {
    fn default() -> Self { Self { first: (870.0, 115.0), second: (154.0, 653.0) } }
}

impl Calibration {
    pub fn valid(self) -> bool {
        [self.first.0, self.first.1, self.second.0, self.second.1].iter().all(|v| v.is_finite() && (0.0..=1023.0).contains(v))
            && (self.second.0 - self.first.0).abs() > 40.0
            && (self.second.1 - self.first.1).abs() > 40.0
    }
    pub fn map(self, aim: (f64, f64)) -> Option<(f64, f64)> {
        let x = 0.15 + 0.7 * (aim.0 - self.first.0) / (self.second.0 - self.first.0);
        let y = 0.15 + 0.7 * (aim.1 - self.first.1) / (self.second.1 - self.first.1);
        (self.valid() && (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y)).then_some((x, y))
    }
    pub fn load() -> Self {
        let path = opencdi::frontend::app_data_file("peacekeeper-calibration.ini");
        let values: Vec<f64> = std::fs::read_to_string(path).unwrap_or_default()
            .split_whitespace().filter_map(|s| s.parse().ok()).collect();
        if values.len() == 4 {
            let c = Self { first: (values[0], values[1]), second: (values[2], values[3]) };
            if c.valid() { return c; }
        }
        Self::default()
    }
    pub fn save(self) -> std::io::Result<()> {
        std::fs::write(opencdi::frontend::app_data_file("peacekeeper-calibration.ini"),
            format!("{} {} {} {}\n", self.first.0, self.first.1, self.second.0, self.second.1))
    }
}

pub struct Wiimote {
    state: Arc<Mutex<Option<(Instant, Sample)>>>,
    pointer_target: Arc<Mutex<Option<(isize, Calibration, Instant)>>>,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Wiimote {
    pub fn new() -> Self {
        let state = Arc::new(Mutex::new(None));
        let pointer_target = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        #[cfg(windows)]
        let worker = {
            let state = state.clone();
            let stop = stop.clone();
            let pointer_target = pointer_target.clone();
            Some(std::thread::spawn(move || native::run(state, stop, pointer_target)))
        };
        #[cfg(not(windows))]
        let worker = None;
        Self { state, pointer_target, stop, worker }
    }
    pub fn set_pointer_target(&self, window: Option<isize>, calibration: Calibration) {
        if let Ok(mut target) = self.pointer_target.lock() {
            *target = window.map(|window| (window, calibration, Instant::now()));
        }
    }
    pub fn sample(&self) -> Option<Sample> {
        self.state.lock().ok()?.as_ref().filter(|(time, _)| time.elapsed() < Duration::from_millis(250)).map(|(_, sample)| *sample)
    }
}

impl Drop for Wiimote {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() { let _ = worker.join(); }
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use std::{ffi::c_void, ptr};
    type Handle = *mut c_void;
    #[repr(C)] struct Point { x: i32, y: i32 }
    #[repr(C)] struct Rect { left: i32, top: i32, right: i32, bottom: i32 }
    #[link(name="user32")]
    unsafe extern "system" {
        fn GetForegroundWindow() -> Handle;
        fn GetClientRect(window: Handle, rect: *mut Rect) -> i32;
        fn ClientToScreen(window: Handle, point: *mut Point) -> i32;
        fn SetCursorPos(x: i32, y: i32) -> i32;
    }
    fn move_pointer(sample: Sample, target: &Mutex<Option<(isize, Calibration, Instant)>>) {
        let target = target.lock().ok().and_then(|target| *target);
        let Some((window, calibration, updated)) = target else { return };
        if updated.elapsed() > Duration::from_millis(250) { return; }
        let Some((x,y)) = sample.aim.and_then(|aim| calibration.map(aim)) else { return };
        unsafe {
            let window = window as Handle;
            if window.is_null() || GetForegroundWindow() != window { return; }
            let mut rect = Rect { left:0, top:0, right:0, bottom:0 };
            if GetClientRect(window, &mut rect) == 0 || rect.right <= 0 || rect.bottom <= 0 { return; }
            let mut point = Point {
                x:(x * (rect.right-1) as f64).round() as i32,
                y:(y * (rect.bottom-1) as f64).round() as i32,
            };
            if ClientToScreen(window, &mut point) != 0 && GetForegroundWindow() == window {
                SetCursorPos(point.x, point.y);
            }
        }
    }
    #[repr(C)] struct Guid { a: u32, b: u16, c: u16, d: [u8; 8] }
    #[repr(C)] struct Overlapped { internal: usize, high: usize, offset: u64, event: Handle }
    #[link(name="cfgmgr32")]
    unsafe extern "system" {
        fn CM_Get_Device_Interface_List_SizeW(size: *mut u32, guid: *const Guid, id: *const u16, flags: u32) -> u32;
        fn CM_Get_Device_Interface_ListW(guid: *const Guid, id: *const u16, list: *mut u16, size: u32, flags: u32) -> u32;
    }
    #[link(name="kernel32")]
    unsafe extern "system" {
        fn CreateFileW(path: *const u16, access: u32, share: u32, security: *mut c_void, disposition: u32, flags: u32, template: Handle) -> Handle;
        fn CreateEventW(security: *const c_void, manual: i32, initial: i32, name: *const u16) -> Handle;
        fn ReadFile(file: Handle, buffer: *mut u8, size: u32, read: *mut u32, overlapped: *mut Overlapped) -> i32;
        fn WaitForSingleObject(handle: Handle, ms: u32) -> u32;
        fn GetOverlappedResult(file: Handle, overlapped: *mut Overlapped, read: *mut u32, wait: i32) -> i32;
        fn CancelIoEx(file: Handle, overlapped: *mut Overlapped) -> i32;
        fn CloseHandle(handle: Handle) -> i32;
        fn GetLastError() -> u32;
    }
    #[link(name="hid")]
    unsafe extern "system" { fn HidD_SetOutputReport(file: Handle, data: *const u8, length: u32) -> u8; }
    struct Owned(Handle);
    impl Drop for Owned { fn drop(&mut self) { unsafe { CloseHandle(self.0); } } }
    struct Device { file: Owned, event: Owned }
    impl Device {
        fn output(&self, id: u8, payload: &[u8]) -> bool {
            let mut report = [0u8; 22];
            report[0] = id;
            report[1..1 + payload.len()].copy_from_slice(payload);
            unsafe { HidD_SetOutputReport(self.file.0, report.as_ptr(), 22) != 0 }
        }
        fn register(&self, address: u32, data: &[u8]) -> bool {
            let mut payload = [0u8; 21];
            payload[..4].copy_from_slice(&(address | 0x04000000).to_be_bytes());
            payload[4] = data.len() as u8;
            payload[5..5 + data.len()].copy_from_slice(data);
            let ok = self.output(0x16, &payload);
            std::thread::sleep(Duration::from_millis(30));
            ok
        }
        fn initialize(&self) -> bool {
            self.output(0x13, &[4]) && self.output(0x1a, &[4])
                && self.register(0xb00030, &[8])
                && self.register(0xb00000, &[2,0,0,0x71,1,0,0x64,0,0xfe])
                && self.register(0xb0001a, &[0xfd,5])
                && self.register(0xb00033, &[3])
                && self.register(0xb00030, &[8])
                && self.output(0x12, &[4,0x33])
        }
        fn read(&self) -> std::io::Result<Option<Sample>> {
            let mut buffer = [0u8;22];
            let mut count = 0;
            let mut io = Overlapped { internal: 0, high: 0, offset: 0, event: self.event.0 };
            // The buffer and OVERLAPPED remain alive until the request finishes,
            // including the cancellation path when the bounded wait expires.
            unsafe {
                let immediate = ReadFile(self.file.0, buffer.as_mut_ptr(), 22, &mut count, &mut io);
                if immediate == 0 {
                    if GetLastError() != 997 { return Err(std::io::Error::last_os_error()); }
                    if WaitForSingleObject(self.event.0, 100) != 0 {
                        CancelIoEx(self.file.0, &mut io);
                        GetOverlappedResult(self.file.0, &mut io, &mut count, 1);
                        return Ok(None);
                    }
                    if GetOverlappedResult(self.file.0, &mut io, &mut count, 0) == 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                }
            }
            Ok(decode(&buffer[..count as usize]))
        }
    }
    impl Drop for Device {
        fn drop(&mut self) {
            self.output(0x12, &[0,0x30]);
            self.output(0x13, &[0]);
            self.output(0x1a, &[0]);
        }
    }
    fn discover() -> Option<Device> {
        unsafe {
            let guid = Guid { a: 0x4d1e55b2, b: 0xf16f, c: 0x11cf, d: [0x88,0xcb,0,0x11,0x11,0,0,0x30] };
            let mut count = 0;
            if CM_Get_Device_Interface_List_SizeW(&mut count, &guid, ptr::null(), 0) != 0 { return None; }
            let mut list = vec![0u16; count as usize];
            if CM_Get_Device_Interface_ListW(&guid, ptr::null(), list.as_mut_ptr(), count, 0) != 0 { return None; }
            for path in list.split(|c| *c == 0).filter(|s| !s.is_empty()) {
                let name = String::from_utf16_lossy(path).to_lowercase();
                if !name.contains("vid_057e&pid_0306") && !name.contains("vid_057e&pid_0330") { continue; }
                let wide: Vec<u16> = path.iter().copied().chain(Some(0)).collect();
                let raw = CreateFileW(wide.as_ptr(), 0xc0000000, 3, ptr::null_mut(), 3, 0x40000000, ptr::null_mut());
                if raw == (-1isize as Handle) { continue; }
                let file = Owned(raw);
                let event = Owned(CreateEventW(ptr::null(), 1, 0, ptr::null()));
                if event.0.is_null() { continue; }
                let device = Device { file, event };
                if device.output(0x15, &[0]) && device.initialize() { return Some(device); }
            }
            None
        }
    }
    pub fn run(state: Arc<Mutex<Option<(Instant, Sample)>>>, stop: Arc<AtomicBool>, pointer_target: Arc<Mutex<Option<(isize, Calibration, Instant)>>>) {
        while !stop.load(Ordering::Relaxed) {
            if let Some(device) = discover() {
                let mut last_report = Instant::now();
                while !stop.load(Ordering::Relaxed) && last_report.elapsed() < Duration::from_secs(2) {
                    match device.read() {
                        Ok(Some(sample)) => {
                            last_report = Instant::now();
                            move_pointer(sample, &pointer_target);
                            if let Ok(mut state) = state.lock() { *state = Some((last_report, sample)); }
                        }
                        Ok(None) => {}
                        Err(_) => break,
                    }
                }
            }
            if let Ok(mut state) = state.lock() { *state = None; }
            for _ in 0..20 {
                if stop.load(Ordering::Relaxed) { return; }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chaos_correction_is_limited_to_the_shooting_reticle() {
        let shooting = 0x00b8000a;
        assert_eq!(correct_game_aim("Chaos Control (USA)",shooting,500,360),(486,346));
        assert_eq!(correct_game_aim("Chaos Control (Europe)",0x00b8000c,200,160),(186,146));
        assert_eq!(correct_game_aim("Chaos Control",0x00b8000f,500,360),(500,360));
        assert_eq!(correct_game_aim("Chaos Control",0x0038000a,500,360),(500,360));
        assert_eq!(correct_game_aim("Mad Dog McCree",shooting,500,360),(500,360));
        assert_eq!(correct_game_aim("Chaos Control",shooting,5,5),(0,0));
    }
    #[test]
    #[ignore = "Requires a connected Mode 4 DolphinBar and awake Wiimote"]
    fn live_mode4_transport() {
        let remote = Wiimote::new();
        let deadline = Instant::now() + Duration::from_secs(6);
        while Instant::now() < deadline {
            if let Some(sample) = remote.sample() {
                println!("Live Mode 4 sample: {sample:?}");
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("No live Mode 4 report received");
    }
    #[test]
    fn live_reports_mask_acceleration_and_require_two_ir_points() {
        let mut report = [0xff;22]; report[0] = 0x33; report[1] = 0x60; report[2] = 0x64;
        report[6..12].copy_from_slice(&[100,200,0, 200,200,0]);
        let sample = decode(&report).unwrap();
        assert_eq!(sample.buttons, B);
        assert_eq!(sample.aim, Some((150.0,200.0)));
        report[9..12].fill(0xff);
        assert!(decode(&report).unwrap().aim.is_none());
        assert!(decode(&report[..10]).is_none());
    }
    #[test]
    fn calibration_maps_targets_and_rejects_bad_or_offscreen_aim() {
        let c = Calibration { first: (800.0,200.0), second: (200.0,600.0) };
        assert_eq!(c.map((500.0,400.0)), Some((0.5,0.5)));
        assert!(c.map((1023.0,400.0)).is_none());
        assert!(!Calibration { first: (200.0,200.0), second: (200.0,300.0) }.valid());
    }

    #[test]
    fn library_buttons_work_without_infrared_tracking() {
        let sample = decode(&[0x30, 0x09, 0x0c]).unwrap();
        assert_eq!(sample.buttons, UP | LEFT | A | B);
        assert!(sample.aim.is_none());
        let mut report = [0xff; 22];
        report[..3].copy_from_slice(&[0x33, 0x06, 0x08]);
        let sample = decode(&report).unwrap();
        assert_eq!(sample.buttons, DOWN | RIGHT | A);
        assert!(sample.aim.is_none());
    }

    #[test]
    fn automatic_controls_debounce_tracking_and_release_held_buttons_on_switch() {
        let start = Instant::now();
        let mut controls = AutoControl::default();
        let pointing = Sample { buttons: A, aim: Some((500.0,400.0)) };
        controls.update(Some(pointing),start);
        assert_eq!(controls.mode,ControlMode::Sideways);
        assert_eq!(controls.update(Some(pointing),start+Duration::from_millis(110)),0);
        assert_eq!(controls.mode,ControlMode::Pointer);
        controls.update(Some(Sample { buttons: 0, ..pointing }),start+Duration::from_millis(120));
        assert_eq!(controls.update(Some(pointing),start+Duration::from_millis(130)),A);
        let lost = Sample { buttons: ONE, aim: None };
        controls.update(Some(lost),start+Duration::from_millis(400));
        assert_eq!(controls.mode,ControlMode::Pointer);
        assert_eq!(controls.update(Some(lost),start+Duration::from_millis(650)),0);
        assert_eq!(controls.mode,ControlMode::Sideways);
        controls.update(None,start+Duration::from_millis(700));
        assert_eq!(controls.update(Some(lost),start+Duration::from_millis(710)),ONE);
    }

    #[test]
    fn sideways_dpad_matches_buttons_on_the_right_grip() {
        let mut controls = AutoControl::default();
        assert_eq!(controls.directions(UP),(-1,0));
        assert_eq!(controls.directions(DOWN),(1,0));
        assert_eq!(controls.directions(RIGHT),(0,-1));
        assert_eq!(controls.directions(LEFT),(0,1));
        controls.mode = ControlMode::Pointer;
        assert_eq!(controls.directions(UP),(0,-1));
    }
}
