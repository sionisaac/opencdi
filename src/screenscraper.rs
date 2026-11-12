use crate::launchbox::{ArtworkKind, FetchTarget, http_get, http_response, normalize_title};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

static PUBLIC_ATTEMPTED: LazyLock<Mutex<HashSet<(String, ArtworkKind, Option<&'static str>)>>> = LazyLock::new(|| Mutex::new(HashSet::new()));
static SEARCH_GAMES: LazyLock<Mutex<HashMap<String, Option<Value>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

pub(crate) fn encode(value: &str) -> String {
    use std::fmt::Write;
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

pub(crate) fn search_name(title: &str) -> String {
    let plain = title.split(['(', '[']).next().unwrap_or(title).trim();
    if normalize_title(title) == "the7thguest" { return "The 7th Guest".into(); }
    if let Some((name, article)) = plain.rsplit_once(',') {
        let article = article.trim();
        if ["a", "an", "the"].iter().any(|word| article.eq_ignore_ascii_case(word)) {
            return format!("{article} {name}");
        }
    }
    plain.replace(" - ", " ")
}

fn relaxed_key(title: &str) -> String {
    let title = search_name(title).to_lowercase().replace([char::from(39), '’'], "");
    title.split(|c: char| !c.is_alphanumeric()).filter_map(|word| match word {
        "" | "the" | "a" | "an" => None,
        "ii" => Some("2"),
        "iii" => Some("3"),
        "iv" => Some("4"),
        _ => Some(word),
    }).collect()
}

type TitleIndex = HashMap<String, HashSet<u64>>;

pub(crate) fn public_game_id(title: &str) -> Option<u64> {
    static INDEX: LazyLock<(TitleIndex, TitleIndex)> = LazyLock::new(|| {
        let mut exact = TitleIndex::new();
        let mut relaxed = TitleIndex::new();
        for (id, alias) in include_str!("../assets/screenscraper-cdi-titles.tsv").lines().filter_map(|line| line.split_once('\t')) {
            let Ok(id) = id.parse() else { continue; };
            exact.entry(normalize_title(alias)).or_default().insert(id);
            relaxed.entry(relaxed_key(alias)).or_default().insert(id);
        }
        (exact, relaxed)
    });
    let ids = INDEX.0.get(&normalize_title(title)).or_else(|| INDEX.1.get(&relaxed_key(title)))?;
    if ids.len() == 1 { ids.iter().copied().next() } else { None }
}

fn game_id(game: &Value) -> Option<u64> {
    game["id"].as_u64().or_else(|| game["id"].as_str()?.parse().ok())
}

fn find_game<'a>(body: &'a Value, title: &str) -> Option<&'a Value> {
    let games = body.pointer("/response/jeux")?.as_array()?;
    if let Some(id) = public_game_id(title) {
        if let Some(game) = games.iter().find(|game| game_id(game) == Some(id)) { return Some(game); }
    }
    let requested_title = normalize_title(title);
    for key in [normalize_title as fn(&str) -> String, relaxed_key] {
        let requested = key(title);
        let mut matches = games.iter().filter(|game| {
            match (requested_title.as_str(), game_id(game)) {
                ("goldenoldiesjukebox", Some(104727 | 104728))
                | ("thejokerswildjr", Some(419679)) => return false,
                _ => {}
            }
            game["noms"].as_array().is_some_and(|names| names.iter().any(|name| {
                name["text"].as_str().is_some_and(|text| key(text) == requested)
            }))
        });
        if let Some(game) = matches.next() {
            return matches.next().is_none().then_some(game);
        }
    }
    None
}

fn english_synopsis(game: &Value) -> Option<&str> {
    game["synopsis"].as_array()?.iter()
        .filter(|entry| entry["langue"].as_str() == Some("en"))
        .filter_map(|entry| entry["text"].as_str())
        .find(|text| !text.trim().is_empty())
}

