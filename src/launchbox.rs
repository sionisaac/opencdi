//! Optional background artwork lookup using LaunchBox's published metadata archive.
//! Downloads are cached under the active OpenCDi data folder and never replace local artwork.

use flate2::read::DeflateDecoder;
use quick_xml::events::Event;
use quick_xml::Reader;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, BufRead, Cursor, Read};
use std::path::Path;
use std::time::{Duration, SystemTime};

const METADATA_URL: &str = "https://gamesdb.launchbox-app.com/Metadata.zip";
const IMAGE_BASE_URL: &str = "https://images.launchbox-app.com/";
const METADATA_MAX_BYTES: usize = 300 * 1024 * 1024;
const IMAGE_MAX_BYTES: usize = 24 * 1024 * 1024;

#[derive(Clone)]
pub struct FetchTarget {
    pub title: String,
    pub cover_missing: bool,
    pub disc_missing: bool,
    pub download_all: bool,
}

#[derive(Default)]
struct GameRecord { id: String, title: String, platform: String }

#[derive(Default)]
struct ImageRecord { id: String, file: String, kind: String, region: String }

#[derive(Default)]
struct ParsedRecords { games: Vec<GameRecord>, images: Vec<ImageRecord> }

pub fn fetch_missing_artwork(targets: &[FetchTarget], artwork_root: &Path, cache_dir: &Path) -> usize {
    let pending: Vec<_> = targets.iter().filter(|t| t.cover_missing || t.disc_missing || t.download_all).cloned().collect();
    if pending.is_empty() { return 0; }
    let covers_dir = artwork_root.join("Covers");
    let discs_dir = artwork_root.join("CD");
    let bezels_dir = artwork_root.join("Bezels");
    let media_dir = artwork_root.join("LaunchBox");
    if fs::create_dir_all(&covers_dir).is_err() || fs::create_dir_all(&discs_dir).is_err() || fs::create_dir_all(&bezels_dir).is_err() || fs::create_dir_all(&media_dir).is_err() || fs::create_dir_all(cache_dir).is_err() { return 0; }
    let archive = cache_dir.join("LaunchBox-Metadata.zip");
    if !metadata_is_fresh(&archive) {
        let Some(bytes) = http_get(METADATA_URL, METADATA_MAX_BYTES) else { log::warn!("LaunchBox artwork lookup could not reach its metadata archive"); return 0; };
        let tmp = archive.with_extension("zip.download");
        if fs::write(&tmp, bytes).is_err() || fs::rename(&tmp, &archive).is_err() {
            let _ = fs::remove_file(&tmp);
            log::warn!("LaunchBox metadata archive could not be cached");
            return 0;
        }
    }
    let Ok(zip) = fs::read(&archive) else { return 0 };
    let Some(entries) = zip_entries(&zip) else { log::warn!("LaunchBox metadata archive could not be read"); return 0; };
    let wanted: HashSet<String> = HashSet::new();
    let mut records = ParsedRecords::default();
    for entry in &entries {
        if !entry.name.to_ascii_lowercase().ends_with(".xml") || entry.uncompressed_size > 512 * 1024 * 1024 { continue; }
        let Some(stream) = entry.open(&zip) else { continue; };
        parse_xml(&mut io::BufReader::new(stream), &wanted, &HashSet::new(), &mut records);
    }
    records.games.retain(|g| g.platform.to_ascii_lowercase().contains("philips cd-i"));
    let mut matched: HashMap<String, Vec<String>> = HashMap::new();
    for target in &pending {
        let key = normalize_title(&target.title);
        let mut exact: Vec<String> = records.games.iter().filter(|g| normalize_title(&g.title) == key).map(|g| g.id.clone()).filter(|id| !id.is_empty()).collect();
        exact.sort(); exact.dedup();
        if exact.is_empty() {
            let mut scored: Vec<(f32, &GameRecord)> = records.games.iter().filter(|g| !g.id.is_empty()).map(|g| (title_similarity(&key, &normalize_title(&g.title)), g)).collect();
            scored.sort_by(|a, b| b.0.total_cmp(&a.0));
            if let Some((score, game)) = scored.first().copied() {
                let next = scored.get(1).map(|v| v.0).unwrap_or(0.0);
                if score >= 0.90 && score - next >= 0.04 { exact.push(game.id.clone()); }
            }
        }
        if !exact.is_empty() { matched.insert(key, exact); }
    }
    if matched.is_empty() { log::info!("LaunchBox has no confident Philips CD-i title matches for missing artwork"); return 0; }
    let ids: HashSet<String> = matched.values().flatten().cloned().collect();
    records.images.clear();
    for entry in &entries {
        if !entry.name.to_ascii_lowercase().ends_with(".xml") || entry.uncompressed_size > 512 * 1024 * 1024 { continue; }
        let Some(stream) = entry.open(&zip) else { continue; };
        parse_xml(&mut io::BufReader::new(stream), &HashSet::new(), &ids, &mut records);
    }
    let mut saved = 0;
    for target in pending {
        let key = normalize_title(&target.title);
        let Some(game_ids) = matched.get(&key) else { continue };
        for game_id in game_ids {
            let candidates: Vec<&ImageRecord> = records.images.iter().filter(|i| i.id == *game_id).collect();
            if target.cover_missing || target.download_all {
                if let Some(image) = choose_image(&candidates, true, &target.title) {
                    if download_image(image, &covers_dir.join(format!("{}.{}", safe_name(&target.title), extension(&image.file)))) { saved += 1; }
                }
            }
            if target.disc_missing || target.download_all {
                if let Some(image) = choose_image(&candidates, false, &target.title) {
                    if download_image(image, &discs_dir.join(format!("{}-disc.{}", safe_name(&target.title), extension(&image.file)))) { saved += 1; }
                }
            }
            let mut stored = HashSet::new();
            for image in candidates.iter().copied() {
                if !supported_image(&image.file) || image.kind.is_empty() { continue; }
                let filename = image.file.rsplit('/').next().unwrap_or(&image.file);
                let filename_stem = Path::new(filename).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "image".to_string());
                let prefix = safe_name(&target.title);
                let unique = format!("{}--{}", prefix, safe_name(&filename_stem));
                if !stored.insert(unique.clone()) { continue; }
                let kind = image.kind.to_ascii_lowercase();
                let target_dir = if kind.contains("disc") { media_dir.join("Disc") }
                    else if kind.contains("fanart - background") || kind == "background" { bezels_dir.clone() }
                    else { media_dir.join(safe_name(&image.kind)) };
                let destination = target_dir.join(format!("{}.{}", unique, extension(&image.file)));
                if download_image(image, &destination) { saved += 1; }
            }
        }
    }
    log::info!("LaunchBox artwork lookup saved {} image(s)", saved);
    saved
}

