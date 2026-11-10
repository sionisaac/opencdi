use crate::{launchbox::ArtworkKind, library, synopsis};
use std::fmt::Write;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static REPORT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub fn refresh(
    destination: &Path,
    discs: &[PathBuf],
    artwork: &Path,
    synopses: &Path,
    lookup_running: bool,
) -> std::io::Result<()> {
    let mut entries = String::new();
    let mut incomplete = 0;
    let mut missing_count = 0;
    for disc in discs {
        let mut missing = Vec::new();
        for (kind, label) in [
            (ArtworkKind::Cover, "Front cover"),
            (ArtworkKind::Disc, "Disc image"),
            (ArtworkKind::Boxart, "3D box art"),
            (ArtworkKind::Back, "Back cover"),
            (ArtworkKind::Logo, "Clear logo"),
        ] {
            let managed = match kind {
                ArtworkKind::Cover | ArtworkKind::Disc => library::has_disc_image(
                    disc,
                    &artwork.join(kind.folder()),
                    kind == ArtworkKind::Cover,
                ),
                _ => library::managed_art_path(disc, artwork, kind).is_some(),
            };
            if !managed {
                missing.push(label);
            }
        }
        if synopsis::load(synopses, disc).is_none() {
            missing.push("Synopsis");
        }
        if missing.is_empty() {
            continue;
        }
        incomplete += 1;
        missing_count += missing.len();
        let name = disc
            .file_name()
            .unwrap_or(disc.as_os_str())
            .to_string_lossy();
        let _ = writeln!(entries, "\n{name}");
        for label in missing {
            let _ = writeln!(entries, "  - {label}");
        }
    }
    let status = if lookup_running {
        "Artwork and synopsis lookups are in progress."
    } else if incomplete == 0 {
        "No missing media found."
    } else {
        "Listed items are currently missing from local storage."
    };
    let body = format!(
        "Library: {} discs checked\nMissing media: {incomplete} discs, {missing_count} items\n{status}\n{entries}",
        discs.len()
    );
    if std::fs::read_to_string(destination).is_ok_and(|text| text.ends_with(&body)) {
        return Ok(());
    }
    let contents = format!(
        "OpenCDi — Missing Media\nLast updated: {}\n{body}",
        timestamp()
    );
    let sequence = REPORT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temp = destination.with_extension(format!("txt.{}.{sequence}.tmp", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    let result = file.write_all(contents.as_bytes());
    drop(file);
    if let Err(error) = result.and_then(|()| std::fs::rename(&temp, destination)) {
        let _ = std::fs::remove_file(&temp);
        return Err(error);
    }
    Ok(())
}

#[cfg(windows)]
fn timestamp() -> String {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetLocalTime(time: *mut [u16; 8]);
    }
    let mut time = [0; 8];
    unsafe {
        GetLocalTime(&mut time);
    }
    let [year, month, _, day, hour, minute, _, _] = time;
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02} (local time)")
}

#[cfg(not(windows))]
fn timestamp() -> String {
    let seconds = std::time::UNIX_EPOCH
        .elapsed()
        .unwrap_or_default()
        .as_secs();
    format!("{seconds} seconds since 1970-01-01 UTC")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn refresh_groups_missing_media_and_removes_arriving_items() {
        let dir =
            std::env::temp_dir().join(format!("opencdi-missing-media-{}", std::process::id()));
        let disc = dir.join("Shadoan (EUR).chd");
        let report = dir.join("Missing Media.txt");
        fs::create_dir_all(&dir).unwrap();
        let write_report = |running| {
            refresh(
                &report,
                std::slice::from_ref(&disc),
                &dir,
                &dir,
                running,
            )
            .unwrap();
            fs::read_to_string(&report).unwrap()
        };

        let first = write_report(true);
        assert!(first.contains("1 discs, 6 items"));
        assert!(first.contains("in progress"));
        assert!(first.contains("Shadoan (EUR).chd\n  - Front cover"));
        let synopsis_path = synopsis::path(&dir, &disc).unwrap();
        synopsis::save(&synopsis_path, "Test synopsis.");
        let second = write_report(false);
        assert!(second.contains("1 discs, 5 items"));
        assert!(!second.contains("- Synopsis"));
        assert!(!second.contains("in progress"));

        fs::remove_file(synopsis_path).unwrap();
        fs::remove_file(report).unwrap();
        fs::remove_dir(dir).unwrap();
    }
}
