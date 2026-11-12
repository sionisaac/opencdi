//! Hash bounded, contiguous Form-1 root files without exporting their contents.
use opencdi::{bios::sha1_hex, cdrom::CdRom};

fn word(d: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(d[at..at + 4].try_into().unwrap())
}
fn read(cd: &mut CdRom, lba: u32, size: usize) -> Option<Vec<u8>> {
    if size > 1024 * 1024 {
        return None;
    }
    let mut out = Vec::with_capacity(size);
    let mut block = [0; 2048];
    for i in 0..size.div_ceil(2048) {
        if !cd.read_data_sector(lba.checked_add(i as u32)?, &mut block) {
            return None;
        }
        out.extend_from_slice(&block[..(size - out.len()).min(2048)]);
    }
    Some(out)
}
fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let slices = args.get(1).is_some_and(|s| s == "--contract-slices");
    if args.is_empty() || args.len() > 2 || (args.len() == 2 && !slices) {
        eprintln!("usage: disc_data_audit <disc.chd|cue|iso> [--contract-slices]");
        std::process::exit(2)
    }
    let mut cd = CdRom::open(&args[0]).expect("open disc");
    let label = read(&mut cd, 16, 2048).expect("read label");
    println!(
        "LABEL\t{}\t{}\t{}\t{}",
        cd.volume_id,
        cd.volume_set,
        cd.publisher,
        sha1_hex(&label)
    );
    let root_lba = if &label[1..6] == b"CD-I " {
        let table = word(&label, 148);
        let block = read(&mut cd, table, 2048).expect("read path table");
        word(&block, 2)
    } else if &label[1..6] == b"CD001" {
        word(&label, 162)
    } else {
        panic!("unrecognized volume descriptor")
    };
    let first = read(&mut cd, root_lba, 2048).expect("read root header");
    let size = word(&first, 14) as usize;
    assert!(
        (34..=65536).contains(&size),
        "bounded root directory required"
    );
    let root = read(&mut cd, root_lba, size).expect("read root directory");
    let mut offset = 0;
    let mut files = 0;
    let mut skipped = 0;
    while offset + 34 <= root.len() {
        let n = root[offset] as usize;
        if n == 0 {
            offset = (offset / 2048 + 1) * 2048;
            continue;
        }
        assert!(
            n >= 34 && offset + n <= root.len() && offset % 2048 + n <= 2048,
            "invalid directory record"
        );
        let d = &root[offset..offset + n];
        let len = d[32] as usize;
        assert!(33 + len <= n, "invalid name extent");
        let name = &d[33..33 + len];
        if name.iter().all(|c| (0x20..0x7f).contains(c)) && len > 0 {
            let count = word(d, 14) as usize;
            let lba = word(d, 6)
                .checked_add(d[1] as u32)
                .expect("extent overflow");
            if d[26] == 0 && d[27] == 0 {
                if let Some(data) = read(&mut cd, lba, count) {
                    println!(
                        "FILE\t{}\t{}\t{}",
                        String::from_utf8_lossy(name),
                        count,
                        sha1_hex(&data)
                    );
                    files += 1;
                    if slices {
                        for (start, requested) in [
                            (0, count),
                            (0, 5),
                            (5, 5),
                            (7, 13),
                            (2040, 32),
                            (count.saturating_sub(8), 32),
                            (1, 5),
                        ] {
                            if start < count {
                                let end = start.saturating_add(requested).min(count);
                                println!(
                                    "SLICE\t{}\t{start}\t{}\t{}",
                                    String::from_utf8_lossy(name),
                                    end - start,
                                    sha1_hex(&data[start..end])
                                );
                            }
                        }
                    }
                } else {
                    skipped += 1
                }
            } else {
                skipped += 1
            }
        }
        offset += n;
    }
    println!("SUMMARY\tfiles={files}\tskipped={skipped}");
    assert!(files > 0, "no eligible root files");
}