fn metadata_is_fresh(path: &Path) -> bool {
    let Ok(meta) = fs::metadata(path) else { return false };
    let Ok(modified) = meta.modified() else { return false };
    SystemTime::now().duration_since(modified).unwrap_or(Duration::MAX) < Duration::from_secs(24 * 60 * 60)
}

fn normalize_title(s: &str) -> String {
    let mut normalized = String::new();
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '(' || ch == '[' {
            let closing = if ch == '(' { ')' } else { ']' };
            let mut tag = String::new();
            while let Some(c) = chars.next() { if c == closing { break; } tag.push(c); }
            let tag_l = tag.to_ascii_lowercase();
            let is_dump_tag = ["usa", "europe", "european", "japan", "japanese", "world", "disc", "disk", "cd", "track", "rev", "revision", "version", "beta", "proto", "demo", "pal", "ntsc", "en,", "fr,", "de,"]
                .iter().any(|needle| tag_l.contains(needle));
            if !is_dump_tag { normalized.extend(tag.chars().filter(|c| c.is_alphanumeric()).map(|c| c.to_ascii_lowercase())); }
        } else if ch.is_alphanumeric() { normalized.push(ch.to_ascii_lowercase()); }
    }
    normalized
}

fn title_similarity(a: &str, b: &str) -> f32 {
    if a.is_empty() || b.is_empty() { return 0.0; }
    if a == b { return 1.0; }
    if a.len().min(b.len()) < 6 { return 0.0; }
    let left: Vec<char> = a.chars().collect();
    let right: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    let mut current = vec![0usize; right.len() + 1];
    for (i, ca) in left.iter().enumerate() {
        current[0] = i + 1;
        for (j, cb) in right.iter().enumerate() {
            current[j + 1] = (previous[j + 1] + 1).min(current[j] + 1).min(previous[j] + usize::from(ca != cb));
        }
        std::mem::swap(&mut previous, &mut current);
    }
    let longest = left.len().max(right.len()) as f32;
    1.0 - previous[right.len()] as f32 / longest
}

