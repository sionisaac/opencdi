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
    pub missing: HashSet<ArtworkKind>,
    pub download_all: bool,
    pub synopsis_path: Option<std::path::PathBuf>,
}

#[derive(Default)]
struct GameRecord { id: String, title: String, platform: String, overview: String }

#[derive(Default)]
struct ImageRecord { id: String, file: String, kind: String, region: String }

#[derive(Default)]
struct ParsedRecords { games: Vec<GameRecord>, images: Vec<ImageRecord> }

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArtworkKind { Cover, Disc, Boxart, Back, Logo }

impl ArtworkKind {
    pub const ALL: [Self; 5] = [Self::Cover, Self::Disc, Self::Boxart, Self::Back, Self::Logo];

    pub fn folder(self) -> &'static str {
        match self {
            Self::Cover => "Covers",
            Self::Disc => "CD",
            Self::Boxart => "Boxart",
            Self::Back => "Back",
            Self::Logo => "Logo",
        }
    }

    pub fn filename(self, title: &str, extension: &str) -> String {
        let suffix = if self == Self::Disc { "-disc" } else { "" };
        format!("{}{suffix}.{extension}", safe_name(title))
    }
}

pub fn fetch_missing_artwork(
    targets: &[FetchTarget],
    artwork_root: &Path,
    cache_dir: &Path,
    mut image_ready: impl FnMut(&str, ArtworkKind, &Path),
    prefer_artwork: impl FnOnce(&mut [FetchTarget], &mut dyn FnMut(&str, ArtworkKind, &Path)),
) -> usize {
    let media_marks = cache_dir.join("LaunchBox-Media");
    let marker = |title: &str| media_marks.join(format!("{}.done", safe_name(title)));
    let needs_media = |target: &FetchTarget| !marker(&target.title).is_file();
    let mut pending: Vec<_> = targets.iter().filter(|t| !t.missing.is_empty() || t.download_all || t.synopsis_path.is_some() || needs_media(t)).cloned().collect();
    if pending.is_empty() { return 0; }
    let bezels_dir = artwork_root.join("Bezels");
    let media_dir = artwork_root.join("LaunchBox");
    // Resolve and publish synopsis text before any network-heavy artwork work.
    let prepared = (|| {
        if fs::create_dir_all(&bezels_dir).is_err() || fs::create_dir_all(&media_dir).is_err() || fs::create_dir_all(cache_dir).is_err() { return None; }
        let archive = cache_dir.join("LaunchBox-Metadata.zip");
        if !metadata_is_fresh(&archive) {
            if let Some(bytes) = http_get(METADATA_URL, METADATA_MAX_BYTES) {
                let tmp = archive.with_extension("zip.download");
                if fs::write(&tmp, bytes).and_then(|_| fs::rename(&tmp, &archive)).is_err() {
                    let _ = fs::remove_file(&tmp);
                    log::warn!("LaunchBox metadata archive could not be refreshed");
                }
            } else if !archive.is_file() {
                log::warn!("LaunchBox metadata archive is unavailable");
                return None;
            }
        }
        let zip = fs::read(&archive).ok()?;
        let Some(entries) = zip_entries(&zip) else { log::warn!("LaunchBox metadata archive could not be read"); return None; };
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
            if exact.is_empty() && let Some(game_id) = crate::screenscraper::public_game_id(&target.title) {
                exact.extend(records.games.iter().filter(|game| !game.id.is_empty()
                    && crate::screenscraper::public_game_id(&game.title) == Some(game_id))
                    .map(|game| game.id.clone()));
            }
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
        if matched.is_empty() { return None; }
        for target in &pending {
            let Some(path) = target.synopsis_path.as_deref() else { continue; };
            let Some(ids) = matched.get(&normalize_title(&target.title)).filter(|ids| ids.len() == 1) else { continue; };
            if let Some(game) = records.games.iter().find(|game| game.id == ids[0] && !game.overview.trim().is_empty()) {
                crate::synopsis::save(path, &game.overview);
            }
        }
        Some((zip, entries, records, matched))
    })();
    prefer_artwork(&mut pending, &mut image_ready);
    let Some((zip, entries, mut records, matched)) = prepared else { return 0; };
    if !pending.iter().any(|t| !t.missing.is_empty() || t.download_all || needs_media(t)) { return 0; }
    let ids: HashSet<String> = matched.values().flatten().cloned().collect();
    records.images.clear();
    for entry in &entries {
        if !entry.name.to_ascii_lowercase().ends_with(".xml") || entry.uncompressed_size > 512 * 1024 * 1024 { continue; }
        let Some(stream) = entry.open(&zip) else { continue; };
        parse_xml(&mut io::BufReader::new(stream), &HashSet::new(), &ids, &mut records);
    }
    let mut saved = 0;
    let mut extras = Vec::new();
    let mut completed_media = HashMap::new();
    for target in pending.iter().filter(|t| !t.missing.is_empty() || t.download_all || needs_media(t)) {
        let key = normalize_title(&target.title);
        let Some(game_ids) = matched.get(&key) else { continue };
        for game_id in game_ids {
            let candidates: Vec<&ImageRecord> = records.images.iter().filter(|i| i.id == *game_id).collect();
            for kind in ArtworkKind::ALL {
                if !target.missing.contains(&kind) && !target.download_all { continue; }
                if let Some(image) = choose_image(&candidates, kind, &target.title) {
                    let path = artwork_root.join(kind.folder()).join(kind.filename(&target.title, extension(&image.file)));
                    let restored = supplemental_path(&media_dir, &bezels_dir, &target.title, image)
                        .and_then(|cached| fs::read(cached).ok())
                        .is_some_and(|bytes| crate::synopsis::publish_new(&path, &bytes));
                    if restored || download_image(image, &path) {
                        saved += 1;
                        image_ready(&target.title, kind, &path);
                    }
                }
            }
            if needs_media(target) || target.download_all {
                extras.push((target, candidates));
            }
        }
    }
    // Fill the library and boot screen before downloading supplemental media.
    for (target, candidates) in extras {
        let mut stored = HashSet::new();
        let mut complete = !candidates.is_empty();
        for image in candidates.iter().copied() {
            if !supported_image(&image.file) || image.kind.is_empty() { continue; }
            let Some(destination) = supplemental_path(&media_dir, &bezels_dir, &target.title, image) else { continue; };
            if !stored.insert(destination.clone()) { continue; }
            if download_image(image, &destination) { saved += 1; }
            complete &= destination.is_file();
        }
        *completed_media.entry(&target.title).or_insert(true) &= complete;
    }
    if fs::create_dir_all(&media_marks).is_ok() {
        for (title, complete) in completed_media {
            if complete { let _ = fs::write(marker(title), []); }
        }
    }
    log::info!("LaunchBox artwork lookup saved {} image(s)", saved);
    saved
}

