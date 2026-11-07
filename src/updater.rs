use serde_json::Value;
use std::ffi::c_void;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::sync::mpsc::{self, Receiver};

const REPO: &str = "sionisaac/opencdi";
const ASSET: &str = "OpenCDi-Windows-x64.zip";
const MAX_BYTES: usize = 64 * 1024 * 1024;
const APPLY: &str = "--apply-opencdi-update";
const SKIP_CHECK: &str = "OPENCDI_SKIP_UPDATE_CHECK";

struct Release {
    tag: String,
    url: String,
    digest: String,
    size: usize,
}

#[derive(Default)]
pub struct Startup {
    check: Option<Receiver<Option<Release>>>,
    download: Option<Receiver<Result<PathBuf, String>>>,
}

impl Startup {
    pub fn start(&mut self) {
        if std::env::var_os(SKIP_CHECK).is_some()
            || !std::env::current_exe().is_ok_and(|exe| {
                exe.file_name()
                    .is_some_and(|name| name.eq_ignore_ascii_case("opencdi.exe"))
            })
        {
            return;
        }
        self.check = Some(background(|| {
            let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
            let data = crate::launchbox::http_get(&url, 1024 * 1024)?;
            parse_release(&data, env!("CARGO_PKG_VERSION"))
        }));
    }

    pub fn poll(&mut self, owner: *mut c_void) -> bool {
        if let Some(Some(release)) = completed(&mut self.check) {
            let message = format!(
                "OpenCDi {} is available.\n\nDownload the update and restart OpenCDi?",
                release.tag
            );
            if dialog(owner, &message, 0x124) == 6 {
                self.download = Some(background(move || prepare(&release)));
            }
        }
        if let Some(result) = completed(&mut self.download) {
            let result = result.and_then(|helper| {
                Command::new(helper)
                    .arg(APPLY)
                    .arg(process::id().to_string())
                    .args(std::env::args_os().skip(1))
                    .spawn()
                    .map_err(|error| error.to_string())
            });
            match result {
                Ok(_) => return true,
                Err(error) => {
                    dialog(
                        owner,
                        &format!("OpenCDi could not prepare the update.\n\n{error}"),
                        0x30,
                    );
                }
            }
        }
        false
    }
}

fn background<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Receiver<T> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(work());
    });
    receiver
}

fn completed<T>(task: &mut Option<Receiver<T>>) -> Option<T> {
    let result = task.as_ref()?.try_recv();
    if !matches!(result, Err(mpsc::TryRecvError::Empty)) {
        *task = None;
    }
    result.ok()
}

fn version(tag: &str) -> Option<[u32; 3]> {
    let mut parts = tag.strip_prefix('v').unwrap_or(tag).split('.');
    let mut numbers = [0; 3];
    for number in &mut numbers {
        let part = parts.next()?;
        if !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        *number = part.parse().ok()?;
    }
    parts.next().is_none().then_some(numbers)
}

fn parse_release(data: &[u8], current: &str) -> Option<Release> {
    let release: Value = serde_json::from_slice(data).ok()?;
    let tag = release["tag_name"].as_str()?;
    if release["draft"].as_bool()?
        || release["prerelease"].as_bool()?
        || version(tag)? <= version(current)?
    {
        return None;
    }
    let asset = release["assets"]
        .as_array()?
        .iter()
        .find(|asset| asset["name"] == ASSET)?;
    let url = format!("https://github.com/{REPO}/releases/download/{tag}/{ASSET}");
    let digest = asset["digest"].as_str()?.strip_prefix("sha256:")?;
    let size = usize::try_from(asset["size"].as_u64()?).ok()?;
    if asset["state"] != "uploaded"
        || asset["browser_download_url"].as_str()? != url
        || digest.len() != 64
        || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
        || !(1..=MAX_BYTES).contains(&size)
    {
        return None;
    }
    Some(Release {
        tag: tag.into(),
        url,
        digest: digest.to_ascii_lowercase(),
        size,
    })
}

