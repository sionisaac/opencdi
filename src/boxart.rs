use crate::{
    launchbox::ArtworkKind,
    library::{self, Artwork},
};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    LazyLock,
    atomic::{AtomicU64, Ordering},
};

const SCALE: usize = 2;
const WIDTH: usize = 400 * SCALE;
const HEIGHT: usize = 459 * SCALE;
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
static TEMPLATE: LazyLock<Artwork> = LazyLock::new(|| {
    library::load_png_bytes(include_bytes!("../assets/boxart_case.png"))
        .expect("embedded case template")
});

pub fn generated_path(root: &Path, title: &str) -> PathBuf {
    root.join("Boxart").join(format!(
        "{}--generated.png",
        crate::launchbox::safe_name(title)
    ))
}

fn encode(image: &Artwork, signature: Option<&str>) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, image.w as u32, image.h as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        if let Some(signature) = signature {
            encoder
                .add_text_chunk("OpenCDiCover".into(), signature.into())
                .map_err(io::Error::other)?;
        }
        let data: Vec<_> = image
            .pixels
            .iter()
            .flat_map(|pixel| {
                [
                    (pixel >> 16) as u8,
                    (pixel >> 8) as u8,
                    *pixel as u8,
                    (pixel >> 24) as u8,
                ]
            })
            .collect();
        encoder
            .write_header()
            .map_err(io::Error::other)?
            .write_image_data(&data)
            .map_err(io::Error::other)?;
    }
    Ok(bytes)
}

fn save(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("Missing media folder"))?;
    fs::create_dir_all(parent)?;
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = path.with_extension(format!("{}.{}.tmp", std::process::id(), sequence));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let result = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    let result = result.and_then(|()| fs::rename(&temporary, path));
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

fn resize_cover(image: &Artwork) -> Artwork {
    let mut pixels = Vec::with_capacity(600 * 600);
    for y in 0..600 {
        for x in 0..600 {
            pixels.push(library::sample_bilinear(
                image,
                (x as f32 + 0.5) * image.w as f32 / 600.0 - 0.5,
                (y as f32 + 0.5) * image.h as f32 / 600.0 - 0.5,
            ));
        }
    }
    Artwork {
        w: 600,
        h: 600,
        pixels: pixels.into(),
    }
}

// Decode JPEGs with the same Windows imaging support used by the library.
pub fn publish_media(kind: ArtworkKind, path: &Path, bytes: &[u8]) -> bool {
    if kind == ArtworkKind::Boxart {
        return false;
    }
    if !matches!(kind, ArtworkKind::Cover | ArtworkKind::Disc) {
        return crate::synopsis::publish_new(path, bytes);
    }
    if path.exists() {
        return false;
    }
    let Some(parent) = path.parent() else {
        return false;
    };
    if fs::create_dir_all(parent).is_err() {
        return false;
    }
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = path.with_extension(format!("{}.{}.source", std::process::id(), sequence));
    let image = library::load_png_bytes(bytes).or_else(|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .ok()?;
        file.write_all(bytes).ok()?;
        drop(file);
        library::load_artwork(&temporary)
    });
    let _ = fs::remove_file(temporary);
    image
        .and_then(|image| {
            let prepared = if kind == ArtworkKind::Disc { crate::disc_art::normalize(&image) }
                else { resize_cover(&image) };
            encode(&prepared, None).ok()
        })
        .is_some_and(|bytes| crate::synopsis::publish_new(path, &bytes))
}

pub fn ensure(
    cover: &Path,
    title: &str,
    root: &Path,
) -> io::Result<Option<PathBuf>> {
    let image =
        library::load_artwork(cover).ok_or_else(|| io::Error::other("Cannot read cover"))?;
    let mut hash = 0xcbf29ce484222325u64;
    for pixel in image.pixels.iter() {
        for byte in pixel.to_le_bytes() {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
        }
    }
    let signature = format!("case-v1-{:x}-{}x{}", hash, image.w, image.h);
    let destination = generated_path(root, title);
    if let Ok(file) = fs::File::open(&destination) {
        let decoder = png::Decoder::new(std::io::BufReader::new(file));
        if let Ok(reader) = decoder.read_info() {
            if reader
                .info()
                .uncompressed_latin1_text
                .iter()
                .any(|chunk| chunk.keyword == "OpenCDiCover" && chunk.text == signature)
            {
                return Ok(None);
            }
        }
    }
    let rendered = render(&TEMPLATE, &image);
    let mut pixels = vec![0; (WIDTH + 32) * (HEIGHT + 32)];
    for y in 0..HEIGHT {
        let start = (y + 16) * (WIDTH + 32) + 16;
        pixels[start..start + WIDTH].copy_from_slice(&rendered[y * WIDTH..(y + 1) * WIDTH]);
    }
    let generated = Artwork {
        w: WIDTH + 32,
        h: HEIGHT + 32,
        pixels: pixels.into(),
    };
    save(&destination, &encode(&generated, Some(&signature))?)?;
    Ok(Some(destination))
}

