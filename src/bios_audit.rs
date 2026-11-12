//! Read-only OS-9/68000 module inventory. No firmware bytes are extracted.
//! Offsets refer to a caller-supplied image in CPU byte order, not runtime addresses.

const HEADER: usize = 48;
pub const CRC_RESIDUE: u32 = 0x800fe3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module {
    pub offset: usize,
    pub size: usize,
    pub name: String,
    pub module_type: u8,
    pub language: u8,
    pub attributes: u8,
    pub revision: u8,
    pub edition: u16,
    pub crc_ok: bool,
    pub sha1: String,
}

/// OS-9's 24-bit, MSB-first CRC accumulator (no final complement).
pub fn crc24(data: &[u8], initial: u32) -> u32 {
    let mut crc = initial & 0xffffff;
    for &byte in data {
        crc ^= (byte as u32) << 16;
        for _ in 0..8 {
            crc = ((crc << 1) ^ if crc & 0x800000 != 0 { 0x800063 } else { 0 }) & 0xffffff;
        }
    }
    crc
}

fn long(data: &[u8], at: usize) -> usize {
    u32::from_be_bytes(data[at..at + 4].try_into().unwrap()) as usize
}

/// Find bounded, named modules with valid 48-byte header parity. CRC failures
/// are retained in the inventory rather than silently hiding damaged modules.
/// This is a structural scan, not proof a module is loaded or executable.
pub fn modules(image: &[u8]) -> Vec<Module> {
    let mut result = Vec::new();
    for offset in (0..image.len().saturating_sub(HEADER - 1)).step_by(2) {
        let header = &image[offset..offset + HEADER];
        if header[..2] != [0x4a, 0xfc]
            || header
                .chunks_exact(2)
                .fold(0u16, |p, w| p ^ u16::from_be_bytes([w[0], w[1]]))
                != 0xffff
        {
            continue;
        }
        let size = long(header, 4);
        let Some(end) = offset.checked_add(size).filter(|&end| end <= image.len()) else {
            continue;
        };
        let name_at = long(header, 12);
        if size < HEADER + 3 || name_at < HEADER || name_at >= size - 3 {
            continue;
        }
        let mut name = String::new();
        let mut terminated = false;
        for &b in image[offset + name_at..end - 3].iter().take(128) {
            if b == 0 {
                terminated = true;
                break;
            }
            let c = b & 0x7f;
            if !(0x20..0x7f).contains(&c) {
                break;
            }
            name.push(c as char);
            if b & 0x80 != 0 {
                terminated = true;
                break;
            }
        }
        if name.is_empty() || !terminated {
            continue;
        }
        let bytes = &image[offset..end];
        result.push(Module {
            offset,
            size,
            name,
            module_type: header[18],
            language: header[19],
            attributes: header[20],
            revision: header[21],
            edition: u16::from_be_bytes([header[22], header[23]]),
            crc_ok: crc24(bytes, 0xffffff) == CRC_RESIDUE,
            sha1: crate::bios::sha1_hex(bytes),
        });
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<u8> {
        // Original synthetic data module; no commercial firmware test fixture.
        let mut d = vec![0; 64];
        d[..2].copy_from_slice(&[0x4a, 0xfc]);
        d[4..8].copy_from_slice(&64u32.to_be_bytes());
        d[12..16].copy_from_slice(&48u32.to_be_bytes());
        d[18] = 4;
        d[22..24].copy_from_slice(&257u16.to_be_bytes());
        d[48..54].copy_from_slice(b"probe\0");
        let p = d[..46]
            .chunks_exact(2)
            .fold(0xffffu16, |p, w| p ^ u16::from_be_bytes([w[0], w[1]]));
        d[46..48].copy_from_slice(&p.to_be_bytes());
        let crc = !crc24(&d[..61], 0xffffff);
        d[61..].copy_from_slice(&crc.to_be_bytes()[1..]);
        d
    }

    #[test]
    fn finds_bounded_module_and_checks_crc() {
        let d = fixture();
        let m = modules(&d);
        assert_eq!(m.len(), 1);
        assert_eq!(
            (&*m[0].name, m[0].edition, m[0].crc_ok),
            ("probe", 257, true)
        );
        let mut damaged = d;
        damaged[55] ^= 1;
        assert!(!modules(&damaged)[0].crc_ok);
    }

    #[test]
    fn rejects_bad_parity_size_names_and_truncation() {
        let good = fixture();
        for len in 0..good.len() {
            assert!(modules(&good[..len]).is_empty());
        }
        for (at, value) in [(2, 1), (4, 0xff), (12, 0xff), (48, 1)] {
            let mut bad = good.clone();
            bad[at] = value;
            assert!(modules(&bad).is_empty());
        }
        assert!(modules(&[]).is_empty());
    }

    #[test]
    fn supports_high_bit_names_offsets_and_streaming_crc() {
        let mut d = fixture();
        d[52] |= 0x80;
        let mut image = vec![0; 10];
        image.extend_from_slice(&d);
        let m = modules(&image);
        assert_eq!((m[0].offset, &*m[0].name), (10, "probe"));
        let a = crc24(&d[..29], 0xffffff);
        assert_eq!(crc24(&d[29..], a), crc24(&d, 0xffffff));
    }

    #[test]
    fn crc_matches_observed_cdi220b_service_contract() {
        // Independently exercised via TRAP #0/F$CRC in bios_contract, not a
        // CRC generated from the implementation under test.
        assert_eq!(crc24(b"123456789", 0xffffff), 0xdff05a);
        assert_eq!(crc24(b"123456789", 0), 0x23ef52);
        assert_eq!(crc24(b"123456789", 0x123456), 0xc6ef89);
    }
}
