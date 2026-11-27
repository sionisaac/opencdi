//! Conservative disc-set identities. Region, language, revision and directory
//! remain part of the key; numbered volumes and sequels are never inferred.
use std::path::{Path, PathBuf};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Disc {
    pub number: u32,
    pub title: String,
    pub label: String,
    pub key: String,
}

pub fn identify(path: &Path) -> Option<Disc> {
    let stem = path.file_stem()?.to_str()?;
    let lower = stem.to_ascii_lowercase();
    for (start, _) in lower.match_indices('(') {
        let end = start + lower[start..].find(')')?;
        let tag = lower[start+1..end].trim();
        let Some(rest) = tag.strip_prefix("disc ").or_else(|| tag.strip_prefix("disk ")) else {continue;};
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        let Ok(number) = digits.parse::<u32>() else { continue; };
        if !(1..=99).contains(&number) { continue; }
        let suffix = rest[digits.len()..].trim();
        if !suffix.is_empty() && !suffix.starts_with("of ") { continue; }
        let mut tail = stem[end+1..].trim();
        let mut retained=Vec::new();
        let mut label = String::new();
        // Disc-content labels are not release/region identifiers.
        while matches!(tail.chars().next(),Some('('|'[')) {
            let closing=if tail.starts_with('('){')'}else{']'};
            let Some(close)=tail.find(closing) else {break;};
            let candidate = &tail[1..close];
            if matches!(candidate.to_ascii_lowercase().as_str(), "the game" | "the music" | "making of" | "the making of" | "bonus disc" | "bonus"
                | "the documentation" | "documentation" | "the episode" | "episode" | "das spiel" | "le jeu" | "die dokumentation") {
                label = candidate.to_owned();
            }else{retained.push(tail[..close+1].to_owned());}
            tail=tail[close+1..].trim();
        }
        if !tail.is_empty(){retained.push(tail.to_owned());}
        let tail=retained.join(" ");
        let title = format!("{}{}{}", stem[..start].trim(), if tail.is_empty(){""}else{" "}, tail);
        let parent = path.parent().unwrap_or(Path::new(""));
        let key = format!("{}|{}",parent.to_string_lossy().to_lowercase(),title.to_lowercase());
        return Some(Disc {number,title,label,key});
    }
    None
}

pub fn groups(paths: &[PathBuf]) -> HashMap<usize, Vec<usize>> {
    let mut sets: HashMap<String, Vec<(u32,usize)>> = HashMap::new();
    for (i,path) in paths.iter().enumerate() {
        if let Some(disc)=identify(path) { sets.entry(disc.key).or_default().push((disc.number,i)); }
    }
    let mut result=HashMap::new();
    for mut set in sets.into_values() {
        set.sort_unstable();
        if set.len()<2 || set.windows(2).any(|pair|pair[0].0==pair[1].0) {continue;}
        result.insert(set[0].1,set.into_iter().map(|(_,i)|i).collect());
    }
    result
}

pub fn members(path: &Path, paths: &[PathBuf]) -> Vec<PathBuf> {
    let Some(disc)=identify(path) else {return vec![path.to_owned()]};
    let mut entries:Vec<_>=paths.iter().filter_map(|p|{
        let other=identify(p)?;(disc.key==other.key).then_some((other.number,p.clone()))
    }).collect();
    entries.sort();
    if entries.len()<2 || entries.windows(2).any(|pair|pair[0].0==pair[1].0) {return vec![path.to_owned()];}
    entries.into_iter().map(|(_,p)|p).collect()
}

pub fn first_disc(path:&Path)->Option<PathBuf> {
    let disc=identify(path)?;
    if disc.number==1 {return Some(path.to_owned());}
    std::fs::read_dir(path.parent()?).ok()?.filter_map(Result::ok).map(|e|e.path())
        .filter(|p|p.extension().map(|e|e.to_string_lossy().to_ascii_lowercase())==path.extension().map(|e|e.to_string_lossy().to_ascii_lowercase()))
        .find(|p|identify(p).is_some_and(|d|d.key==disc.key && d.number==1))
}

pub fn discover(path:&Path)->Vec<PathBuf>{
    let Some(parent)=path.parent() else{return vec![path.to_owned()];};
    let Ok(dir)=std::fs::read_dir(parent) else{return vec![path.to_owned()];};
    let paths=dir.filter_map(Result::ok).take(4096).map(|entry|entry.path())
        .filter(|p|p.extension().map(|e|e.to_ascii_lowercase())==path.extension().map(|e|e.to_ascii_lowercase())).collect::<Vec<_>>();
    members(path,&paths)
}

#[cfg(test)]
mod tests {
 use super::*;
 #[test] fn seventh_guest_game_and_music_share_a_set_without_losing_disc_labels(){
  let paths:Vec<PathBuf>=["Discs/7th Guest, The (USA) (Disc 1) (The Game).chd",
   "Discs/7th Guest, The (USA) (Disc 2) (The Music).chd",
   "Discs/7th Guest, The (Europe) (Disc 2) (The Music).chd"].map(PathBuf::from).to_vec();
  let first=identify(&paths[0]).unwrap();let second=identify(&paths[1]).unwrap();
  assert_eq!(first.key,second.key);
  assert_eq!(first.title,"7th Guest, The (USA)");
  assert_eq!(second.title,first.title);
  assert_eq!((first.label.as_str(),second.label.as_str()),("The Game","The Music"));
  assert_eq!(groups(&paths)[&0],vec![0,1]);
  assert_eq!(members(&paths[1],&paths),paths[..2]);
  assert_ne!(second.key,identify(&paths[2]).unwrap().key);
 }
 #[test] fn grouping_preserves_releases_and_rejects_ambiguous_discs(){
  let paths:Vec<PathBuf>=["7th Guest, The (Europe) (Disc 1) (The Game).chd","7th Guest, The (Europe) (Disc 2) (The Making Of).chd","7th Guest, The (USA) (Disc 1).chd","7th Guest, The (USA) (Disc 2).chd","Annual 1997.chd","Annual 1998.chd"].map(PathBuf::from).to_vec();
  let g=groups(&paths);assert_eq!(g.len(),2);assert_eq!(g[&0],vec![0,1]);assert_eq!(g[&2],vec![2,3]);
  assert_eq!(identify(&paths[0]).unwrap().title,"7th Guest, The (Europe)");
  let mut duplicate=paths.clone();duplicate.push(paths[1].clone());assert!(!groups(&duplicate).contains_key(&0));
 }
 #[test] fn tosec_disc_counts_and_revisions_remain_distinct(){
  let a=identify(Path::new("Title (1994)(Philips)(EU)(Disc 1 of 2).chd")).unwrap();
  let b=identify(Path::new("Title (1994)(Philips)(EU)(Disc 2 of 2).chd")).unwrap();
  assert_eq!(a.key,b.key);assert_eq!(b.number,2);
  assert!(identify(Path::new("Title 2.chd")).is_none());
  assert_ne!(a.key,identify(Path::new("Title (1994)(Philips)(US)(Disc 1 of 2).chd")).unwrap().key);
  assert_eq!(identify(Path::new("Marco Polo (Europe) (Disc 1) (The Game).chd")).unwrap().key,
    identify(Path::new("Marco Polo (Europe) (Disc 2) (The Documentation).chd")).unwrap().key);
  assert_eq!(identify(Path::new("Title (1994)(Philips)(DE)(Disc 1 of 2)[!][Das Spiel][DVC].chd")).unwrap().key,
    identify(Path::new("Title (1994)(Philips)(DE)(Disc 2 of 2)[!][Die Dokumentation][DVC].chd")).unwrap().key);
 }
}