pub(crate) fn disc_regions(title: &str) -> Vec<&'static str> {
    let mut regions = Vec::new();
    let lower = title.to_ascii_lowercase();
    for tag in lower.split('(').skip(1).filter_map(|tag| tag.split_once(')').map(|(tag, _)| tag)) {
        if tag.contains(',') && tag.split(',').all(|part| part.trim().len() == 2) { continue; }
        for part in tag.split(',') {
            let code = region_code(part);
            if let Some(code) = code && !regions.contains(&code) {
                regions.push(code);
            }
        }
    }
    for fallback in ["us", "uk", "eu"] {
        if !regions.contains(&fallback) { regions.push(fallback); }
    }
    regions
}

pub(crate) fn region_code(region: &str) -> Option<&'static str> {
    Some(match region.trim().to_ascii_lowercase().as_str() {
        "usa" | "us" | "united states" | "north america" | "america" => "us",
        "uk" | "gb" | "gbr" | "united kingdom" | "great britain" | "england" => "uk",
        "europe" | "eur" | "eu" => "eu",
        "world" | "worldwide" | "wor" => "wor",
        "japan" | "jpn" | "jp" => "jp",
        "france" | "fr" => "fr",
        "germany" | "de" => "de",
        "italy" | "it" => "it",
        "netherlands" | "nl" => "nl",
        "sweden" | "se" => "se",
        "spain" | "es" => "es",
        "brazil" | "br" => "br",
        "australia" | "au" => "au",
        "canada" | "ca" => "ca",
        "belgium" | "be" => "be",
        "china" | "cn" => "cn",
        "korea" | "kr" => "kr",
        "asia" | "asi" => "asi",
        "taiwan" | "tw" => "tw",
        "portugal" | "pt" => "pt",
        "denmark" | "dk" => "dk",
        "norway" | "no" => "no",
        "finland" | "fi" => "fi",
        "russia" | "ru" => "ru",
        _ => return None,
    })
}

fn region(title: &str) -> &str {
    let lower = title.to_ascii_lowercase();
    for (tags, region) in [
        (&["(europe)", "(eur)"][..], "eu"),
        (&["(usa)", "(us)"][..], "us"),
        (&["(japan)", "(jpn)"][..], "jp"),
        (&["(france)"][..], "fr"),
        (&["(germany)"][..], "de"),
    ] {
        if tags.iter().any(|tag| lower.contains(tag)) { return region; }
    }
    "wor"
}

fn media_url<'a>(game: &'a Value, kind: ArtworkKind, title: &str, disc_region: Option<&str>) -> Option<&'a str> {
    let wanted_region = region(title);
    let wanted_disc = disc_number(title);
    game["medias"].as_array()?.iter().filter_map(|media| {
        let media_type = media["type"].as_str()?;
        let type_priority = match (kind, media_type) {
            (ArtworkKind::Cover, "box-2D") | (ArtworkKind::Disc, "support-2D") => 0,
            (ArtworkKind::Disc, "support-texture") => 1,
            (ArtworkKind::Disc, "support-scan") => 2,
            (ArtworkKind::Back, "box-2D-back") | (ArtworkKind::Logo, "wheel") => 0,
            (ArtworkKind::Logo, "wheel-hd") => 1,
            _ => return None,
        };
        let url = media["url"].as_str()?;
        if !url.starts_with("https://www.screenscraper.fr/") && !url.starts_with("https://screenscraper.fr/")
            && !url.starts_with("https://api.screenscraper.fr/") && !url.starts_with("https://medias.screenscraper.fr/") { return None; }
        let support = media.get("support").or_else(|| media.get("numsupport")).unwrap_or(&Value::Null);
        let disc = support.as_u64().or_else(|| support.as_str()?.parse().ok()).unwrap_or(0);
        if wanted_disc.is_some_and(|wanted| disc != 0 && disc != wanted) { return None; }
        if wanted_disc.is_none() && disc > 1 { return None; }
        let disc_priority = u8::from(wanted_disc != Some(disc));
        let media_region = media["region"].as_str().unwrap_or("");
        if let Some(required) = disc_region
            && region_code(media_region) != Some(required) { return None; }
        let region_priority = match media_region {
            value if value == wanted_region => 0,
            "wor" | "ss" | "" => 1,
            _ => 2,
        };
        Some(((region_priority, disc_priority, type_priority), url))
    }).min_by_key(|(priority, _)| *priority).map(|(_, url)| url)
}

