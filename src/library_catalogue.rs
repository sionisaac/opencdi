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
    bundles: Vec<(usize, Vec<usize>)>,
}

// Public catalogue snapshot, 10 November 2026. The masks combine its
// numbered categories into the library tabs. Aliases are edition-aware titles.
static CATALOGUE: LazyLock<Catalogue> = LazyLock::new(|| {
    let records: Vec<(u8, String, String, Vec<String>)> =
        serde_json::from_str(include_str!("../assets/world-of-cdi-catalogue.json"))
            .expect("valid bundled World of CD-i catalogue");
    let mut index = HashMap::new();
    let titles: Vec<_> = records.into_iter().enumerate().map(|(id, (mask, name, slug, aliases))| {
        for title in std::iter::once(&name).chain(&aliases) {
            let key = title_key(title);
            index.entry(key).and_modify(|current| {
                if *current != Some(id) { *current = None; }
            }).or_insert(Some(id));
        }
        Title { mask, name, slug }
    }).collect();
    let bundle_records: Vec<serde_json::Value> = serde_json::from_str(include_str!("../assets/catalogue-bundles.json"))
        .expect("valid bundled collection relationships");
    let id = |slug: &str| titles.iter().position(|title| title.slug == slug)
        .expect("bundle references an existing catalogue title");
    let bundles = bundle_records.into_iter().map(|bundle| {
        let members=bundle["includes"].as_array().expect("multipack contents");
        assert!(!members.is_empty(), "a multipack must include titles");
        (id(bundle["slug"].as_str().expect("multipack slug")), members.iter()
            .map(|slug| id(slug.as_str().expect("multipack member slug"))).collect())
    }).collect();
    Catalogue { titles, index, bundles }
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
            crate::title_aliases::slug(&item.title)
                .and_then(|slug| CATALOGUE.titles.iter().position(|title| title.slug == slug))
                .or_else(|| CATALOGUE.index.get(&title_key(&item.title)).copied().flatten())
        }).collect();
        self.masks = owned.iter().map(|id| id.map_or(0, |id| CATALOGUE.titles[id].mask)).collect();
        for (head,members) in crate::multidisc::groups(&items.iter().map(|item|item.path.clone()).collect::<Vec<_>>()) {
            let mask=members.iter().map(|&i|self.masks[i]).find(|&m|m!=0).unwrap_or(self.masks[head]);
            for i in members {self.masks[i]=mask;}
        }
        let mut owned: HashSet<_> = owned.into_iter().flatten().collect();
        // A package is not another disc image. Its contents satisfy the
        // missing-title checklist without adding a synthetic library item.
        for (bundle, members) in &CATALOGUE.bundles {
            if members.iter().all(|member| owned.contains(member)) {
                owned.insert(*bundle);
            }
        }
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

    fn item(title: &str) -> Item {
        Item { path: format!("{title}.chd").into(), title: title.into(), cover: None, disc: None, logo: None }
    }

    fn nfl_pack_missing(collection: &Collection) -> bool {
        (0..collection.missing_count()).any(|i| collection.missing_title(i).unwrap().slug
            == "nfl-the-ultimate-football-collection-triple-pack")
    }

    #[test]
    fn nfl_pack_is_owned_when_all_three_distinct_contents_are_present() {
        let items = vec![item("NFL Football Trivia Challenge '94-'95 Edition (USA)"),
            item("NFL Hall of Fame Football (Europe)"), item("NFL 100 Greatest Touchdowns (USA)")];
        let mut collection=Collection::default();
        collection.refresh(&items);
        assert!(!nfl_pack_missing(&collection));
        assert_eq!(collection.owned_count(),3,"the multipack is not a fourth physical disc");
        collection.category=Category::Games;
        assert!(!nfl_pack_missing(&collection),"a multimedia component must still satisfy the Games multipack");
        for omitted in 0..3 {
            let partial:Vec<_>=items.iter().enumerate().filter(|(i,_)|*i!=omitted)
                .map(|(_,it)|item(&it.title)).collect();
            collection.refresh(&partial);
            assert!(nfl_pack_missing(&collection),"every included title is required");
        }
    }

    #[test]
    fn nfl_pack_does_not_accept_an_older_trivia_edition_or_duplicate_regions() {
        let mut collection=Collection::default();
        collection.refresh(&[item("NFL Football Trivia Challenge (USA)"),
            item("NFL Hall of Fame Football (USA)"),item("NFL 100 Greatest Touchdowns (Europe)")]);
        assert!(nfl_pack_missing(&collection));
        collection.refresh(&[item("NFL Hall of Fame Football (USA)"),
            item("NFL Hall of Fame Football (Europe)"),item("NFL 100 Greatest Touchdowns (Europe)")]);
        assert!(nfl_pack_missing(&collection));
        collection.refresh(&[item("NFL – The Ultimate Football Collection – Triple Pack")]);
        assert!(!nfl_pack_missing(&collection),"direct title matching must still work");
    }

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
