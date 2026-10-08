//! The edits, applied in a fixed order to linear RGB. A port of termilight's `pipeline.py`: the same
//! steps, constants and order, so both give the same picture.

use rayon::prelude::*;

use super::{
    Image,
    geometry::{self, Aspect, FULL, Rect},
    white::{self, White},
};

pub const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];
pub const CURVE_X: [f32; 5] = [0.0, 0.25, 0.5, 0.75, 1.0];
const CURVE_IDENTITY: [f32; 5] = [0.0, 25.0, 50.0, 75.0, 100.0];
pub const HSL_BANDS: [&str; 8] = [
    "red", "orange", "yellow", "green", "aqua", "blue", "purple", "magenta",
];
/// Hues the bands are centred on, as shares of the colour wheel, with red again at the end.
const HSL_CENTERS: [f32; 9] = [0.0, 30.0, 60.0, 120.0, 180.0, 240.0, 270.0, 300.0, 360.0];

/// Every slider. Each has a neutral value at which its step is skipped, so the defaults change nothing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    /// Kelvin and tint from -150 to 150 for a raw that knows its white, as Lightroom shows them;
    /// otherwise shifts from -100 to 100.
    pub temp: f32,
    pub tint: f32,
    /// The white a raw was shot with, which `temp` and `tint` start at.
    pub as_shot: Option<White>,
    /// Stops, -5 to 5. The other sliders go from -100 to 100 unless noted.
    pub exposure: f32,
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,
    /// Output at 0, 25, 50, 75 and 100% input, 0 to 100.
    pub curve: [f32; 5],
    pub hsl_h: [f32; 8],
    pub hsl_s: [f32; 8],
    pub hsl_l: [f32; 8],
    pub vibrance: f32,
    pub saturation: f32,
    pub clarity: f32,
    pub texture: f32,
    /// 0 to 150.
    pub sharpen: f32,
    /// Pixels of the full-size photo, 0.5 to 3.
    pub sharpen_radius: f32,
    /// Quarter turns clockwise.
    pub rot90: u8,
    /// Degrees, -45 to 45.
    pub straighten: f32,
    pub crop: Rect,
    pub aspect: Aspect,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            temp: 0.0,
            tint: 0.0,
            as_shot: None,
            exposure: 0.0,
            contrast: 0.0,
            highlights: 0.0,
            shadows: 0.0,
            whites: 0.0,
            blacks: 0.0,
            curve: CURVE_IDENTITY,
            hsl_h: [0.0; 8],
            hsl_s: [0.0; 8],
            hsl_l: [0.0; 8],
            vibrance: 0.0,
            saturation: 0.0,
            clarity: 0.0,
            texture: 0.0,
            sharpen: 0.0,
            sharpen_radius: 1.0,
            rot90: 0,
            straighten: 0.0,
            crop: FULL,
            aspect: Aspect::Free,
        }
    }
}

/// Applies `s` to a linear RGB picture and gives sRGB clipped to 0..1. `scale` is the picture's
/// width over the full photo's, which sharpening's radius is given in.
pub fn process(img: Image, s: &Settings, scale: f32) -> Image {
    let mut img = geometry::crop(geometry(img, s), s.crop);
    white_balance(&mut img, s);
    if s.exposure != 0.0 {
        let gain = 2f32.powf(s.exposure);
        map(&mut img, |p| p.map(|c| c * gain));
    }
    contrast(&mut img, s.contrast);
    map(&mut img, |p| p.map(linear_to_srgb));
    tone(&mut img, s.highlights, s.shadows, s.whites, s.blacks);
    tone_curve(&mut img, s.curve);
    hsl(&mut img, &s.hsl_h, &s.hsl_s, &s.hsl_l);
    vibrance_saturation(&mut img, s.vibrance, s.saturation);
    detail(&mut img, s, scale);
    map(&mut img, |p| p.map(|c| c.clamp(0.0, 1.0)));
    img
}