fn supported_image(file: &str) -> bool {
    file.rsplit_once('.').map(|(_, ext)| matches!(ext.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg")).unwrap_or(false)
}

fn choose_image<'a>(images: &[&'a ImageRecord], cover: bool, title: &str) -> Option<&'a ImageRecord> {
    let priorities: &[&str] = if cover { &["Box - Front", "Box - Front - Reconstructed", "Fanart - Box - Front", "Box - 3D"] }
        else { &["Disc", "Fanart - Disc"] };
    let region = title.to_ascii_lowercase();
    for kind in priorities {
        let candidates: Vec<_> = images.iter().copied().filter(|i| i.kind.eq_ignore_ascii_case(kind) && supported_image(&i.file)).collect();
        if candidates.is_empty() { continue; }
        if region.contains("(europe") || region.contains("(uk") {
            if let Some(i) = candidates.iter().copied().find(|i| i.region.to_ascii_lowercase().contains("europe") || i.region.to_ascii_lowercase().contains("united kingdom")) { return Some(i); }
        } else if region.contains("(usa") || region.contains("(north america") {
            if let Some(i) = candidates.iter().copied().find(|i| i.region.to_ascii_lowercase().contains("north america") || i.region.to_ascii_lowercase().contains("united states")) { return Some(i); }
        } else if region.contains("(japan") {
            if let Some(i) = candidates.iter().copied().find(|i| i.region.to_ascii_lowercase().contains("japan")) { return Some(i); }
        }
        if let Some(i) = candidates.iter().copied().find(|i| i.region.to_ascii_lowercase().contains("world")) { return Some(i); }
        return candidates.first().copied();
    }
    None
}

fn safe_name(title: &str) -> String {
    let name: String = title.chars().map(|c| if c.is_control() || matches!(c, '<'|'>'|':'|'"'|'/'|'\\'|'|'|'?'|'*') { '_' } else { c }).collect();
    name.trim().trim_matches('.').to_string()
}

fn extension(file: &str) -> &'static str {
    match file.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase()) {
        Some(ext) if ext == "jpg" => "jpg",
        Some(ext) if ext == "jpeg" => "jpeg",
        _ => "png",
    }
}

fn download_image(record: &ImageRecord, destination: &Path) -> bool {
    if destination.exists() { return false; }
    let file = record.file.trim_start_matches('/');
    if file.contains("..") || file.contains('\\') { return false; }
    let Some(bytes) = http_get(&format!("{IMAGE_BASE_URL}{file}"), IMAGE_MAX_BYTES) else { return false; };
    if !bytes.starts_with(&[137,80,78,71,13,10,26,10]) && !bytes.starts_with(&[0xFF,0xD8,0xFF]) { return false; }
    let Some(parent) = destination.parent() else { return false; };
    if fs::create_dir_all(parent).is_err() { return false; }
    let tmp = destination.with_extension(format!("{}.download", extension(&record.file)));
    if fs::write(&tmp, bytes).is_err() { return false; }
    if fs::rename(&tmp, destination).is_err() { let _ = fs::remove_file(&tmp); return false; }
    true
}

