use crate::launchbox::{FetchTarget, http_response, normalize_title};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

const SITE: &str = "https://www.theworldofcdi.com";
const PAGE_LIMIT: usize = 2 * 1024 * 1024;
const MISS_RETRY: Duration = Duration::from_secs(24 * 60 * 60);
type TitleKey = (String, bool);
static MISSES: LazyLock<Mutex<HashMap<TitleKey, Instant>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn fill_missing(targets: &[FetchTarget]) {
    MISSES
        .lock()
        .unwrap()
        .retain(|_, time| time.elapsed() < MISS_RETRY);
    let mut results = HashMap::new();
    for target in targets {
        let Some(path) = target
            .synopsis_path
            .as_deref()
            .filter(|path| !path.exists())
        else {
            continue;
        };
        let key = title_key(&target.title);
        if key.0.is_empty() || MISSES.lock().unwrap().contains_key(&key) {
            continue;
        }
        if !results.contains_key(&key) {
            match lookup(&target.title) {
                Ok(Some(text)) => {
                    results.insert(key.clone(), text);
                }
                Ok(None) => {
                    MISSES.lock().unwrap().insert(key, Instant::now());
                    continue;
                }
                // Stop the pass if the server is unavailable or limiting requests.
                Err(()) => break,
            }
        }
        if let Some(text) = results.get(&key) {
            crate::synopsis::save(path, text);
        }
    }
}

fn lookup(title: &str) -> Result<Option<String>, ()> {
    let query = crate::screenscraper::encode(&crate::screenscraper::search_name(title));
    let Some(search) = get_page(&format!("{SITE}/?s={query}"))? else {
        return Ok(None);
    };
    let Some(url) = find_entry(&search, title) else {
        return Ok(None);
    };
    let Some(page) = get_page(url)? else {
        return Ok(None);
    };
    Ok(description(&page, title).map(|text| format!("Source: The World of CD-i\n{url}\n\n{text}")))
}

fn get_page(url: &str) -> Result<Option<String>, ()> {
    let response = http_response(url, PAGE_LIMIT);
    std::thread::sleep(Duration::from_secs(1));
    match response {
        Some((200, body)) => String::from_utf8(body).map(Some).map_err(|_| ()),
        Some((404 | 410, _)) => Ok(None),
        _ => Err(()),
    }
}

fn title_key(title: &str) -> TitleKey {
    let demo = title.split(|c: char| !c.is_alphanumeric()).any(|word| {
        word.eq_ignore_ascii_case("demo") || word.eq_ignore_ascii_case("demonstration")
    });
    (normalize_title(title), demo)
}

fn find_entry<'a>(html: &'a str, title: &str) -> Option<&'a str> {
    let key = title_key(title);
    let mut urls = ["h1", "h2"]
        .into_iter()
        .flat_map(|tag| elements(html, tag))
        .filter(|(heading, text)| {
            heading.contains("entry-title") && title_key(&plain_text(text)) == key
        })
        .filter_map(|(_, heading)| elements(heading, "a").next())
        .filter_map(|(anchor, _)| {
            let value = anchor.split_once("href=")?.1;
            let quote = value.chars().next().filter(|c| matches!(c, '"' | '\''))?;
            let url = value[1..].split_once(quote)?.0;
            let slug = url
                .strip_prefix(SITE)?
                .strip_prefix("/cd-i_encyclopedia/")?
                .strip_suffix('/')?;
            (!slug.is_empty() && slug.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
                .then_some(url)
        });
    let url = urls.next()?;
    urls.all(|other| other == url).then_some(url)
}

fn description(html: &str, title: &str) -> Option<String> {
    let (_, text) = elements(html, "h1").find(|(tag, _)| tag.contains("entry-title"))?;
    if title_key(&plain_text(text)) != title_key(title) {
        return None;
    }
    let row = elements(html, "tr")
        .find(|(_, row)| {
            elements(row, "th")
                .next()
                .is_some_and(|(_, label)| plain_text(label) == "Description")
        })?
        .1;
    let text = plain_text(elements(row, "td").next()?.1);
    (!text.is_empty()).then_some(text)
}

