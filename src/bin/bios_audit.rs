//! Usage: bios_audit <rom>... . Prints metadata only; never exports module bytes.
use opencdi::{bios, bios_audit};

fn main() {
    let paths: Vec<_> = std::env::args_os().skip(1).collect();
    if paths.is_empty() {
        eprintln!("usage: bios_audit <system-or-cartridge.rom>...");
        std::process::exit(2);
    }
    let mut failed = false;
    for path in paths {
        let mut data = match std::fs::read(&path) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("{}: {e}", path.to_string_lossy());
                failed = true;
                continue;
            }
        };
        let file_sha1 = bios::sha1_hex(&data);
        let (model, swapped) =
            bios::identify(&data).map_or(("unknown", false), |(r, s)| (r.model, s));
        if swapped {
            bios::swap_words(&mut data);
        }
        let modules = bios_audit::modules(&data);
        println!(
            "IMAGE name={} model={} size={} word_swapped={} file_sha1={} cpu_sha1={}",
            path.to_string_lossy(),
            model,
            data.len(),
            swapped,
            file_sha1,
            bios::sha1_hex(&data)
        );
        println!("offset\tsize\ttype\tlanguage\tattributes\trevision\tedition\tcrc\tsha1\tname");
        for m in &modules {
            println!(
                "{:08x}\t{}\t{:02x}\t{:02x}\t{:02x}\t{}\t{}\t{}\t{}\t{}",
                m.offset,
                m.size,
                m.module_type,
                m.language,
                m.attributes,
                m.revision,
                m.edition,
                if m.crc_ok { "ok" } else { "BAD" },
                m.sha1,
                m.name
            );
        }
        let bad = modules.iter().filter(|m| !m.crc_ok).count();
        println!("SUMMARY modules={} crc_failures={bad}", modules.len());
        failed |= modules.is_empty() || bad != 0;
    }
    if failed {
        std::process::exit(1);
    }
}