fn supplemental_path(media_dir: &Path, bezels_dir: &Path, title: &str, image: &ImageRecord) -> Option<std::path::PathBuf> {
    if !supported_image(&image.file) || image.kind.is_empty() { return None; }
    let filename = image.file.rsplit('/').next()?;
    let stem = Path::new(filename).file_stem()?.to_str()?;
    let kind = image.kind.to_ascii_lowercase();
    let folder = if kind.contains("disc") { media_dir.join("Disc") }
        else if kind.contains("fanart - background") || kind == "background" { bezels_dir.to_path_buf() }
        else { media_dir.join(safe_name(&image.kind)) };
    Some(folder.join(format!("{}--{}.{}", safe_name(title), safe_name(stem), extension(&image.file))))
}

fn metadata_is_fresh(path: &Path) -> bool {
    let Ok(meta) = fs::metadata(path) else { return false };
    let Ok(modified) = meta.modified() else { return false };
    SystemTime::now().duration_since(modified).unwrap_or(Duration::MAX) < Duration::from_secs(24 * 60 * 60)
}

pub(crate) fn normalize_title(s: &str) -> String {
    let s = s.trim();
    let s = s.rsplit_once('.').filter(|(_, ext)| matches!(ext.to_ascii_lowercase().as_str(), "chd" | "cue" | "iso" | "bin")).map(|(stem, _)| stem).unwrap_or(s);
    let mut normalized = String::new();
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '(' || ch == '[' {
            let closing = if ch == '(' { ')' } else { ']' };
            let mut tag = String::new();
            while let Some(c) = chars.next() { if c == closing { break; } tag.push(c); }
            let tag_l = tag.to_ascii_lowercase();
            let is_dump_tag = ["usa", "eur", "europe", "european", "japan", "japanese", "world", "disc", "disk", "cd", "dvc", "track", "rev", "revision", "version", "beta", "proto", "demo", "pal", "ntsc", "en,", "fr,", "de,"]
                .iter().any(|needle| tag_l.contains(needle));
            if !is_dump_tag { normalized.extend(tag.chars().filter(|c| c.is_alphanumeric()).map(|c| c.to_ascii_lowercase())); }
        } else if ch.is_alphanumeric() { normalized.push(ch.to_ascii_lowercase()); }
    }
    // This dump-set subtitle is not part of LaunchBox's title.
    if normalized == "7thguestthethegame" { return "the7thguest".into(); }
    // Dump names commonly sort the article after the title ("7th Guest, The").
    if s.to_ascii_lowercase().contains(", the") {
        if normalized.ends_with("the") { normalized.truncate(normalized.len() - 3); normalized.insert_str(0, "the"); }
    }
    for article in ["a", "an"] {
        if s.rsplit_once(',').is_some_and(|(_, suffix)| suffix.trim().split(['(', '[']).next().is_some_and(|part| part.trim().eq_ignore_ascii_case(article)))
            && normalized.ends_with(article)
        {
            normalized.truncate(normalized.len() - article.len());
            normalized.insert_str(0, article);
            break;
        }
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

fn choose_image<'a>(images: &[&'a ImageRecord], kind: ArtworkKind, title: &str) -> Option<&'a ImageRecord> {
    let priorities: &[&str] = match kind {
        ArtworkKind::Cover => &["Box - Front", "Box - Front - Reconstructed", "Fanart - Box - Front"],
        ArtworkKind::Disc => &["Disc", "Fanart - Disc"],
        ArtworkKind::Boxart => &["Box - 3D"],
        ArtworkKind::Back => &["Box - Back", "Box - Back - Reconstructed", "Fanart - Box - Back"],
        ArtworkKind::Logo => &["Clear Logo"],
    };
    let region = title.to_ascii_lowercase();
    for kind in priorities {
        let candidates: Vec<_> = images.iter().copied().filter(|i| i.kind.eq_ignore_ascii_case(kind) && supported_image(&i.file)).collect();
        if candidates.is_empty() { continue; }
        if region.contains("(eur") || region.contains("(uk") {
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

pub(crate) fn safe_name(title: &str) -> String {
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
pub(crate) fn http_response(url: &str, max_bytes: usize) -> Option<(u32, Vec<u8>)> {
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
        fn HttpQueryInfoW(handle: InternetHandle, level: u32, buffer: *mut c_void, length: *mut u32, index: *mut u32) -> i32;
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
        let mut status = 0u32;
        let mut length = 4u32;
        if HttpQueryInfoW(request.0, 19 | 0x2000_0000, (&mut status as *mut u32).cast(), &mut length, std::ptr::null_mut()) == 0 {
            return None;
        }
        let mut body = Vec::new();
        let mut buffer = [0u8; 65536];
        loop {
            let mut count = 0u32;
            if InternetReadFile(request.0, buffer.as_mut_ptr().cast(), buffer.len() as u32, &mut count) == 0 { return None; }
            if count == 0 { break; }
            if body.len().saturating_add(count as usize) > max_bytes { return None; }
            body.extend_from_slice(&buffer[..count as usize]);
        }
        Some((status, body))
    }
}

#[cfg(not(windows))]
pub(crate) fn http_response(_url: &str, _max_bytes: usize) -> Option<(u32, Vec<u8>)> { None }

pub(crate) fn http_get(url: &str, max_bytes: usize) -> Option<Vec<u8>> {
    let (status, body) = http_response(url, max_bytes)?;
    (status == 200 && !body.is_empty()).then_some(body)
}

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
        let compressed = local.get(start..end)?;
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

#[cfg(windows)]
pub(crate) fn read_unique_zip_member(zip: &[u8], names: &[&str], max_bytes: usize) -> Option<Vec<u8>> {
    let mut entries = zip_entries(zip)?.into_iter()
        .filter(|entry| names.contains(&entry.name.as_str()));
    let entry = entries.next()?;
    if entries.next().is_some() || entry.uncompressed_size > max_bytes {
        return None;
    }
    let mut bytes = Vec::new();
    entry.open(zip)?.take(entry.uncompressed_size as u64 + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() == entry.uncompressed_size).then_some(bytes)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "Downloads current LaunchBox metadata and artwork; set OPENCDI_LIVE_ARTWORK_DIR"]
    fn live_artwork_downloads() {
        let root = std::path::PathBuf::from(std::env::var_os("OPENCDI_LIVE_ARTWORK_DIR").expect("isolated verification directory required"));
        let titles = ["7th Guest, The - The Game (EUR) [DVC]", "Backgammon (EUR)", "Brain Dead 13 (EUR) [DVC]", "Burn - Cycle (EUR)"];
        let targets: Vec<_> = titles.iter().map(|title| FetchTarget { title: (*title).into(), missing: ArtworkKind::ALL.into_iter().collect(), download_all: true, synopsis_path: None }).collect();
        let _ = env_logger::builder().is_test(true).filter_level(log::LevelFilter::Info).try_init();
        let count = fetch_missing_artwork(&targets, &root.join("artwork"), &root.join("cache"), |_, _, _| {}, |_, _| {});
        println!("Downloaded {count} images");
        for title in titles {
            let found = fs::read_dir(root.join("artwork/Covers")).unwrap().flatten().any(|entry| entry.path().file_stem().unwrap().to_string_lossy() == safe_name(title));
            assert!(found, "No cover downloaded for {title}");
        }
        assert!(count >= 4);
    }

    #[test]
    fn reads_launchbox_overview_and_xml_references() {
        let xml = "<LaunchBox><Game><Name>A National Parks Tour</Name><Platform>Philips CD-i</Platform><DatabaseID>1</DatabaseID><Overview>Visit Yellowstone &amp; Yosemite. <![CDATA[Explore the Grand Canyon.]]></Overview></Game></LaunchBox>";
        let mut records = ParsedRecords::default();
        parse_xml(&mut xml.as_bytes(), &HashSet::new(), &HashSet::new(), &mut records);
        assert_eq!(records.games[0].overview, "Visit Yellowstone & Yosemite. Explore the Grand Canyon.");
        assert_eq!(normalize_title("National Parks Tour, A.chd"), normalize_title(&records.games[0].title));
    }

    #[test]
    fn dump_filenames_match_catalog_titles() {
        for (filename, title) in [
            ("7th Guest, The - The Game (EUR) [DVC].chd", "The 7th Guest"),
            ("Backgammon (EUR).chd", "Backgammon"),
            ("Brain Dead 13 (EUR) [DVC].chd", "Brain Dead 13"),
            ("Burn - Cycle (EUR).chd", "Burn:Cycle"),
            ("7th Guest, The (Europe).chd", "The 7th Guest"),
        ] {
            assert_eq!(normalize_title(filename), normalize_title(title), "{filename}");
        }
        assert_ne!(normalize_title("Backgammon"), normalize_title("Super Backgammon"));
    }

    #[test]
    fn eur_prefers_european_artwork() {
        let us = ImageRecord { region: "North America".into(), kind: "Box - Front".into(), file: "us.jpg".into(), ..Default::default() };
        let eu = ImageRecord { region: "Europe".into(), file: "eu.jpg".into(), kind: "Box - Front".into(), ..Default::default() };
        assert_eq!(choose_image(&[&us, &eu], ArtworkKind::Cover, "Backgammon (EUR)").unwrap().file, "eu.jpg");
    }

    #[test]
    fn zip_entry_reads_data_relative_to_its_local_header() {
        let payload = b"<Data/>";
        let mut zip = vec![0u8; 47];
        zip.extend_from_slice(&0x04034b50u32.to_le_bytes());
        zip.resize(47 + 30, 0);
        zip.extend_from_slice(payload);
        let entry = ZipEntry { name: "metadata.xml".into(), method: 0, compressed_size: payload.len(), uncompressed_size: payload.len(), offset: 47 };
        let mut actual = Vec::new();
        entry.open(&zip).unwrap().read_to_end(&mut actual).unwrap();
        assert_eq!(actual, payload);
    }
}

fn parse_xml<R: BufRead>(reader: &mut R, wanted_titles: &HashSet<String>, wanted_ids: &HashSet<String>, out: &mut ParsedRecords) {
    let mut xml = Reader::from_reader(reader);
    xml.config_mut().trim_text(false);
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
                        match tag.to_ascii_lowercase().as_str() { "databaseid" => g.id.push_str(&text), "name" => g.title.push_str(&text), "platform" => g.platform.push_str(&text), "overview" | "notes" => g.overview.push_str(&text), _ => {} }
                    }
                }
            }
            Ok(Event::GeneralRef(e)) => {
                if let Ok(name) = e.decode() {
                    let escaped = format!("&{name};");
                    if let Ok(value) = quick_xml::escape::unescape(&escaped) {
                        match stack.last().map(String::as_str) {
                            Some("Overview" | "Notes") => { if let Some(g) = &mut game { g.overview.push_str(&value); } }
                            Some("Name") => { if let Some(g) = &mut game { g.title.push_str(&value); } }
                            _ => {}
                        }
                    }
                }
            }
            Ok(Event::CData(e)) => {
                if let Some(tag) = stack.last().map(String::as_str) {
                    if let Some(g) = game.as_mut() {
                        if let Ok(s) = e.decode() {
                            match tag.to_ascii_lowercase().as_str() {
                                "name" => g.title.push_str(&s),
                                "overview" | "notes" => g.overview.push_str(&s),
                                _ => {}
                            }
                        }
                    }
                }
            }
            Ok(Event::End(e)) => {
                let tag = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                stack.pop();
                if tag.eq_ignore_ascii_case("GameImage") {
                    if let Some(i) = image.take() { if wanted_ids.contains(&i.id) { out.images.push(i); } }
                } else if tag.eq_ignore_ascii_case("Game") {
                    if let Some(mut g) = game.take() {
                        g.id = g.id.trim().to_owned();
                        g.title = g.title.trim().to_owned();
                        g.platform = g.platform.trim().to_owned();
                        if wanted_ids.is_empty() && g.platform.eq_ignore_ascii_case("Philips CD-i")
                            && (wanted_titles.is_empty() || wanted_titles.contains(&normalize_title(&g.title)))
                        {
                            out.games.push(g);
                        }
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buffer.clear();
    }
}

