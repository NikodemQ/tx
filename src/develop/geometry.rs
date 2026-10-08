//! Turning, straightening and cropping. Crop rectangles are `[x0, y0, x1, y1]` as shares of the
//! picture after it is turned and straightened.

use rayon::prelude::*;

use super::Image;

pub type Rect = [f32; 4];

pub const FULL: Rect = [0.0, 0.0, 1.0, 1.0];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Aspect {
    #[default]
    Free,
    Original,
    Ratio(u8, u8),
}

pub const ASPECTS: [Aspect; 6] = [
    Aspect::Free,
    Aspect::Original,
    Aspect::Ratio(1, 1),
    Aspect::Ratio(4, 3),
    Aspect::Ratio(3, 2),
    Aspect::Ratio(16, 9),
];

/// Width over height of a crop with this aspect, turned to match a portrait picture. `None` is free.
pub fn aspect_ratio(aspect: Aspect, w: usize, h: usize) -> Option<f32> {
    let r = match aspect {
        Aspect::Free => return None,
        Aspect::Original => return Some(w as f32 / h as f32),
        Aspect::Ratio(a, b) => f32::from(a) / f32::from(b),
    };
    Some(if h > w { 1.0 / r } else { r })
}

/// A quarter turn clockwise.
pub fn rotate_cw(img: &Image) -> Image {
    let (w, h) = (img.width, img.height);
    let mut pixels = vec![[0.0; 3]; w * h];
    pixels.par_chunks_mut(h).enumerate().for_each(|(y, row)| {
        for (x, p) in row.iter_mut().enumerate() {
            *p = img.pixels[(h - 1 - x) * w + y];
        }
    });
    Image::new(h, w, pixels)
}

/// Turns the picture by `angle` degrees, counter-clockwise for positive ones, and keeps the largest
/// rectangle of the same shape that has no corners from outside the picture.
pub fn straighten(img: Image, angle: f32) -> Image {
    if angle == 0.0 {
        return img;
    }
    let (w, h) = (img.width, img.height);
    let (wf, hf) = (w as f32, h as f32);
    let (cw, ch) = straightened_size(w, h, angle);
    let (x0, y0) = ((w - cw) / 2, (h - ch) / 2);
    let (sin, cos) = angle.to_radians().sin_cos();
    let (cx, cy) = ((wf - 1.0) / 2.0, (hf - 1.0) / 2.0);
    let mut pixels = vec![[0.0; 3]; cw * ch];
    pixels.par_chunks_mut(cw).enumerate().for_each(|(y, row)| {
        for (x, p) in row.iter_mut().enumerate() {
            let (dx, dy) = ((x0 + x) as f32 - cx, (y0 + y) as f32 - cy);
            *p = bilinear(&img, cx + cos * dx - sin * dy, cy + sin * dx + cos * dy);
        }
    });
    Image::new(cw, ch, pixels)
}

/// The size a `w` by `h` picture has after [`straighten`].
pub fn straightened_size(w: usize, h: usize, angle: f32) -> (usize, usize) {
    if angle == 0.0 {
        return (w, h);
    }
    let (s, c) = angle.abs().to_radians().sin_cos();
    let (wf, hf) = (w as f32, h as f32);
    let f = (wf / (wf * c + hf * s)).min(hf / (wf * s + hf * c));
    (((wf * f) as usize).max(1), ((hf * f) as usize).max(1))
}

/// The colour at a point between pixels, taking the nearest edge pixel outside the picture.
fn bilinear(img: &Image, x: f32, y: f32) -> [f32; 3] {
    let x = x.clamp(0.0, (img.width - 1) as f32);
    let y = y.clamp(0.0, (img.height - 1) as f32);
    let (xa, ya) = (x.floor() as usize, y.floor() as usize);
    let (xb, yb) = ((xa + 1).min(img.width - 1), (ya + 1).min(img.height - 1));
    let (tx, ty) = (x - xa as f32, y - ya as f32);
    let at = |x: usize, y: usize| img.pixels[y * img.width + x];
    let (a, b, c, d) = (at(xa, ya), at(xb, ya), at(xa, yb), at(xb, yb));
    std::array::from_fn(|i| {
        let top = a[i] + (b[i] - a[i]) * tx;
        let bottom = c[i] + (d[i] - c[i]) * tx;
        top + (bottom - top) * ty
    })
}

pub fn crop(img: Image, rect: Rect) -> Image {
    if rect == FULL {
        return img;
    }
    let (w, h) = (img.width, img.height);
    let [x0, y0, x1, y1] = rect;
    let at = |share: f32, size: usize| (share * size as f32).round_ties_even().max(0.0) as usize;
    let (xa, ya) = (at(x0, w).min(w - 1), at(y0, h).min(h - 1));
    let (xb, yb) = (at(x1, w).max(xa + 1).min(w), at(y1, h).max(ya + 1).min(h));
    let pixels = (ya..yb)
        .flat_map(|y| img.pixels[y * w + xa..y * w + xb].iter().copied())
        .collect();
    Image::new(xb - xa, yb - ya, pixels)
}