// The encyclopedia uses closed headings, links and table cells.
fn elements<'a>(html: &'a str, tag: &str) -> impl Iterator<Item = (&'a str, &'a str)> {
    let opening = format!("<{tag}");
    let closing = format!("</{tag}>");
    let mut remaining = html;
    std::iter::from_fn(move || {
        while let Some((_, tail)) = remaining.split_once(&opening) {
            remaining = tail;
            if !tail.starts_with(['>', ' ', '\t', '\n', '\r']) {
                continue;
            }
            let (attributes, body) = tail.split_once('>')?;
            let (contents, rest) = body.split_once(&closing)?;
            remaining = rest;
            return Some((attributes, contents));
        }
        None
    })
}

fn plain_text(html: &str) -> String {
    let mut text = String::new();
    let mut remaining = html;
    while let Some((before, tail)) = remaining.split_once('<') {
        text.push_str(before);
        let Some((tag, after)) = tail.split_once('>') else {
            break;
        };
        if matches!(
            tag.split_ascii_whitespace().next().unwrap_or(""),
            "br" | "br/" | "/p" | "/li"
        ) {
            text.push(' ');
        }
        remaining = after;
    }
    text.push_str(remaining);
    // Decode entities independently so a literal ampersand doesn't block the rest.
    let mut decoded = String::new();
    let mut parts = text.split('&');
    decoded.push_str(parts.next().unwrap_or_default());
    for part in parts {
        let entity = part.split_once(';').filter(|(name, _)| name.len() <= 16);
        if let Some((name, rest)) = entity {
            let encoded = format!("&{name};");
            if let Ok(value) = quick_xml::escape::unescape_with(&encoded, |name| {
                quick_xml::escape::resolve_predefined_entity(name).or(match name {
                    "nbsp" => Some(" "),
                    "rsquo" | "lsquo" => Some("’"),
                    "rdquo" | "ldquo" => Some("”"),
                    "ndash" => Some("–"),
                    "mdash" => Some("—"),
                    "hellip" => Some("…"),
                    _ => None,
                })
            }) {
                decoded.push_str(&value);
                decoded.push_str(rest);
                continue;
            }
        }
        decoded.push('&');
        decoded.push_str(part);
    }
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_the_matching_encyclopedia_entry() {
        let search = r#"
            <h2 class="entry-title"><a href="https://www.theworldofcdi.com/cd-i_encyclopedia/golden-oldies-jukebox-demo/">Golden Oldies Jukebox – Demo</a></h2>
            <h2 class="entry-title"><a href="https://www.theworldofcdi.com/cd-i_encyclopedia/golden-oldies-jukebox/">Golden Oldies Jukebox</a></h2>"#;
        assert_eq!(
            find_entry(search, "Golden Oldies Jukebox (USA)"),
            Some("https://www.theworldofcdi.com/cd-i_encyclopedia/golden-oldies-jukebox/")
        );
        assert_eq!(find_entry(search, "Golden Oldies II"), None);
        assert_eq!(
            find_entry(
                &search.replace("https://www.theworldofcdi.com", "https://example.com"),
                "Golden Oldies Jukebox (USA)"
            ),
            None
        );
    }

    #[test]
    fn extracts_description_and_verifies_page_title() {
        let page = r#"<h1 class="entry-title"><a>Joker&#8217;s Wild! Jr.</a></h1>
            <table><tr><th scope="row">Description</th><td>A &amp; B.<br />A <em>new</em> round.</td></tr>
            <tr><th>Remarks</th><td>Ignored.</td></tr></table>"#;
        assert_eq!(
            description(page, "Joker's Wild! Jr. (USA)").as_deref(),
            Some("A & B. A new round.")
        );
        assert_eq!(description(page, "Joker's Wild! (USA)"), None);
    }
}
