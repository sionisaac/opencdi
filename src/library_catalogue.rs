use crate::library::Item;
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Category {
    #[default]
    All,
    Games,
    Kids,
    Multimedia,
    Music,
    Video,
    Homebrew,
    Miscellaneous,
}

impl Category {
    pub const TABS: [Self; 8] = [
        Self::All, Self::Games, Self::Kids, Self::Multimedia,
        Self::Music, Self::Video, Self::Homebrew, Self::Miscellaneous,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Games => "Games",
            Self::Kids => "Kids",
            Self::Multimedia => "Multimedia",
            Self::Music => "Music",
            Self::Video => "Video",
            Self::Homebrew => "Homebrew",
            Self::Miscellaneous => "Miscellaneous",
        }
    }

    pub fn step(self, delta: isize) -> Self {
        Self::TABS[(self as isize + delta).rem_euclid(Self::TABS.len() as isize) as usize]
    }

    fn contains(self, mask: u8) -> bool {
        self == Self::All || mask & (1 << self as u8) != 0
    }
}

pub struct Title {
    mask: u8,
    pub name: String,
    slug: String,
}

impl Title {
    pub fn url(&self) -> String {
        format!("https://www.theworldofcdi.com/cd-i_encyclopedia/{}/", self.slug)
    }
}

struct Catalogue {
    titles: Vec<Title>,
    index: HashMap<String, Option<usize>>,
}

// Public catalogue snapshot, 10 November 2026. The masks combine its
// numbered categories into the library tabs. Aliases are edition-aware titles.
static CATALOGUE: LazyLock<Catalogue> = LazyLock::new(|| {
    let records: Vec<(u8, String, String, Vec<String>)> =
        serde_json::from_str(include_str!("../assets/world-of-cdi-catalogue.json"))
            .expect("valid bundled World of CD-i catalogue");
    let mut index = HashMap::new();
    let titles = records.into_iter().enumerate().map(|(id, (mask, name, slug, aliases))| {
        for title in std::iter::once(&name).chain(&aliases) {
            let key = title_key(title);
            index.entry(key).and_modify(|current| {
                if *current != Some(id) { *current = None; }
            }).or_insert(Some(id));
        }
        Title { mask, name, slug }
    }).collect();
    Catalogue { titles, index }
});

fn title_key(title: &str) -> String {
    let mut base = title.trim();
    let mut editions = Vec::new();
    while let Some(head) = base.strip_suffix(')')
        && let Some((rest, tag)) = head.rsplit_once('(')
    {
        let tag = tag.to_ascii_lowercase();
        match tag.as_str() {
            "simultaneous 2 player game" => editions.push("2 player version".to_owned()),
            _ if tag.contains("demo") || tag.contains("beta") || tag.contains("proto")
                || tag.contains("testing") => editions.push(tag),
            _ => {}
        }
        base = rest.trim_end();
    }
    let base = if editions.is_empty() {
        base.to_owned()
    } else {
        editions.reverse();
        format!("{base} {}", editions.join(" "))
    };
    crate::launchbox::normalize_title(&base.to_lowercase())
}

#[derive(Default)]
pub struct Collection {
    pub category: Category,
    masks: Vec<u8>,
    counts: [usize; 8],
    missing: [Vec<usize>; 8],
    pub missing_offset: Option<usize>,
}

pub const MISSING_ROWS: usize = 10;

impl Collection {
    pub fn refresh(&mut self, items: &[Item]) {
        let owned: Vec<_> = items.iter().map(|item| {
            CATALOGUE.index.get(&title_key(&item.title)).copied().flatten()
        }).collect();
        self.masks = owned.iter().map(|id| id.map_or(0, |id| CATALOGUE.titles[id].mask)).collect();
        let owned: HashSet<_> = owned.into_iter().flatten().collect();
        for category in Category::TABS {
            let index = category as usize;
            self.counts[index] = self.masks.iter().filter(|&&mask| category.contains(mask)).count();
            self.missing[index] = CATALOGUE.titles.iter().enumerate()
                .filter(|(id, title)| category.contains(title.mask) && !owned.contains(id))
                .map(|(id, _)| id).collect();
        }
        self.scroll_missing(0);
    }

    pub fn includes(&self, item: usize) -> bool {
        self.category.contains(self.masks.get(item).copied().unwrap_or(0))
    }

    pub fn owned_count(&self) -> usize {
        self.counts[self.category as usize]
    }

    pub fn missing_count(&self) -> usize {
        self.missing[self.category as usize].len()
    }

    pub fn missing_title(&self, index: usize) -> Option<&'static Title> {
        let id = *self.missing[self.category as usize].get(index)?;
        CATALOGUE.titles.get(id)
    }

    pub fn scroll_missing(&mut self, delta: isize) {
        let last = self.missing_count().saturating_sub(MISSING_ROWS);
        if let Some(offset) = &mut self.missing_offset {
            *offset = offset.saturating_add_signed(delta).min(last);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_keeps_numbered_editions_and_prototypes_distinct() {
        let lookup = |title| CATALOGUE.index.get(&title_key(title)).copied().flatten();
        let retail = lookup("International Tennis Open (USA) (En,Fr)").unwrap();
        let doubles = lookup("International Tennis Open (Europe) (En,Fr) (Simultaneous 2 Player Game)").unwrap();
        assert_ne!(retail, doubles);
        assert!(lookup("Zelda's Adventure (USA) (Beta) (1994-07-27)").is_none());
        assert_ne!(lookup("Golden Oldies Jukebox (Europe)"), lookup("Golden Oldies II (Europe)"));
        assert!(Category::Kids.contains(CATALOGUE.titles[lookup("Space Safari (UK)").unwrap()].mask));
    }
}