fn prepare(release: &Release) -> Result<PathBuf, String> {
    let zip = crate::launchbox::http_get(&release.url, release.size)
        .ok_or("The release ZIP could not be downloaded.")?;
    if zip.len() != release.size || sha256(&zip).as_deref() != Some(&release.digest) {
        return Err("The download did not match GitHub's SHA-256 checksum.".into());
    }
    let exe = crate::launchbox::read_unique_zip_member(
        &zip,
        &["opencdi.exe", "OpenCDi/opencdi.exe"],
        MAX_BYTES,
    )
    .filter(|bytes| is_x64_executable(bytes))
    .ok_or("The release ZIP does not contain a supported Windows executable.")?;
    stage(&exe).map_err(|error| error.to_string())
}

fn is_x64_executable(bytes: &[u8]) -> bool {
    if !bytes.starts_with(b"MZ") {
        return false;
    }
    let Some(offset) = bytes.get(0x3c..0x40).and_then(|data| data.try_into().ok()) else {
        return false;
    };
    bytes
        .get(u32::from_le_bytes(offset) as usize..)
        .is_some_and(|header| header.starts_with(b"PE\0\0\x64\x86"))
}

fn stage(executable: &[u8]) -> io::Result<PathBuf> {
    let live = std::env::current_exe()?;
    let directory = live.with_file_name(format!(".opencdi-update-{}", process::id()));
    fs::create_dir(&directory)?;
    let helper = directory.join("apply.exe");
    let pending = directory.join("new.exe");
    let backup = directory.join("previous.exe");
    let result = fs::write(&pending, executable)
        .and_then(|_| fs::copy(&live, &helper))
        .and_then(|_| fs::copy(&live, &backup));
    if let Err(error) = result {
        for path in [pending, helper, backup] {
            let _ = fs::remove_file(path);
        }
        let _ = fs::remove_dir(directory);
        return Err(error);
    }
    Ok(helper)
}

pub fn run_helper() -> bool {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new(APPLY)) {
        return false;
    }
    let result = (|| -> io::Result<()> {
        let pid = args
            .next()
            .and_then(|arg| arg.to_str()?.parse::<u32>().ok())
            .ok_or_else(|| io::Error::other("Missing update process ID"))?;
        let helper = std::env::current_exe()?;
        let directory = helper
            .parent()
            .filter(|dir| {
                dir.file_name().is_some_and(|name| {
                    name == std::ffi::OsStr::new(&format!(".opencdi-update-{pid}"))
                })
            })
            .filter(|_| helper.ends_with("apply.exe"))
            .ok_or_else(|| io::Error::other("Invalid update staging folder"))?;
        wait_for_exit(pid)?;
        let live = directory.with_file_name("opencdi.exe");
        let mut restart = Command::new(&live);
        restart.args(args).env(SKIP_CHECK, "1");
        install(directory, &live, || restart.spawn().map(|_| ()))
    })();
    if let Err(error) = result {
        dialog(
            std::ptr::null_mut(),
            &format!("OpenCDi could not complete the update.\n\n{error}"),
            0x30,
        );
    }
    true
}

fn install(
    directory: &Path,
    live: &Path,
    mut restart: impl FnMut() -> io::Result<()>,
) -> io::Result<()> {
    if let Err(error) = fs::rename(directory.join("new.exe"), live) {
        let _ = restart();
        return Err(error);
    }
    if let Err(error) = restart() {
        fs::rename(directory.join("previous.exe"), live)?;
        let _ = restart();
        return Err(error);
    }
    Ok(())
}

fn wait_for_exit(pid: u32) -> io::Result<()> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn WaitForSingleObject(handle: *mut c_void, milliseconds: u32) -> u32;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }
    let handle = unsafe { OpenProcess(0x0010_0000, 0, pid) };
    if handle.is_null() {
        let error = io::Error::last_os_error();
        return if error.raw_os_error() == Some(87) {
            Ok(())
        } else {
            Err(error)
        };
    }
    let result = unsafe { WaitForSingleObject(handle, 60_000) };
    unsafe {
        CloseHandle(handle);
    }
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::other("Timed out waiting for OpenCDi to close"))
    }
}