fn preferred_filename(kind: ArtworkKind, title: &str, extension: &str) -> String {
    kind.filename(&format!("{title}--screenscraper"), extension)
}

pub fn preferred_gallery_image(folder: &Path, kind: ArtworkKind, title: &str) -> Option<PathBuf> {
    ["png", "jpg", "jpeg"].into_iter()
        .flat_map(|extension| {
            let preferred = folder.join(preferred_filename(kind, title, extension));
            let old = folder.join(format!("{}--screenscraper-disc.{extension}", crate::launchbox::safe_name(title)));
            [Some(preferred), (kind == ArtworkKind::Disc).then_some(old)].into_iter().flatten()
        })
        .find(|path| path.is_file())
}

fn public_gallery_url(game: &Value, kind: ArtworkKind, region: &str, disc: Option<u64>) -> Option<String> {
    let id = game["id"].as_u64().or_else(|| game["id"].as_str()?.parse().ok())?;
    let media = match kind {
        ArtworkKind::Boxart => return None,
        ArtworkKind::Disc => "support-2D",
        ArtworkKind::Cover => "box-2D",
        ArtworkKind::Back => "box-2D-back",
        ArtworkKind::Logo => "wheel",
    };
    let disc = disc.map(|number| number.to_string()).unwrap_or_default();
    Some(format!("https://www.screenscraper.fr/image.php?plateformid=133&gameid={id}&media={media}&hd=0&region={region}&num={disc}&version=&maxwidth=600&maxheight=600"))
}

fn image_extension(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") { Some("png") }
    else if bytes.starts_with(b"\xff\xd8\xff") { Some("jpg") }
    else { None }
}

fn disc_number(title: &str) -> Option<u64> {
    title.to_ascii_lowercase().split("(disc ").nth(1)
        .and_then(|part| part.split(')').next()).and_then(|part| part.parse().ok())
}

fn gallery_disc_candidates(title: &str) -> Vec<Option<u64>> {
    match disc_number(title) {
        Some(number) => vec![Some(number), None],
        None => vec![None],
    }
}