#[cfg(windows)]
fn http_get(url: &str, max_bytes: usize) -> Option<Vec<u8>> {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    type InternetHandle = *mut c_void;
    const INTERNET_OPEN_TYPE_PRECONFIG: u32 = 0;
    const INTERNET_FLAG_RELOAD: u32 = 0x8000_0000;
    const INTERNET_FLAG_NO_CACHE_WRITE: u32 = 0x0400_0000;
    const INTERNET_FLAG_KEEP_CONNECTION: u32 = 0x0040_0000;
    const INTERNET_FLAG_SECURE: u32 = 0x0080_0000;
    const INTERNET_OPTION_CONNECT_TIMEOUT: u32 = 2;
    const INTERNET_OPTION_RECEIVE_TIMEOUT: u32 = 6;
    #[link(name = "wininet")]
    unsafe extern "system" {
        fn InternetOpenW(agent: *const u16, access: u32, proxy: *const u16, bypass: *const u16, flags: u32) -> InternetHandle;
        fn InternetOpenUrlW(handle: InternetHandle, url: *const u16, headers: *const u16, headers_len: u32, flags: u32, context: usize) -> InternetHandle;
        fn InternetReadFile(handle: InternetHandle, buffer: *mut c_void, requested: u32, read: *mut u32) -> i32;
        fn InternetCloseHandle(handle: InternetHandle) -> i32;
        fn InternetSetOptionW(handle: InternetHandle, option: u32, buffer: *mut c_void, length: u32) -> i32;
    }
    struct Handle(InternetHandle);
    impl Drop for Handle { fn drop(&mut self) { if !self.0.is_null() { unsafe { InternetCloseHandle(self.0); } } } }
    let wide = |s: &str| std::ffi::OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect::<Vec<_>>();
    let agent = wide("OpenCDi/0.2");
    let url_w = wide(url);
    unsafe {
        let internet = Handle(InternetOpenW(agent.as_ptr(), INTERNET_OPEN_TYPE_PRECONFIG, std::ptr::null(), std::ptr::null(), 0));
        if internet.0.is_null() { return None; }
        let mut timeout: u32 = 15000;
        InternetSetOptionW(internet.0, INTERNET_OPTION_CONNECT_TIMEOUT, (&mut timeout as *mut u32).cast(), 4);
        InternetSetOptionW(internet.0, INTERNET_OPTION_RECEIVE_TIMEOUT, (&mut timeout as *mut u32).cast(), 4);
        let request = Handle(InternetOpenUrlW(internet.0, url_w.as_ptr(), std::ptr::null(), 0, INTERNET_FLAG_RELOAD | INTERNET_FLAG_NO_CACHE_WRITE | INTERNET_FLAG_KEEP_CONNECTION | INTERNET_FLAG_SECURE, 0));
        if request.0.is_null() { return None; }
        let mut body = Vec::new();
        let mut buffer = [0u8; 65536];
        loop {
            let mut count = 0u32;
            if InternetReadFile(request.0, buffer.as_mut_ptr().cast(), buffer.len() as u32, &mut count) == 0 { return None; }
            if count == 0 { break; }
            if body.len().saturating_add(count as usize) > max_bytes { return None; }
            body.extend_from_slice(&buffer[..count as usize]);
        }
        (!body.is_empty()).then_some(body)
    }
}

#[cfg(not(windows))]
fn http_get(_url: &str, _max_bytes: usize) -> Option<Vec<u8>> { None }

#[derive(Clone)]
struct ZipEntry { name: String, method: u16, compressed_size: usize, uncompressed_size: usize, offset: usize }

impl ZipEntry {
    fn open<'a>(&self, zip: &'a [u8]) -> Option<ZipReader<'a>> {
        let local = zip.get(self.offset..)?;
        if read_u32(local, 0)? != 0x04034b50 { return None; }
        let name_len = read_u16(local, 26)? as usize;
        let extra_len = read_u16(local, 28)? as usize;
        let start = 30usize.checked_add(name_len)?.checked_add(extra_len)?;
        let end = start.checked_add(self.compressed_size)?;
        let compressed = zip.get(start..end)?;
        match self.method {
            0 => Some(ZipReader::Stored(Cursor::new(compressed))),
            8 => Some(ZipReader::Deflate(DeflateDecoder::new(Cursor::new(compressed)))),
            _ => None,
        }
    }
}

enum ZipReader<'a> { Stored(Cursor<&'a [u8]>), Deflate(DeflateDecoder<Cursor<&'a [u8]>>) }
impl Read for ZipReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> { match self { Self::Stored(r) => r.read(buf), Self::Deflate(r) => r.read(buf) } }
}

