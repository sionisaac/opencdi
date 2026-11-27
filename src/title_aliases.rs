//! Catalogue aliases exported from the editable disc-name workbook.
//! Exact identities only: ambiguous aliases never choose the first record.
use std::collections::HashMap;
use std::sync::LazyLock;
use serde_json::Value;

struct Index {
    names: Vec<(String, String)>,
    keys: HashMap<String, Option<usize>>,
}

pub(crate) fn edition(title: &str) -> String {
    let lower = title.to_lowercase();
    let words: Vec<_> = lower.split(|c: char| !c.is_alphanumeric()).collect();
    let mut tags = Vec::new();
    for (aliases, tag) in [(&["demo", "demonstration"][..], "demo"),
        (&["beta"][..], "beta"), (&["proto", "prototype"][..], "proto"),
        (&["testing"][..], "testing")]
    {
        if aliases.iter().any(|word| words.contains(word)) { tags.push(tag); }
    }
    // Beta and Testing Beta describe the same development status.
    if tags.contains(&"beta") { tags.retain(|tag| *tag != "testing"); }
    tags.join("-")
}

fn key(title: &str) -> String {
    format!("{}|{}", crate::launchbox::normalize_title_raw(title), edition(title))
}

impl Index {
    fn load(bundled: &str, extra: Option<&str>) -> Self {
        let data: Value = serde_json::from_str(bundled).expect("valid bundled title aliases");
        let mut index = Self { names: Vec::new(), keys: HashMap::new() };
        for title in data["titles"].as_array().expect("alias titles") {
            let (Some(slug), Some(name)) = (title["slug"].as_str(), title["name"].as_str()) else { continue; };
            let id = index.names.len();
            index.names.push((slug.into(), name.into()));
            index.add(name, id);
            if let Some(aliases) = title["aliases"].as_array() {
                for alias in aliases.iter().filter_map(Value::as_str) { index.add(alias, id); }
            }
        }
        if let Some(extra) = extra.and_then(|text| serde_json::from_str::<Value>(text).ok())
            && extra["version"].as_u64() == Some(1)
            && let Some(titles) = extra["titles"].as_array()
        {
            for title in titles.iter().take(5000) {
                let Some(id) = index.names.iter().position(|(slug, _)| Some(slug.as_str()) == title["slug"].as_str()) else { continue; };
                if let Some(aliases) = title["aliases"].as_array() {
                    for alias in aliases.iter().take(500).filter_map(Value::as_str) {
                        if alias.len() <= 4096 { index.add(alias, id); }
                    }
                }
            }
        }
        index
    }

    fn add(&mut self, alias: &str, id: usize) {
        if alias.trim().is_empty() { return; }
        self.keys.entry(key(alias)).and_modify(|current| {
            if *current != Some(id) { *current = None; }
        }).or_insert(Some(id));
    }

    fn resolve(&self, title: &str) -> Option<&(String, String)> {
        self.names.get(self.keys.get(&key(title)).copied().flatten()?)
    }
}

static INDEX: LazyLock<Index> = LazyLock::new(|| {
    let path = opencdi::frontend::app_data_dir().join("TitleAliases.json");
    let extra = std::fs::metadata(&path).ok().filter(|m| m.len() <= 8 * 1024 * 1024)
        .and_then(|_| std::fs::read_to_string(path).ok());
    Index::load(include_str!("../assets/title-aliases.json"), extra.as_deref())
});

pub(crate) fn canonical(title: &str) -> Option<&'static str> {
    INDEX.resolve(title).map(|(_, name)| name.as_str())
}

pub(crate) fn slug(title: &str) -> Option<&'static str> {
    INDEX.resolve(title).map(|(slug, _)| slug.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn regional_and_multidisc_aliases_share_identity() {
        assert_eq!(slug("Clue (USA)"), slug("Cluedo®"));
        assert_eq!(slug("7th Guest, The (1994)(Philips)(DE)(Disc 1 of 2)[!][Das Spiel][DVC]"), slug("The 7th Guest"));
        assert_eq!(slug("7th Guest, The (Europe) (Disc 2)"), slug("The 7th Guest"));
        assert_ne!(slug("Golden Oldies Jukebox (Europe)"), slug("Golden Oldies II (Europe)"));
        assert_ne!(slug("Joker's Wild!, The (USA)"), slug("Joker's Wild! Jr., The (USA)"));
    }
    #[test]
    fn overrides_reject_unknown_records_and_ambiguous_names() {
        let base = r#"{"titles":[{"slug":"a","name":"One","aliases":[]},{"slug":"b","name":"Two","aliases":[]}]}"#;
        let index = Index::load(base, Some(r#"{"version":1,"titles":[{"slug":"a","aliases":["Shared"]},{"slug":"b","aliases":["Shared"]},{"slug":"fake","aliases":["Other"]}]}"#));
        assert!(index.resolve("Shared").is_none());
        assert!(index.resolve("Other").is_none());
        assert!(index.resolve("One (Beta)").is_none());
        assert!(index.resolve("One (USA)").is_some());
    }

    #[test]
    fn dump_metadata_does_not_erase_edition_identity() {
        use crate::launchbox::normalize_title as normal;
        assert_eq!(normal("Clue (1994)(Philips)(US)[DVC].chd"), normal("Cluedo®"));
        assert_eq!(normal("Clue (France)"), normal("Cluedo®"));
        assert_ne!(normal("Zelda's Adventure (USA) (Beta)"), normal("Zelda's Adventure (USA)"));
        assert_ne!(normal("Golden Oldies Jukebox"), normal("Golden Oldies II"));
        assert_ne!(normal("Grolier 1997"), normal("Grolier 1998"));
        assert_ne!(normal("Joker's Wild! Jr., The (USA)"), normal("Joker's Wild!, The (USA)"));
    }
}
