use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub fn path(root: &Path, disc: &Path) -> Option<PathBuf> {
    let mut name = disc.file_stem()?.to_os_string();
    name.push(".txt");
    Some(root.join(name))
}

pub fn load(root: &Path, disc: &Path) -> Option<String> {
    let mut text = String::new();
    fs::File::open(path(root, disc)?).ok()?.take(64 * 1024).read_to_string(&mut text).ok()?;
    (!text.trim().is_empty()).then_some(text)
}

pub fn save(path: &Path, text: &str) {
    let text = text.trim();
    if text.is_empty() || text.len() > 64 * 1024 { return; }
    publish_new(path, format!("{text}\n").as_bytes());
}

pub(crate) fn publish_new(path: &Path, contents: &[u8]) -> bool {
    if path.exists() { return false; }
    let Some(dir) = path.parent() else { return false; };
    if fs::create_dir_all(dir).is_err() { return false; }
    let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("tmp");
    let tmp = path.with_extension(format!("{extension}.{}.download", std::process::id()));
    let Ok(mut file) = fs::OpenOptions::new().write(true).create_new(true).open(&tmp) else { return false; };
    let written = file.write_all(contents).is_ok();
    drop(file);
    let published = written && (fs::hard_link(&tmp, path).is_ok() || {
        // Some portable filesystems lack hard links.
        if let Ok(mut destination) = fs::OpenOptions::new().write(true).create_new(true).open(path) {
            let ok = destination.write_all(contents).is_ok();
            drop(destination);
            if !ok { let _ = fs::remove_file(path); }
            ok
        } else {
            false
        }
    });
    let _ = fs::remove_file(tmp);
    published
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_exact_disc_filename_and_existing_synopsis() {
        let dir = std::env::temp_dir().join(format!("opencdi-synopsis-{}", std::process::id()));
        let disc = Path::new("National Parks Tour, A (USA).chd");
        let target = path(&dir, disc).unwrap();
        assert_eq!(target.file_name().unwrap(), "National Parks Tour, A (USA).txt");
        save(&target, "A tour of Yosemite.");
        save(&target, "Do not replace the original.");
        assert_eq!(load(&dir, disc).as_deref(), Some("A tour of Yosemite.\n"));
        fs::remove_file(target).unwrap();
        fs::remove_dir(dir).unwrap();
    }
}