fn render(template: &Artwork, cover: &Artwork) -> Vec<u32> {
    let mut pixels = vec![0; WIDTH * HEIGHT];
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let px = x as f32 / SCALE as f32;
            let py = y as f32 / SCALE as f32;
            let top = if px < 48.0 {
                6.0 - px / 8.0
            } else {
                (px - 48.0) * 23.0 / 351.0
            };
            let bottom = if px < 48.0 {
                445.0 + px * 14.0 / 48.0
            } else {
                459.0 - (px - 48.0) * 15.0 / 351.0
            };
            if py < top || py > bottom {
                continue;
            }
            let mut color = crate::library::sample_bilinear(template, px, py);
            if (50.0..396.0).contains(&px) {
                let u = (px - 50.0) / 346.0;
                let front_top = 13.0 + 15.0 * u;
                let front_bottom = 445.0 - 10.0 * u;
                if py >= front_top && py <= front_bottom {
                    let v = (py - front_top) / (front_bottom - front_top);
                    color = crate::library::sample_bilinear(
                        cover,
                        u * (cover.w - 1) as f32,
                        v * (cover.h - 1) as f32,
                    ) | 0xff000000;
                }
            }
            if px >= 48.0 {
                let u = ((px - 50.0) / 346.0).clamp(0.0, 1.0);
                if py < 13.0 + 15.0 * u || py > 445.0 - 10.0 * u {
                    let light =
                        (((color >> 16) & 255) * 3 + ((color >> 8) & 255) * 6 + (color & 255)) / 10;
                    let silver = light.max(130);
                    color = 0xff000000 | silver << 16 | silver << 8 | silver;
                }
            }
            if px < 11.0 {
                color = 0xff0b0b0b;
            }
            pixels[y * WIDTH + x] = color;
        }
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downloaded_covers_are_square_and_generated_cases_refresh_when_cover_changes() {
        let root =
            std::env::temp_dir().join(format!("opencdi-generated-boxart-{}", std::process::id()));
        let cover = root.join("Covers/Example.png");
        let original = Artwork {
            w: 32,
            h: 16,
            pixels: vec![0xff336699; 32 * 16].into(),
        };
        assert!(publish_media(
            ArtworkKind::Cover,
            &cover,
            &encode(&original, None).unwrap()
        ));
        let normalized = library::load_artwork(&cover).unwrap();
        assert_eq!((normalized.w, normalized.h), (600, 600));

        let case = ensure(&cover, "Example", &root).unwrap().unwrap();
        let first = fs::read(&case).unwrap();
        let image = library::load_artwork(&case).unwrap();
        assert_eq!((image.w, image.h), (832, 950));
        assert_eq!(image.pixels[0] >> 24, 0);
        assert_eq!(image.pixels[400 * image.w + 400], 0xff336699);
        assert_eq!(ensure(&cover, "Example", &root).unwrap(), None);
        assert_eq!(fs::read(&case).unwrap(), first);

        let changed = Artwork {
            w: 600,
            h: 600,
            pixels: vec![0xffbb5500; 600 * 600].into(),
        };
        save(&cover, &encode(&changed, None).unwrap()).unwrap();
        assert!(ensure(&cover, "Example", &root).unwrap().is_some());
        assert_ne!(fs::read(&case).unwrap(), first);
        assert!(!publish_media(
            ArtworkKind::Boxart,
            &root.join("downloaded.png"),
            b"blocked"
        ));

        fs::remove_file(case).unwrap();
        fs::remove_file(cover).unwrap();
        fs::remove_dir(root.join("Boxart")).unwrap();
        fs::remove_dir(root.join("Covers")).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