type GalleryImage = (Vec<u8>, &'static str);

fn probe_gallery_image(game: &Value, kind: ArtworkKind, title: &str, disc_region: Option<&str>) -> (Option<GalleryImage>, bool) {
    let regions = match disc_region {
        Some(code) => vec![code],
        None => {
            let requested = region(title);
            let mut regions = vec![requested];
            for fallback in ["wor", "ss"] {
                if !regions.contains(&fallback) { regions.push(fallback); }
            }
            regions
        }
    };
    let mut retry = false;
    for region in regions {
        for disc in gallery_disc_candidates(title) {
            let Some(url) = public_gallery_url(game, kind, region, disc) else { return (None, retry); };
            let response = http_response(&url, 24 * 1024 * 1024);
            std::thread::sleep(Duration::from_secs(1));
            match response {
                Some((200, bytes)) => {
                    if let Some(art) = crate::library::load_png_bytes(&bytes) {
                        if art.w >= 100 && art.h >= if kind == ArtworkKind::Logo { 20 } else { 100 } {
                            return (Some((bytes, "png")), false);
                        }
                        continue; // Missing-media icon.
                    }
                    // ScreenScraper can prepend a file-not-found warning to its missing-media icon.
                    let missing = b"No such file or directory";
                    let is_missing = bytes.windows(missing.len()).any(|part| part == missing);
                    retry |= !is_missing;
                }
                Some((404 | 410, _)) => {}
                _ => retry = true,
            }
        }
    }
    (None, retry)
}

pub fn fetch_missing(
    targets: &[FetchTarget], artwork_root: &Path,
    mut ready: impl FnMut(&str, ArtworkKind, &Path),
    mut synopsis_ready: impl FnMut(&Path, &str),
) {
    fetch_missing_for_kinds(targets, artwork_root, &ArtworkKind::DOWNLOADS, None, &mut ready, &mut synopsis_ready);
}

pub fn fetch_missing_for_kinds(
    targets: &[FetchTarget], artwork_root: &Path, kinds: &[ArtworkKind], disc_region: Option<&'static str>,
    mut ready: impl FnMut(&str, ArtworkKind, &Path),
    mut synopsis_ready: impl FnMut(&Path, &str),
) {
    let mut targets = targets.to_vec();
    for target in &mut targets {
        let Some(id) = public_game_id(&target.title) else { continue; };
        let game = serde_json::json!({"id": id});
        for &kind in kinds {
            if !target.missing.contains(&kind) { continue; }
            let key = (target.title.clone(), kind, disc_region);
            if PUBLIC_ATTEMPTED.lock().unwrap().contains(&key) { continue; }
            let (image, retry) = probe_gallery_image(&game, kind, &target.title, disc_region);
            let Some((bytes, extension)) = image else {
                if !retry { PUBLIC_ATTEMPTED.lock().unwrap().insert(key); }
                continue;
            };
            let filename = kind.filename(&target.title, if matches!(kind, ArtworkKind::Cover | ArtworkKind::Disc) { "png" } else { extension });
            let path = artwork_root.join(kind.folder()).join(filename);
            if crate::boxart::publish_media(kind, &path, &bytes) {
                ready(&target.title, kind, &path);
                target.missing.remove(&kind);
                PUBLIC_ATTEMPTED.lock().unwrap().insert(key);
            }
        }
    }
    let (Ok(id), Ok(password)) = (std::env::var("OPENCDI_SCREENSCRAPER_DEV_ID"), std::env::var("OPENCDI_SCREENSCRAPER_DEV_PASSWORD")) else { return; };
    if id.is_empty() || password.is_empty() { return; }
    let mut query = format!("devid={}&devpassword={}&softname=OpenCDi&output=json&systemeid=133", encode(&id), encode(&password));
    if let (Ok(user), Ok(password)) = (std::env::var("OPENCDI_SCREENSCRAPER_USER"), std::env::var("OPENCDI_SCREENSCRAPER_PASSWORD")) {
        query.push_str(&format!("&ssid={}&sspassword={}", encode(&user), encode(&password)));
    }
    for target in targets {
        let synopsis = target.synopsis_path.as_deref().filter(|path| !path.exists());
        if synopsis.is_none() && !kinds.iter().any(|kind| target.missing.contains(kind)) { continue; }
        let cached = SEARCH_GAMES.lock().unwrap().get(&target.title).cloned();
        let game = match cached {
            Some(game) => game,
            None => {
                let url = format!("https://api.screenscraper.fr/api2/jeuRecherche.php?{query}&recherche={}", encode(&search_name(&target.title)));
                let response = http_response(&url, 4 * 1024 * 1024);
                std::thread::sleep(Duration::from_secs(1));
                let game = match response {
                    Some((200, bytes)) => {
                        let Ok(body) = serde_json::from_slice::<Value>(&bytes) else { break; };
                        if !body.get("response").is_some_and(Value::is_object) { break; }
                        find_game(&body, &target.title).cloned()
                    }
                    Some((404, _)) => None,
                    _ => {
                        log::warn!("ScreenScraper stopped the artwork lookup");
                        break;
                    }
                };
                SEARCH_GAMES.lock().unwrap().insert(target.title.clone(), game.clone());
                game
            }
        };
        let Some(game) = game.as_ref() else { continue; };
        if let (Some(path), Some(text)) = (synopsis, english_synopsis(game)) {
            synopsis_ready(path, text);
        }
        for &kind in kinds {
            if kind == ArtworkKind::Boxart || !target.missing.contains(&kind) { continue; }
            let asset = media_url(game, kind, &target.title, disc_region)
                .and_then(|url| http_get(url, 24 * 1024 * 1024))
                .and_then(|bytes| image_extension(&bytes).map(|extension| (bytes, extension)));
            let Some((bytes, ext)) = asset else { continue; };
            let filename = kind.filename(&target.title, if matches!(kind, ArtworkKind::Cover | ArtworkKind::Disc) { "png" } else { ext });
            let path = artwork_root.join(kind.folder()).join(filename);
            if crate::boxart::publish_media(kind, &path, &bytes) {
                ready(&target.title, kind, &path);
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolves_cdi_dragon_lair_filenames_to_distinct_game_ids() {
        assert_eq!(public_game_id("Dragon's Lair (EUR).chd"), Some(104713));
        assert_eq!(public_game_id("Dragons Lair II - Time Warp (EUR).chd"), Some(104714));
        assert_eq!(public_game_id("Unknown CD-i disc"), None);
        assert_eq!(public_game_id("Joker's Wild!, The (USA)"), Some(419679));
        let junior = "Joker's Wild! Jr., The (USA)";
        assert_eq!(public_game_id(junior), None);
        let body = serde_json::json!({"response": {"jeux": [
            {"id": 419679, "noms": [{"text": "The Joker's Wild! Jr."}]}
        ]}});
        assert!(find_game(&body, junior).is_none());
        assert_eq!(public_game_id("Monty Python's Invasion from the Planet Skyron (Europe).chd"), Some(104762));
        assert_eq!(public_game_id("Invasion from the Planet Skyron"), Some(104762));
        for title in ["Golden Oldies Jukebox (Europe).chd", "Golden Oldies Jukebox (USA).chd"] {
            assert_eq!(public_game_id(title), None);
            let body = serde_json::json!({"response": {"jeux": [
                {"id": 104727, "noms": [{"text": "Golden Oldies Jukebox"}]},
                {"id": 104728, "noms": [{"text": "Golden Oldies Jukebox"}]}
            ]}});
            assert!(find_game(&body, title).is_none());
        }
    }

    #[test]
    fn never_requests_downloaded_3d_boxart() {
        let game = serde_json::json!({"id":"104723","medias":[{"type":"box-texture"}]});
        assert_eq!(media_url(&game, ArtworkKind::Boxart, "Flashback (Europe)", None), None);
        assert!(public_gallery_url(&game, ArtworkKind::Boxart, "eu", None).is_none());
        assert!(!ArtworkKind::DOWNLOADS.contains(&ArtworkKind::Boxart));
    }

    #[test]
    fn uses_support_2d_for_disc_art_and_keeps_the_disc_number() {
        let game = serde_json::json!({"id":104714});
        let url = public_gallery_url(&game, ArtworkKind::Disc, "eu", Some(2)).unwrap();
        assert!(url.contains("gameid=104714&media=support-2D"));
        assert!(url.contains("&region=eu&num=2&"));
        assert_eq!(preferred_filename(ArtworkKind::Disc, "Flashback (Europe)", "png"), "Flashback (Europe)--screenscraper.png");
    }

    #[test]
    fn thunder_disc_filename_matches_and_tries_shared_artwork_after_numbered_artwork() {
        let title = "Thunder in Paradise (Europe) (Disc 1) (The Game).chd";
        assert_eq!(public_game_id(title), Some(104801));
        assert_eq!(public_game_id("Thunder in Paradise Interactive"), Some(104801));
        assert_eq!(gallery_disc_candidates(title), vec![Some(1), None]);
        assert_eq!(gallery_disc_candidates("Brain Dead 13 (Europe) (Disc 2)"), vec![Some(2), None]);
        assert_eq!(gallery_disc_candidates("Flashback (Europe)"), vec![None]);
    }

    #[test]
    #[ignore = "Downloads live ScreenScraper artwork into OPENCDI_LIVE_ARTWORK_DIR"]
    fn thunder_disc_downloads_four_artwork_kinds() {
        let root = std::path::PathBuf::from(std::env::var_os("OPENCDI_LIVE_ARTWORK_DIR")
            .expect("isolated verification directory required"));
        let title = "Thunder in Paradise (Europe) (Disc 1) (The Game)";
        let target = FetchTarget {
            title: title.into(), missing: ArtworkKind::DOWNLOADS.into_iter().collect(),
            download_all: false, synopsis_path: None,
        };
        let mut kinds = HashSet::new();
        fetch_missing(&[target], &root, |received_title, kind, path| {
            assert_eq!(received_title, title);
            let image = crate::library::load_artwork(path).expect("valid downloaded artwork");
            if kind == ArtworkKind::Cover { assert_eq!((image.w, image.h), (600, 600)); }
            println!("{}: {} x {}", kind.folder(), image.w, image.h);
            kinds.insert(kind);
        }, |_, _| {});
        assert!(ArtworkKind::DOWNLOADS.into_iter().all(|kind| kinds.contains(&kind)));
    }

    #[test]
    fn disc_region_fallback_keeps_the_requested_region_first() {
        assert_eq!(disc_regions("Title (Sweden)"), ["se", "us", "uk", "eu"]);
        assert_eq!(disc_regions("Title (Europe) (En,Fr) (Disc 2)"), ["eu", "us", "uk"]);
        assert_eq!(disc_regions("Title (Japan)"), ["jp", "us", "uk", "eu"]);
        assert_eq!(disc_regions("Title (UK)"), ["uk", "us", "eu"]);
        assert_eq!(disc_regions("Title"), ["us", "uk", "eu"]);
    }

    #[test]
    fn selects_unique_game_and_regional_disc_media() {
        let game = serde_json::json!({
            "noms":[{"text":"Brain Dead 13"}],
            "synopsis":[{"langue":"fr","text":"FR"},{"langue":"en","text":"English synopsis"}],
            "medias":[
                {"type":"support-texture","region":"eu","support":"1","url":"https://www.screenscraper.fr/first"},
                {"type":"support-texture","region":"us","support":"2","url":"https://www.screenscraper.fr/second-us"},
                {"type":"support-texture","region":"eu","support":"2","url":"https://www.screenscraper.fr/second-eu"},
                {"type":"box-2D","region":"eu","url":"https://www.screenscraper.fr/front"},
                {"type":"box-2D-back","region":"eu","url":"https://www.screenscraper.fr/back"},
                {"type":"wheel","region":"wor","url":"https://www.screenscraper.fr/logo"}]
        });
        let body = serde_json::json!({"response":{"jeux":[game]}});
        let matched = find_game(&body, "Brain Dead 13 (Europe) (Disc 2)").unwrap();
        assert_eq!(english_synopsis(matched), Some("English synopsis"));
        assert_eq!(media_url(matched, ArtworkKind::Disc, "Brain Dead 13 (Europe) (Disc 2)", Some("eu")), Some("https://www.screenscraper.fr/second-eu"));
        assert_eq!(media_url(matched, ArtworkKind::Boxart, "Brain Dead 13 (Europe)", None), None);
        assert_eq!(media_url(matched, ArtworkKind::Back, "Brain Dead 13 (Europe)", None), Some("https://www.screenscraper.fr/back"));
        assert_eq!(media_url(matched, ArtworkKind::Logo, "Brain Dead 13 (Europe)", None), Some("https://www.screenscraper.fr/logo"));
        assert_eq!(encode("A & B"), "A%20%26%20B");
    }
}