fn sha256(bytes: &[u8]) -> Option<String> {
    #[link(name = "bcrypt")]
    unsafe extern "system" {
        fn BCryptOpenAlgorithmProvider(
            handle: *mut *mut c_void,
            algorithm: *const u16,
            implementation: *const u16,
            flags: u32,
        ) -> i32;
        fn BCryptHash(
            handle: *mut c_void,
            secret: *const u8,
            secret_len: u32,
            input: *const u8,
            input_len: u32,
            output: *mut u8,
            output_len: u32,
        ) -> i32;
        fn BCryptCloseAlgorithmProvider(handle: *mut c_void, flags: u32) -> i32;
    }
    let algorithm: Vec<u16> = "SHA256\0".encode_utf16().collect();
    let len = u32::try_from(bytes.len()).ok()?;
    let mut handle = std::ptr::null_mut();
    let mut output = [0u8; 32];
    unsafe {
        if BCryptOpenAlgorithmProvider(&mut handle, algorithm.as_ptr(), std::ptr::null(), 0) != 0 {
            return None;
        }
        let status = BCryptHash(
            handle,
            std::ptr::null(),
            0,
            bytes.as_ptr(),
            len,
            output.as_mut_ptr(),
            32,
        );
        BCryptCloseAlgorithmProvider(handle, 0);
        (status == 0).then(|| output.iter().map(|byte| format!("{byte:02x}")).collect())
    }
}

fn dialog(owner: *mut c_void, message: &str, flags: u32) -> i32 {
    let title: Vec<u16> = "OpenCDi Update\0".encode_utf16().collect();
    let message: Vec<u16> = message.encode_utf16().chain([0]).collect();
    unsafe { crate::MessageBoxW(owner, message.as_ptr(), title.as_ptr(), flags) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_newer_stable_releases_with_a_verified_asset() {
        let mut release = serde_json::json!({
            "tag_name": "v0.2.10", "draft": false, "prerelease": false,
            "assets": [{
                "name": ASSET, "state": "uploaded", "size": 100,
                "browser_download_url": format!("https://github.com/{REPO}/releases/download/v0.2.10/{ASSET}"),
                "digest": format!("sha256:{}", "a".repeat(64))
            }]
        });
        let parse =
            |release: &Value, current| parse_release(release.to_string().as_bytes(), current);
        assert!(parse(&release, "0.2.9").is_some());
        assert!(parse(&release, "0.2.10").is_none());
        release["prerelease"] = true.into();
        assert!(parse(&release, "0.2.9").is_none());
        release["prerelease"] = false.into();
        release["assets"][0]["digest"] = Value::Null;
        assert!(parse(&release, "0.2.9").is_none());
    }

    #[test]
    fn restores_previous_executable_if_windows_rejects_the_update() {
        let dir = std::env::temp_dir().join(format!("opencdi-update-test-{}", process::id()));
        fs::create_dir_all(&dir).unwrap();
        let live = dir.join("opencdi.exe");
        fs::write(&live, b"original").unwrap();
        fs::copy(&live, dir.join("previous.exe")).unwrap();
        fs::write(dir.join("new.exe"), b"invalid replacement").unwrap();

        let mut attempts = 0;
        let result = install(&dir, &live, || {
            attempts += 1;
            if attempts == 1 {
                Err(io::Error::other("Windows refused the update"))
            } else {
                Ok(())
            }
        });
        assert!(result.is_err());
        assert_eq!(attempts, 2);
        assert_eq!(fs::read(&live).unwrap(), b"original");

        fs::remove_file(live).unwrap();
        fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn computes_github_checksum() {
        assert_eq!(
            sha256(b"abc").as_deref(),
            Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        );
    }
}