/// The largest rectangle `ratio` wide for each unit high, centred in `rect`.
pub fn fit_aspect(rect: Rect, ratio: f32, w: usize, h: usize) -> Rect {
    let (wf, hf) = (w as f32, h as f32);
    let [x0, y0, x1, y1] = rect;
    let (mut cw, mut ch) = ((x1 - x0) * wf, (y1 - y0) * hf);
    if cw / ch > ratio {
        cw = ch * ratio;
    } else {
        ch = cw / ratio;
    }
    let (cx, cy) = ((x0 + x1) / 2.0 * wf, (y0 + y1) / 2.0 * hf);
    [
        (cx - cw / 2.0) / wf,
        (cy - ch / 2.0) / hf,
        (cx + cw / 2.0) / wf,
        (cy + ch / 2.0) / hf,
    ]
}

/// Moves the rectangle, stopping at the edges of the picture.
pub fn move_crop(rect: Rect, dx: f32, dy: f32) -> Rect {
    let [x0, y0, x1, y1] = rect;
    let dx = dx.max(-x0).min(1.0 - x1);
    let dy = dy.max(-y0).min(1.0 - y1);
    [x0 + dx, y0 + dy, x1 + dx, y1 + dy]
}

/// Grows or shrinks the rectangle from its bottom right corner, keeping `ratio` when there is one.
/// A change that would leave the picture or make the rectangle tiny is refused.
pub fn resize_crop(rect: Rect, dw: f32, dh: f32, ratio: Option<f32>, w: usize, h: usize) -> Rect {
    let [x0, y0, x1, y1] = rect;
    let (wf, hf) = (w as f32, h as f32);
    let (mut nx1, mut ny1) = (x1 + dw, y1 + dh);
    if let Some(ratio) = ratio {
        if dw != 0.0 {
            ny1 = y0 + (nx1 - x0) * wf / ratio / hf;
        } else {
            nx1 = x0 + (ny1 - y0) * hf * ratio / wf;
        }
    }
    if nx1 - x0 < 0.05 || ny1 - y0 < 0.05 || nx1 > 1.0 + 1e-6 || ny1 > 1.0 + 1e-6 {
        return rect;
    }
    [x0, y0, nx1, ny1]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(w: usize, h: usize) -> Image {
        let pixels = (0..w * h)
            .map(|i| [(i % w) as f32 / w as f32, (i / w) as f32 / h as f32, 0.5])
            .collect();
        Image::new(w, h, pixels)
    }

    fn close(a: Rect, b: Rect) -> bool {
        a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-5)
    }

    #[test]
    fn a_quarter_turn_swaps_the_sides_and_four_come_back() {
        let img = gradient(24, 16);
        let turned = rotate_cw(&img);
        assert_eq!((turned.width, turned.height), (16, 24));
        // The bottom left corner comes to the top left.
        assert_eq!(turned.pixels[0], img.pixels[15 * 24]);
        let back = rotate_cw(&rotate_cw(&rotate_cw(&turned)));
        assert_eq!(back, img);
    }

    #[test]
    fn straightening_shrinks_keeping_the_shape_and_zero_changes_nothing() {
        let img = gradient(100, 60);
        assert_eq!(straighten(img.clone(), 0.0), img);
        let out = straighten(img, 10.0);
        assert!(out.width < 100 && out.height < 60);
        let shape = out.width as f32 / out.height as f32;
        assert!((shape / (100.0 / 60.0) - 1.0).abs() < 0.05, "{shape}");
    }

    #[test]
    fn a_square_crop_of_a_landscape_takes_its_full_height() {
        let img = gradient(300, 200);
        let out = crop(img, fit_aspect(FULL, 1.0, 300, 200));
        assert_eq!((out.width, out.height), (200, 200));
    }

    #[test]
    fn ratios_turn_with_the_picture() {
        assert_eq!(aspect_ratio(Aspect::Free, 300, 200), None);
        assert_eq!(aspect_ratio(Aspect::Original, 300, 200), Some(1.5));
        assert_eq!(aspect_ratio(Aspect::Ratio(4, 3), 300, 200), Some(4.0 / 3.0));
        assert_eq!(aspect_ratio(Aspect::Ratio(4, 3), 200, 300), Some(3.0 / 4.0));
    }

    #[test]
    fn moving_stops_at_the_edge() {
        let moved = move_crop([0.1, 0.1, 0.5, 0.5], 1.0, 0.0);
        assert!(close(moved, [0.6, 0.1, 1.0, 0.5]), "{moved:?}");
    }

    #[test]
    fn resizing_keeps_the_ratio_and_stays_inside() {
        let grown = resize_crop([0.0, 0.0, 0.5, 0.5], 0.1, 0.0, Some(1.0), 300, 200);
        assert!(close(grown, [0.0, 0.0, 0.6, 0.9]), "{grown:?}");
        let full = [0.0, 0.0, 0.6, 0.9];
        assert_eq!(resize_crop(full, 0.1, 0.0, Some(1.0), 300, 200), full);
        let tiny = [0.0, 0.0, 0.06, 0.06];
        assert_eq!(resize_crop(tiny, -0.05, 0.0, None, 300, 200), tiny);
    }

    #[test]
    fn crops_of_tiny_pictures_keep_a_pixel() {
        let out = crop(gradient(1, 1), [0.2, 0.2, 0.8, 0.8]);
        assert_eq!((out.width, out.height), (1, 1));
        let out = straighten(gradient(2, 3), 10.0);
        assert!(out.width >= 1 && out.height >= 1);
    }
}
