use crate::library::{self, Artwork};

const SIZE: usize = 600;

fn background(pixel: u32, reference: [f32; 3]) -> bool {
    if pixel >> 24 < 24 {
        return true;
    }
    [16, 8, 0]
        .into_iter()
        .zip(reference)
        .all(|(shift, value)| (((pixel >> shift) & 255) as f32 - value).abs() < 38.0)
}

fn median(values: &mut [f32]) -> f32 {
    values.sort_by(f32::total_cmp);
    values[values.len() / 2]
}

pub fn normalize(source: &Artwork) -> Artwork {
    let mut pixels = Vec::with_capacity(SIZE * SIZE);
    for y in 0..SIZE {
        for x in 0..SIZE {
            pixels.push(library::sample_bilinear(
                source,
                (x as f32 + 0.5) * source.w as f32 / SIZE as f32 - 0.5,
                (y as f32 + 0.5) * source.h as f32 / SIZE as f32 - 0.5,
            ));
        }
    }
    let corners = [
        pixels[0],
        pixels[SIZE - 1],
        pixels[(SIZE - 1) * SIZE],
        pixels[SIZE * SIZE - 1],
    ];
    let reference = [16, 8, 0].map(|shift| {
        let mut values = corners.map(|pixel| ((pixel >> shift) & 255) as f32);
        median(&mut values)
    });
    let mut boundary = Vec::new();
    for index in 0..128 {
        let angle = index as f32 * std::f32::consts::TAU / 128.0;
        let (sin, cos) = angle.sin_cos();
        let mut run = 0;
        for radius in (240..325).rev() {
            let x = (299.5 + cos * radius as f32).round() as isize;
            let y = (299.5 + sin * radius as f32).round() as isize;
            let foreground = (0..600).contains(&x)
                && (0..600).contains(&y)
                && !background(pixels[y as usize * SIZE + x as usize], reference);
            run = if foreground { run + 1 } else { 0 };
            if run == 3 {
                let radius = (radius + 2) as f32;
                boundary.push((299.5 + cos * radius, 299.5 + sin * radius));
                break;
            }
        }
    }
    let (mut cx, mut cy) = (299.5, 299.5);
    if boundary.len() >= 64 {
        let (mut left, mut right, mut top, mut bottom) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for &(x, y) in &boundary {
            if (y - 299.5).abs() < 32.0 {
                if x < 299.5 {
                    left.push(x);
                } else {
                    right.push(x);
                }
            }
            if (x - 299.5).abs() < 32.0 {
                if y < 299.5 {
                    top.push(y);
                } else {
                    bottom.push(y);
                }
            }
        }
        if !left.is_empty() && !right.is_empty() {
            cx = ((median(&mut left) + median(&mut right)) / 2.0).clamp(280.0, 320.0);
        }
        if !top.is_empty() && !bottom.is_empty() {
            cy = ((median(&mut top) + median(&mut bottom)) / 2.0).clamp(280.0, 320.0);
        }
    }
    let mut radii: Vec<_> = boundary
        .iter()
        .map(|&(x, y)| (x - cx).hypot(y - cy))
        .collect();
    let outer = if radii.len() >= 64 {
        median(&mut radii).clamp(260.0, 301.0)
    } else {
        299.0
    };
    // The physical hole is small. Do not erase the larger printed or clear hub.
    let mut hole = outer * 0.125;
    let centre_pixel = pixels[cy.round() as usize * SIZE + cx.round() as usize];
    let brightness = |pixel: u32| {
        (((pixel >> 16) & 255) * 3 + ((pixel >> 8) & 255) * 6 + (pixel & 255)) as f32 / 10.0
    };
    if centre_pixel >> 24 < 24 {
        hole = 0.0;
    } else if brightness(centre_pixel) > 180.0 {
        // Find the inner edge of the physical hole, bounded so a white hub
        // or printed label cannot be mistaken for a larger cut-out.
        for radius in (outer * 0.06) as usize..=(outer * 0.20) as usize {
            let mut samples = Vec::with_capacity(32);
            for step in 0..32 {
                let angle = step as f32 * std::f32::consts::TAU / 32.0;
                let (sin, cos) = angle.sin_cos();
                let x = (cx + cos * radius as f32).round() as usize;
                let y = (cy + sin * radius as f32).round() as usize;
                samples.push(brightness(pixels[y * SIZE + x]));
            }
            if brightness(centre_pixel) - median(&mut samples) > 28.0 {
                hole = radius as f32;
                break;
            }
        }
    }
    for y in 0..SIZE {
        for x in 0..SIZE {
            let distance = (x as f32 - cx).hypot(y as f32 - cy);
            let coverage =
                (outer - distance + 0.5).clamp(0.0, 1.0) * (distance - hole + 0.5).clamp(0.0, 1.0);
            let index = y * SIZE + x;
            let alpha = (((pixels[index] >> 24) as f32) * coverage).round() as u32;
            pixels[index] = if alpha == 0 {
                0
            } else {
                (pixels[index] & 0x00ffffff) | (alpha << 24)
            };
        }
    }
    Artwork {
        w: SIZE,
        h: SIZE,
        pixels: pixels.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_background_and_hole_without_removing_white_disc_print() {
        let mut pixels = vec![0xffffffff; 400 * 400];
        for y in 0..400 {
            for x in 0..400 {
                let distance = (x as f32 - 199.5).hypot(y as f32 - 199.5);
                if distance < 194.0 {
                    pixels[y * 400 + x] = 0xff2244bb;
                }
                if (130..160).contains(&x) && (80..150).contains(&y) {
                    pixels[y * 400 + x] = 0xffffffff;
                }
            }
        }
        let original = pixels.clone();
        let source = Artwork {
            w: 400,
            h: 400,
            pixels: pixels.into(),
        };
        let result = normalize(&source);
        assert_eq!((result.w, result.h), (600, 600));
        assert_eq!(result.pixels[0] >> 24, 0);
        assert_eq!(result.pixels[300 * 600 + 300] >> 24, 0);
        assert_eq!(result.pixels[150 * 600 + 210], 0xffffffff);
        assert_eq!(source.pixels.as_ref(), original.as_slice());
    }
}