fn zip_entries(zip: &[u8]) -> Option<Vec<ZipEntry>> {
    let search_start = zip.len().saturating_sub(65_557);
    let eocd = (search_start..zip.len().saturating_sub(3)).rev().find(|&p| read_u32(zip, p) == Some(0x06054b50))?;
    let count = read_u16(zip, eocd + 10)? as usize;
    let mut offset = read_u32(zip, eocd + 16)? as usize;
    let mut out = Vec::with_capacity(count.min(4096));
    for _ in 0..count {
        if read_u32(zip, offset)? != 0x02014b50 { return None; }
        let method = read_u16(zip, offset + 10)?;
        let compressed_size = read_u32(zip, offset + 20)? as usize;
        let uncompressed_size = read_u32(zip, offset + 24)? as usize;
        let name_len = read_u16(zip, offset + 28)? as usize;
        let extra_len = read_u16(zip, offset + 30)? as usize;
        let comment_len = read_u16(zip, offset + 32)? as usize;
        let local_offset = read_u32(zip, offset + 42)? as usize;
        let name = String::from_utf8_lossy(zip.get(offset + 46..offset + 46 + name_len)?).into_owned();
        out.push(ZipEntry { name, method, compressed_size, uncompressed_size, offset: local_offset });
        offset = offset.checked_add(46 + name_len + extra_len + comment_len)?;
    }
    Some(out)
}

fn read_u16(data: &[u8], at: usize) -> Option<u16> { Some(u16::from_le_bytes(data.get(at..at + 2)?.try_into().ok()?)) }
fn read_u32(data: &[u8], at: usize) -> Option<u32> { Some(u32::from_le_bytes(data.get(at..at + 4)?.try_into().ok()?)) }

fn parse_xml<R: BufRead>(reader: &mut R, wanted_titles: &HashSet<String>, wanted_ids: &HashSet<String>, out: &mut ParsedRecords) {
    let mut xml = Reader::from_reader(reader);
    xml.config_mut().trim_text(true);
    let mut buffer = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    let mut game: Option<GameRecord> = None;
    let mut image: Option<ImageRecord> = None;
    loop {
        match xml.read_event_into(&mut buffer) {
            Ok(Event::Start(e)) => {
                let tag = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                if tag.eq_ignore_ascii_case("Game") { game = Some(GameRecord::default()); }
                if tag.eq_ignore_ascii_case("GameImage") { image = Some(ImageRecord::default()); }
                stack.push(tag);
            }
            Ok(Event::Text(e)) => {
                let Ok(decoded) = e.decode() else { buffer.clear(); continue };
                let Ok(text) = quick_xml::escape::unescape(&decoded) else { buffer.clear(); continue };
                if let Some(tag) = stack.last().map(String::as_str) {
                    if let Some(i) = image.as_mut() {
                        match tag.to_ascii_lowercase().as_str() { "databaseid" => i.id.push_str(&text), "filename" => i.file.push_str(&text), "type" => i.kind.push_str(&text), "region" => i.region.push_str(&text), _ => {} }
                    } else if let Some(g) = game.as_mut() {
                        match tag.to_ascii_lowercase().as_str() { "databaseid" => g.id.push_str(&text), "name" => g.title.push_str(&text), "platform" => g.platform.push_str(&text), _ => {} }
                    }
                }
            }
            Ok(Event::CData(e)) => {
                if let Some(tag) = stack.last().map(String::as_str) {
                    if let Some(g) = game.as_mut() {
                        if tag.eq_ignore_ascii_case("Name") { if let Ok(s) = e.decode() { g.title.push_str(&s); } }
                    }
                }
            }
            Ok(Event::End(e)) => {
                let tag = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                stack.pop();
                if tag.eq_ignore_ascii_case("GameImage") {
                    if let Some(i) = image.take() { if wanted_ids.contains(&i.id) { out.images.push(i); } }
                } else if tag.eq_ignore_ascii_case("Game") {
                    if let Some(g) = game.take() {
                        if wanted_titles.is_empty() || wanted_titles.contains(&normalize_title(&g.title)) { out.games.push(g); }
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buffer.clear();
    }
}