/// The quarter turns and the straightening, which the crop rectangle is relative to.
pub fn geometry(mut img: Image, s: &Settings) -> Image {
    for _ in 0..s.rot90 % 4 {
        img = geometry::rotate_cw(&img);
    }
    geometry::straighten(img, s.straighten)
}

pub fn srgb_to_linear(x: f32) -> f32 {
    if x <= 0.04045 {
        x / 12.92
    } else {
        ((x + 0.055) / 1.055).powf(2.4)
    }
}

pub fn linear_to_srgb(x: f32) -> f32 {
    let x = x.max(0.0);
    if x <= 0.003_130_8 {
        x * 12.92
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    }
}

fn luma(p: [f32; 3]) -> f32 {
    p[0] * LUMA[0] + p[1] * LUMA[1] + p[2] * LUMA[2]
}

fn map(img: &mut Image, f: impl Fn([f32; 3]) -> [f32; 3] + Sync) {
    img.pixels.par_iter_mut().for_each(|p| *p = f(*p));
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// For a raw, an adaptation from the white it was shot with to the one chosen. Otherwise gains for
/// red, green and blue that keep the brightness of grey.
fn white_balance(img: &mut Image, s: &Settings) {
    let (temp, tint) = (s.temp, s.tint);
    if let Some(shot) = s.as_shot {
        if [temp, tint] != shot {
            let m = white::rebalance([temp, tint], shot);
            map(img, |p| {
                m.map(|row| row[0] * p[0] + row[1] * p[1] + row[2] * p[2])
            });
        }
        return;
    }
    if temp == 0.0 && tint == 0.0 {
        return;
    }
    let (t, g) = (temp / 100.0, tint / 100.0);
    let m = [2f32.powf(0.5 * t), 2f32.powf(-0.5 * g), 2f32.powf(-0.5 * t)];
    let norm = luma(m);
    let m = m.map(|v| v / norm);
    map(img, |p| [p[0] * m[0], p[1] * m[1], p[2] * m[2]]);
}

/// A power curve through 18% grey, which stays where it is.
fn contrast(img: &mut Image, amount: f32) {
    if amount == 0.0 {
        return;
    }
    let k = 1.0 + amount / 100.0 * 0.6;
    map(img, |p| p.map(|c| 0.18 * (c.max(0.0) / 0.18).powf(k)));
}

/// Highlights, shadows, whites and blacks, on values after the gamma that may still be above one.
fn tone(img: &mut Image, highlights: f32, shadows: f32, whites: f32, blacks: f32) {
    if highlights == 0.0 && shadows == 0.0 && whites == 0.0 && blacks == 0.0 {
        return;
    }
    map(img, |p| {
        let y = luma(p).clamp(0.0, 1.0);
        let d = (highlights * smoothstep(0.5, 1.0, y)
            + shadows * (1.0 - smoothstep(0.0, 0.5, y))
            + whites * y.powi(4)
            + blacks * (1.0 - y).powi(4))
            / 100.0
            * 0.5;
        p.map(|c| c + d)
    });
}

/// The tone curve through the five points as a function of input from 0 to 1. Monotone cubic
/// (PCHIP, as scipy's `PchipInterpolator`), so it never overshoots between points.
pub fn curve_fn(points: [f32; 5]) -> impl Fn(f32) -> f32 {
    let h = 0.25;
    let y = points.map(|p| p / 100.0);
    let m: [f32; 4] = std::array::from_fn(|k| (y[k + 1] - y[k]) / h);
    let mut d = [0.0f32; 5];
    for k in 1..4 {
        if m[k - 1] * m[k] > 0.0 {
            d[k] = 2.0 / (1.0 / m[k - 1] + 1.0 / m[k]);
        }
    }
    let edge = |m0: f32, m1: f32| {
        let d = (3.0 * m0 - m1) / 2.0;
        if d.signum() != m0.signum() || m0 == 0.0 {
            0.0
        } else if m0.signum() != m1.signum() && d.abs() > 3.0 * m0.abs() {
            3.0 * m0
        } else {
            d
        }
    };
    d[0] = edge(m[0], m[1]);
    d[4] = edge(m[3], m[2]);
    move |x: f32| {
        let k = ((x / h) as usize).min(3);
        let t = (x - CURVE_X[k]) / h;
        let (t2, t3) = (t * t, t * t * t);
        (2.0 * t3 - 3.0 * t2 + 1.0) * y[k]
            + (t3 - 2.0 * t2 + t) * h * d[k]
            + (-2.0 * t3 + 3.0 * t2) * y[k + 1]
            + (t3 - t2) * h * d[k + 1]
    }
}

fn tone_curve(img: &mut Image, points: [f32; 5]) {
    if points == CURVE_IDENTITY {
        return;
    }
    let f = curve_fn(points);
    let lut: Vec<f32> = (0..4096)
        .map(|i| f(i as f32 / 4095.0).clamp(0.0, 1.0))
        .collect();
    map(img, |p| {
        p.map(|c| lut[(c.clamp(0.0, 1.0) * 4095.0 + 0.5) as usize])
    });
}

/// Hue as a share of the wheel, saturation and value.
pub fn rgb_to_hsv([r, g, b]: [f32; 3]) -> [f32; 3] {
    let mx = r.max(g).max(b);
    let d = mx - r.min(g).min(b);
    let h = if d <= 0.0 {
        0.0
    } else if mx == r {
        ((g - b) / d).rem_euclid(6.0) / 6.0
    } else if mx == g {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    let s = if mx > 0.0 { d / mx } else { 0.0 };
    [h.rem_euclid(1.0), s, mx]
}

pub fn hsv_to_rgb([h, s, v]: [f32; 3]) -> [f32; 3] {
    [5.0, 3.0, 1.0].map(|n: f32| {
        let k = (n + h * 6.0).rem_euclid(6.0);
        v - v * s * k.min(4.0 - k).clamp(0.0, 1.0)
    })
}

/// Hue, saturation and lightness per colour band. Each hue belongs to the two nearest bands, blended
/// by a cosine so neighbouring bands meet smoothly.
fn hsl(img: &mut Image, dh: &[f32; 8], ds: &[f32; 8], dl: &[f32; 8]) {
    if [dh, ds, dl].iter().all(|v| v.iter().all(|&x| x == 0.0)) {
        return;
    }
    // Band weights for 1024 hues, worked out once, then only looked up.
    let lut = |v: &[f32; 8]| -> Vec<f32> {
        (0..1024)
            .map(|q| {
                let hue = q as f32 / 1023.0 * 360.0;
                let i = HSL_CENTERS
                    .iter()
                    .rposition(|&c| c <= hue)
                    .unwrap_or(0)
                    .min(7);
                let t = (hue - HSL_CENTERS[i]) / (HSL_CENTERS[i + 1] - HSL_CENTERS[i]);
                let t = 0.5 - 0.5 * (std::f32::consts::PI * t).cos();
                (v[i] * (1.0 - t) + v[(i + 1) % 8] * t) / 100.0
            })
            .collect()
    };
    let (lh, ls, ll) = (lut(dh), lut(ds), lut(dl));
    map(img, |p| {
        let [h, s, v] = rgb_to_hsv(p.map(|c| c.clamp(0.0, 1.0)));
        let q = (h * 1023.0 + 0.5) as usize;
        hsv_to_rgb([
            (h + lh[q] * 30.0 / 360.0).rem_euclid(1.0),
            (s * (1.0 + ls[q])).clamp(0.0, 1.0),
            // Greys have no hue to belong to a band, so they keep their lightness.
            v * (1.0 + 0.5 * ll[q] * s),
        ])
    });
}

/// Vibrance raises dull colours more than vivid ones; saturation raises all alike.
fn vibrance_saturation(img: &mut Image, vibrance: f32, saturation: f32) {
    if vibrance == 0.0 && saturation == 0.0 {
        return;
    }
    map(img, |p| {
        let y = luma(p);
        let mx = p[0].max(p[1]).max(p[2]);
        let mn = p[0].min(p[1]).min(p[2]);
        let s = if mx > 0.0 {
            (mx - mn) / mx.max(1e-6)
        } else {
            0.0
        };
        let k = (1.0 + saturation / 100.0) * (1.0 + vibrance / 100.0 * (1.0 - s));
        p.map(|c| y + (c - y) * k)
    });
}

/// Clarity, texture and sharpening: the same local contrast at three sizes. Clarity's and texture's
/// sizes are shares of the picture, so a preview looks like the full photo.
fn detail(img: &mut Image, s: &Settings, scale: f32) {
    let long = img.width.max(img.height) as f32;
    if s.clarity != 0.0 {
        local_contrast(img, s.clarity / 100.0, 0.02 * long, true);
    }
    if s.texture != 0.0 {
        local_contrast(img, s.texture / 100.0, 0.003 * long, false);
    }
    if s.sharpen != 0.0 {
        local_contrast(
            img,
            s.sharpen / 100.0,
            (s.sharpen_radius * scale).max(0.3),
            false,
        );
    }
}

/// Adds the difference between the brightness and a blur of it. `midtones` spares the darkest and
/// brightest parts, which would otherwise clip.
fn local_contrast(img: &mut Image, amount: f32, sigma: f32, midtones: bool) {
    let y: Vec<f32> = img
        .pixels
        .par_iter()
        .map(|&p| luma(p).clamp(0.0, 1.0))
        .collect();
    let blurred = blur(&y, img.width, img.height, sigma);
    img.pixels
        .par_iter_mut()
        .zip(y.par_iter().zip(&blurred))
        .for_each(|(p, (&y, &b))| {
            let mut d = y - b;
            if midtones {
                d *= (4.0 * y * (1.0 - y)).clamp(0.0, 1.0);
            }
            *p = p.map(|c| c + amount * d);
        });
}

/// A Gaussian blur. Wide ones are three box blurs, whose cost does not grow with the width.
fn blur(data: &[f32], w: usize, h: usize, sigma: f32) -> Vec<f32> {
    let pass = |data: &[f32], w: usize, h: usize| -> Vec<f32> {
        let rows = if sigma < 3.0 {
            gaussian_rows(data, w, sigma)
        } else {
            let size = (4.0 * sigma * sigma + 1.0).sqrt().round_ties_even() as usize;
            let once = box_rows(data, w, size);
            let twice = box_rows(&once, w, size);
            box_rows(&twice, w, size)
        };
        transpose(&rows, w, h)
    };
    let across = pass(data, w, h);
    pass(&across, h, w)
}

fn transpose(data: &[f32], w: usize, h: usize) -> Vec<f32> {
    let mut out = vec![0.0; w * h];
    out.par_chunks_mut(h).enumerate().for_each(|(x, col)| {
        for (y, v) in col.iter_mut().enumerate() {
            *v = data[y * w + x];
        }
    });
    out
}

/// Each row blurred with a Gaussian, mirrored at the ends as scipy's `gaussian_filter` does.
fn gaussian_rows(data: &[f32], w: usize, sigma: f32) -> Vec<f32> {
    let radius = (4.0 * sigma + 0.5) as isize;
    let weights: Vec<f32> = (-radius..=radius)
        .map(|i| (-0.5 * (i * i) as f32 / (sigma * sigma)).exp())
        .collect();
    let total: f32 = weights.iter().sum();
    let mirror = |i: isize| {
        let period = 2 * w as isize;
        let m = i.rem_euclid(period) as usize;
        if m < w { m } else { 2 * w - 1 - m }
    };
    let mut out = vec![0.0; data.len()];
    out.par_chunks_mut(w)
        .zip(data.par_chunks(w))
        .for_each(|(out, row)| {
            for (x, o) in out.iter_mut().enumerate() {
                let sum: f32 = weights
                    .iter()
                    .enumerate()
                    .map(|(k, wt)| wt * row[mirror(x as isize + k as isize - radius)])
                    .sum();
                *o = sum / total;
            }
        });
    out
}

/// Each row averaged over `size` neighbours, repeating the end pixels as scipy's `uniform_filter`
/// with mode "nearest" does.
fn box_rows(data: &[f32], w: usize, size: usize) -> Vec<f32> {
    let left = size / 2;
    let mut out = vec![0.0; data.len()];
    out.par_chunks_mut(w)
        .zip(data.par_chunks(w))
        .for_each(|(out, row)| {
            let mut sums = Vec::with_capacity(w + size + 1);
            sums.push(0.0f64);
            for k in 0..w + size {
                let v = row[(k as isize - left as isize).clamp(0, w as isize - 1) as usize];
                sums.push(sums[k] + f64::from(v));
            }
            for (x, o) in out.iter_mut().enumerate() {
                *o = ((sums[x + size] - sums[x]) / size as f64) as f32;
            }
        });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pseudo-random but fixed values from 0 to 1.
    fn noise(w: usize, h: usize, seed: u32) -> Image {
        let mut state = seed.wrapping_mul(2_654_435_761).max(1);
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as f32 / u32::MAX as f32
        };
        Image::new(w, h, (0..w * h).map(|_| [next(), next(), next()]).collect())
    }

    fn solid(w: usize, h: usize, p: [f32; 3]) -> Image {
        Image::new(w, h, vec![p; w * h])
    }

    fn row(pixels: &[[f32; 3]]) -> Image {
        Image::new(pixels.len(), 1, pixels.to_vec())
    }

    fn linear(p: [f32; 3]) -> [f32; 3] {
        p.map(srgb_to_linear)
    }

    fn spread(p: [f32; 3]) -> f32 {
        p[0].max(p[1]).max(p[2]) - p[0].min(p[1]).min(p[2])
    }

    fn near(a: f32, b: f32, tolerance: f32) -> bool {
        (a - b).abs() <= tolerance
    }

    #[test]
    fn the_defaults_only_apply_the_gamma() {
        let img = noise(24, 16, 1);
        let out = process(img.clone(), &Settings::default(), 1.0);
        for (o, i) in out.pixels.iter().zip(&img.pixels) {
            for c in 0..3 {
                assert!(near(o[c], linear_to_srgb(i[c]).min(1.0), 1e-4));
            }
        }
    }

    #[test]
    fn one_stop_doubles_the_light() {
        let img = noise(8, 8, 2);
        let img = Image::new(
            8,
            8,
            img.pixels.iter().map(|p| p.map(|c| c * 0.4)).collect(),
        );
        let s = Settings {
            exposure: 1.0,
            ..Settings::default()
        };
        let out = process(img.clone(), &s, 1.0);
        for (o, i) in out.pixels.iter().zip(&img.pixels) {
            for c in 0..3 {
                assert!(near(srgb_to_linear(o[c]), i[c] * 2.0, 1e-4));
            }
        }
    }

    #[test]
    fn temperature_warms_and_tint_turns_magenta() {
        let grey = solid(2, 2, [0.2; 3]);
        let warm = Settings {
            temp: 50.0,
            ..Settings::default()
        };
        let p = linear(process(grey.clone(), &warm, 1.0).pixels[0]);
        assert!(p[0] > p[2]);
        let magenta = Settings {
            tint: 50.0,
            ..Settings::default()
        };
        let p = linear(process(grey, &magenta, 1.0).pixels[0]);
        assert!(p[1] < p[0]);
    }

    #[test]
    fn a_raw_white_in_kelvin_changes_nothing_as_shot_and_warms_above_it() {
        let grey = solid(2, 2, [0.2; 3]);
        let shot = Settings {
            temp: 5000.0,
            tint: 5.0,
            as_shot: Some([5000.0, 5.0]),
            ..Settings::default()
        };
        let same = process(grey.clone(), &shot, 1.0);
        assert_eq!(same, process(grey.clone(), &Settings::default(), 1.0));
        let warmer = Settings {
            temp: 6500.0,
            ..shot
        };
        let p = linear(process(grey, &warmer, 1.0).pixels[0]);
        assert!(p[0] > p[2], "{p:?}");
        assert!(near(luma(p), 0.2, 0.01), "grey keeps its brightness: {p:?}");
    }

    #[test]
    fn contrast_keeps_middle_grey() {
        let s = Settings {
            contrast: 50.0,
            ..Settings::default()
        };
        let out = process(row(&[[0.18; 3], [0.5; 3]]), &s, 1.0);
        assert!(near(srgb_to_linear(out.pixels[0][0]), 0.18, 1e-3));
        assert!(srgb_to_linear(out.pixels[1][0]) > 0.5);
    }

    #[test]
    fn shadows_lift_darks_more_than_brights() {
        let img = row(&[[0.02; 3], [0.7; 3]]);
        let base = process(img.clone(), &Settings::default(), 1.0);
        let s = Settings {
            shadows: 60.0,
            ..Settings::default()
        };
        let out = process(img, &s, 1.0);
        let dark = out.pixels[0][0] - base.pixels[0][0];
        let bright = out.pixels[1][0] - base.pixels[1][0];
        assert!(dark > 0.05 && dark > bright * 5.0);
    }

    #[test]
    fn highlights_recover_overexposed_parts() {
        let s = Settings {
            exposure: 1.0,
            highlights: -100.0,
            ..Settings::default()
        };
        assert!(process(solid(1, 1, [0.9; 3]), &s, 1.0).pixels[0][0] < 1.0);
    }

    #[test]
    fn the_curve_starts_straight_and_never_turns_back() {
        let straight = curve_fn(CURVE_IDENTITY);
        let bent = curve_fn([0.0, 10.0, 50.0, 90.0, 100.0]);
        let mut last = -1.0;
        for i in 0..50 {
            let x = i as f32 / 49.0;
            assert!(near(straight(x), x, 1e-6));
            assert!(bent(x) >= last);
            last = bent(x);
        }
    }

    #[test]
    fn the_curve_moves_the_middle() {
        let s = Settings {
            curve: [0.0, 25.0, 70.0, 75.0, 100.0],
            ..Settings::default()
        };
        // sRGB 0.5.
        let out = process(solid(1, 1, [0.2140; 3]), &s, 1.0);
        assert!(near(out.pixels[0][0], 0.70, 0.01));
    }

    #[test]
    fn hsv_comes_back_to_the_same_colour() {
        for p in noise(10, 10, 3).pixels {
            let back = hsv_to_rgb(rgb_to_hsv(p));
            assert!((0..3).all(|c| near(back[c], p[c], 1e-5)), "{p:?} {back:?}");
        }
    }

    #[test]
    fn the_red_band_only_changes_red() {
        let img = row(&[
            linear([1.0, 0.0, 0.0]),
            linear([0.0, 0.0, 1.0]),
            linear([0.5; 3]),
        ]);
        let mut s = Settings::default();
        s.hsl_s[0] = -100.0;
        let out = process(img.clone(), &s, 1.0);
        let base = process(img, &Settings::default(), 1.0);
        assert!(spread(out.pixels[0]) < 1e-3);
        for i in 1..3 {
            assert!((0..3).all(|c| near(out.pixels[i][c], base.pixels[i][c], 1e-4)));
        }
    }

    #[test]
    fn no_saturation_is_grey() {
        let s = Settings {
            saturation: -100.0,
            ..Settings::default()
        };
        for p in process(noise(24, 16, 4), &s, 1.0).pixels {
            assert!(spread(p) < 1e-4);
        }
    }

    #[test]
    fn vibrance_raises_dull_colours_more_than_vivid_ones() {
        let img = row(&[linear([0.5, 0.45, 0.45]), linear([1.0, 0.1, 0.1])]);
        let base = process(img.clone(), &Settings::default(), 1.0);
        let s = Settings {
            vibrance: 100.0,
            ..Settings::default()
        };
        let out = process(img, &s, 1.0);
        let gain = |i: usize| spread(out.pixels[i]) / spread(base.pixels[i]);
        assert!(gain(0) > gain(1) && gain(1) > 1.0 - 1e-6);
    }

    fn edge(w: usize, h: usize) -> Image {
        let pixels = (0..w * h)
            .map(|i| if i % w < w / 2 { [0.05; 3] } else { [0.4; 3] })
            .collect();
        Image::new(w, h, pixels)
    }

    #[test]
    fn clarity_and_sharpening_raise_contrast_at_an_edge() {
        let at = |img: &Image, x: usize, y: usize| img.pixels[y * img.width + x][0];
        let base = process(edge(64, 32), &Settings::default(), 1.0);
        let s = Settings {
            clarity: 100.0,
            ..Settings::default()
        };
        let out = process(edge(64, 32), &s, 1.0);
        assert!(at(&out, 31, 16) < at(&base, 31, 16));
        assert!(at(&out, 32, 16) > at(&base, 32, 16));
        let base = process(edge(16, 16), &Settings::default(), 1.0);
        let s = Settings {
            sharpen: 100.0,
            ..Settings::default()
        };
        assert!(at(&process(edge(16, 16), &s, 1.0), 8, 8) > at(&base, 8, 8));
    }

    #[test]
    fn extreme_settings_stay_finite_and_in_range() {
        let extreme = Settings {
            exposure: 5.0,
            contrast: 100.0,
            highlights: -100.0,
            shadows: 100.0,
            whites: 100.0,
            blacks: -100.0,
            temp: 100.0,
            tint: -100.0,
            curve: [100.0, 0.0, 100.0, 0.0, 100.0],
            hsl_h: [100.0; 8],
            hsl_s: [100.0; 8],
            hsl_l: [-100.0; 8],
            vibrance: 100.0,
            saturation: 100.0,
            clarity: 100.0,
            texture: 100.0,
            sharpen: 150.0,
            ..Settings::default()
        };
        let dark = Settings {
            exposure: -5.0,
            contrast: -100.0,
            highlights: 100.0,
            shadows: -100.0,
            whites: -100.0,
            blacks: 100.0,
            saturation: -100.0,
            clarity: -100.0,
            texture: -100.0,
            ..Settings::default()
        };
        for s in [extreme, dark] {
            for p in process(noise(24, 16, 5), &s, 1.0).pixels {
                assert!(p.iter().all(|c| c.is_finite() && (0.0..=1.0).contains(c)));
            }
        }
    }

    #[test]
    fn a_quarter_turn_swaps_the_sides() {
        let s = Settings {
            rot90: 1,
            ..Settings::default()
        };
        let out = process(noise(24, 16, 6), &s, 1.0);
        assert_eq!((out.width, out.height), (16, 24));
    }

    #[test]
    fn tiny_pictures_survive_everything() {
        let s = Settings {
            clarity: 50.0,
            texture: 50.0,
            sharpen: 100.0,
            straighten: 10.0,
            rot90: 1,
            crop: [0.2, 0.2, 0.8, 0.8],
            ..Settings::default()
        };
        for (w, h) in [(1, 1), (3, 2), (2, 3)] {
            let out = process(noise(w, h, 7), &s, 1.0);
            assert!(out.width >= 1 && out.height >= 1);
            assert!(out.pixels.iter().flatten().all(|c| c.is_finite()));
        }
    }
}
