//! Developing photos: the edits of a raw converter applied to a picture, shown live and exported.

pub mod geometry;
pub mod load;
pub mod pipeline;
pub mod white;

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use image::DynamicImage;
use ratatui::crossterm::event::KeyCode;

use crate::{
    imageview::ImageData,
    keys::Key,
    preview::{Line, Span},
    theme::{DIM, FG},
};
use geometry::{ASPECTS, Aspect, FULL};
use pipeline::{HSL_BANDS, Settings, linear_to_srgb, process};

/// A picture as floating point RGB, row by row.
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<[f32; 3]>,
}

impl Image {
    pub fn new(width: usize, height: usize, pixels: Vec<[f32; 3]>) -> Image {
        assert_eq!(pixels.len(), width * height);
        Image {
            width,
            height,
            pixels,
        }
    }
}

pub const PANELS: [&str; 5] = ["Basic", "Curve", "HSL", "Detail", "Crop"];
const HSL: usize = 2;
const CROP: usize = 4;

/// Edits are previewed on a copy of the photo this long, which is redone in about 50 ms with
/// every slider moved.
// ponytail: fixed size, the preview column's real pixel size if it shows soft on big screens
const PROXY_LONGEST: usize = 1600;
/// The preview is redone once the keys pause this long, so holding one down never queues work.
const DEBOUNCE: Duration = Duration::from_millis(50);
const HISTOGRAM_BINS: usize = 36;
const HISTOGRAM_ROWS: usize = 8;
/// Clipping below this share of the pixels is not worth a warning.
const CLIPPED_SHARE: f32 = 0.005;
const CROP_STEP: f32 = 0.01;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Field {
    Temp,
    Tint,
    Exposure,
    Contrast,
    Highlights,
    Shadows,
    Whites,
    Blacks,
    Vibrance,
    Saturation,
    Curve(usize),
    Hue,
    Sat,
    Lum,
    Clarity,
    Texture,
    Sharpen,
    Radius,
}

struct Slider {
    label: &'static str,
    field: Field,
    min: f32,
    max: f32,
    step: f32,
}

const fn slider(label: &'static str, field: Field, min: f32, max: f32, step: f32) -> Slider {
    Slider {
        label,
        field,
        min,
        max,
        step,
    }
}

const BASIC: [Slider; 10] = [
    slider("Temperature", Field::Temp, -100.0, 100.0, 1.0),
    slider("Tint", Field::Tint, -100.0, 100.0, 1.0),
    slider("Exposure", Field::Exposure, -5.0, 5.0, 0.05),
    slider("Contrast", Field::Contrast, -100.0, 100.0, 1.0),
    slider("Highlights", Field::Highlights, -100.0, 100.0, 1.0),
    slider("Shadows", Field::Shadows, -100.0, 100.0, 1.0),
    slider("Whites", Field::Whites, -100.0, 100.0, 1.0),
    slider("Blacks", Field::Blacks, -100.0, 100.0, 1.0),
    slider("Vibrance", Field::Vibrance, -100.0, 100.0, 1.0),
    slider("Saturation", Field::Saturation, -100.0, 100.0, 1.0),
];
const CURVE: [Slider; 5] = [
    slider("Point 0%", Field::Curve(0), 0.0, 100.0, 1.0),
    slider("Point 25%", Field::Curve(1), 0.0, 100.0, 1.0),
    slider("Point 50%", Field::Curve(2), 0.0, 100.0, 1.0),
    slider("Point 75%", Field::Curve(3), 0.0, 100.0, 1.0),
    slider("Point 100%", Field::Curve(4), 0.0, 100.0, 1.0),
];
const HSL_SLIDERS: [Slider; 3] = [
    slider("Hue", Field::Hue, -100.0, 100.0, 1.0),
    slider("Saturation", Field::Sat, -100.0, 100.0, 1.0),
    slider("Luminance", Field::Lum, -100.0, 100.0, 1.0),
];
const DETAIL: [Slider; 4] = [
    slider("Clarity", Field::Clarity, -100.0, 100.0, 1.0),
    slider("Texture", Field::Texture, -100.0, 100.0, 1.0),
    slider("Sharpen", Field::Sharpen, 0.0, 150.0, 1.0),
    slider("Radius", Field::Radius, 0.5, 3.0, 0.1),
];

/// For a raw that knows its white, temperature is in kelvin and tint goes to 150, as in Lightroom.
const KELVIN: Slider = slider(
    "Temperature",
    Field::Temp,
    white::KELVIN_MIN,
    white::KELVIN_MAX,
    1.0,
);
const RAW_TINT: Slider = slider(
    "Tint",
    Field::Tint,
    -white::TINT_RANGE,
    white::TINT_RANGE,
    1.0,
);
/// A step of the temperature, in reciprocal megakelvin: about 50 K at 5000 K, finer when warmer,
/// coarser when cooler, as the eye sees the difference.
const MIRED_STEP: f32 = 2.0;

/// `kelvin` moved by `steps` of [`MIRED_STEP`], to the nearest 10 K and at least 10 K.
fn step_kelvin(kelvin: f32, steps: i32) -> f32 {
    let moved = 1e6 / (1e6 / kelvin - steps as f32 * MIRED_STEP);
    let rounded = (moved / 10.0).round() * 10.0;
    let rounded = if rounded == kelvin {
        kelvin + 10.0 * steps.signum() as f32
    } else {
        rounded
    };
    rounded.clamp(white::KELVIN_MIN, white::KELVIN_MAX)
}

/// Where `v` sits between `min` and `max` from 0 to 1. Kelvin are spaced as reciprocals, so that the
/// common temperatures are not all squeezed against the left end.
fn position(slider: &Slider, v: f32, kelvin: bool) -> f32 {
    if kelvin && slider.field == Field::Temp {
        (1.0 / slider.min - 1.0 / v) / (1.0 / slider.min - 1.0 / slider.max)
    } else {
        (v - slider.min) / (slider.max - slider.min)
    }
}

fn sliders(panel: usize) -> &'static [Slider] {
    match panel {
        0 => &BASIC,
        1 => &CURVE,
        HSL => &HSL_SLIDERS,
        3 => &DETAIL,
        _ => &[],
    }
}

/// The setting a slider moves. The HSL sliders move the one of `band`.
fn slot(s: &mut Settings, field: Field, band: usize) -> &mut f32 {
    match field {
        Field::Temp => &mut s.temp,
        Field::Tint => &mut s.tint,
        Field::Exposure => &mut s.exposure,
        Field::Contrast => &mut s.contrast,
        Field::Highlights => &mut s.highlights,
        Field::Shadows => &mut s.shadows,
        Field::Whites => &mut s.whites,
        Field::Blacks => &mut s.blacks,
        Field::Vibrance => &mut s.vibrance,
        Field::Saturation => &mut s.saturation,
        Field::Curve(i) => &mut s.curve[i],
        Field::Hue => &mut s.hsl_h[band],
        Field::Sat => &mut s.hsl_s[band],
        Field::Lum => &mut s.hsl_l[band],
        Field::Clarity => &mut s.clarity,
        Field::Texture => &mut s.texture,
        Field::Sharpen => &mut s.sharpen,
        Field::Radius => &mut s.sharpen_radius,
    }
}

fn value(s: &Settings, field: Field, band: usize) -> f32 {
    *slot(&mut s.clone(), field, band)
}

/// Work for the runtime to do off the main thread.
pub enum Job {
    Load {
        path: PathBuf,
    },
    Render {
        path: PathBuf,
        generation: u64,
        proxy: Arc<Image>,
        settings: Settings,
        scale: f32,
        before: bool,
        framing: bool,
    },
    Export {
        path: PathBuf,
        dest: PathBuf,
        settings: Settings,
    },
}

/// What a job came back with.
pub enum Done {
    Loaded {
        path: PathBuf,
        /// The preview, its width over the photo's and the white a raw was shot with.
        result: Result<(Image, f32, Option<white::White>), String>,
    },
    Rendered {
        path: PathBuf,
        generation: u64,
        result: Result<(ImageData, Vec<Line>), String>,
    },
    Exported {
        dest: PathBuf,
        settings: Settings,
        result: Result<(), String>,
    },
}

impl Job {
    pub fn is_render(&self) -> bool {
        matches!(self, Job::Render { .. })
    }

    pub fn run(self) -> Done {
        match self {
            Job::Load { path } => {
                let result = guarded(|| {
                    let photo = load::load(&path)?;
                    let width = photo.image.width;
                    let proxy = load::downscale(photo.image, PROXY_LONGEST);
                    let scale = proxy.width as f32 / width as f32;
                    Ok((proxy, scale, photo.as_shot))
                });
                Done::Loaded { path, result }
            }
            Job::Render {
                path,
                generation,
                proxy,
                settings,
                scale,
                before,
                framing,
            } => {
                let result = guarded(|| {
                    let (image, histogram) = render(&proxy, &settings, scale, before, framing);
                    Ok((ImageData(Arc::new(image)), histogram))
                });
                Done::Rendered {
                    path,
                    generation,
                    result,
                }
            }
            Job::Export {
                path,
                dest,
                settings,
            } => {
                let result = guarded(|| {
                    let full = load::load(&path)?.image;
                    load::export(&process(full, &settings, 1.0), &dest).map_err(|e| e.to_string())
                });
                Done::Exported {
                    dest,
                    settings,
                    result,
                }
            }
        }
    }
}

/// `work`'s result, or an error when it panics, as a raw decoder may on a file it does not expect.
fn guarded<T>(work: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
        .unwrap_or_else(|_| Err("the decoder crashed on this file".into()))
}

/// The picture to show and its histogram. `before` shows the photo unedited; `framing` shows all of
/// it with what the crop leaves out dimmed, and the histogram of what it keeps.
fn render(
    proxy: &Image,
    s: &Settings,
    scale: f32,
    before: bool,
    framing: bool,
) -> (DynamicImage, Vec<Line>) {
    let mut out = if before {
        let pixels = proxy
            .pixels
            .iter()
            .map(|p| p.map(|c| linear_to_srgb(c).min(1.0)))
            .collect();
        Image::new(proxy.width, proxy.height, pixels)
    } else if framing {
        let whole = Settings { crop: FULL, ..*s };
        process(proxy.clone(), &whole, scale)
    } else {
        process(proxy.clone(), s, scale)
    };
    let histogram = if framing && !before {
        histogram(&geometry::crop(out.clone(), s.crop))
    } else {
        histogram(&out)
    };
    if framing && !before {
        dim_outside(&mut out, s.crop);
    }
    let (w, h) = (out.width as u32, out.height as u32);
    let bytes = out
        .pixels
        .iter()
        .flatten()
        .map(|c| (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)
        .collect();
    let rgb = image::RgbImage::from_raw(w, h, bytes).expect("sized to fit");
    (DynamicImage::ImageRgb8(rgb), histogram)
}

fn dim_outside(img: &mut Image, rect: geometry::Rect) {
    let (w, h) = (img.width as f32, img.height as f32);
    let [x0, y0, x1, y1] = rect;
    let (xa, xb) = ((x0 * w) as usize, (x1 * w) as usize);
    let (ya, yb) = ((y0 * h) as usize, (y1 * h) as usize);
    for (i, p) in img.pixels.iter_mut().enumerate() {
        let (x, y) = (i % img.width, i / img.width);
        if !(xa..xb).contains(&x) || !(ya..yb).contains(&y) {
            *p = p.map(|c| c * 0.35);
        }
    }
}

/// Red, green and blue bars laid over each other, so where they meet they mix towards white, and a
/// line when shadows or highlights clip.
fn histogram(img: &Image) -> Vec<Line> {
    let mut counts = [[0u32; HISTOGRAM_BINS]; 3];
    let (mut dark, mut bright) = (0usize, 0usize);
    for p in &img.pixels {
        let p = p.map(|c| c.clamp(0.0, 1.0));
        for c in 0..3 {
            counts[c][((p[c] * HISTOGRAM_BINS as f32) as usize).min(HISTOGRAM_BINS - 1)] += 1;
        }
        dark += usize::from(p[0].max(p[1]).max(p[2]) <= 0.002);
        bright += usize::from(p[0].min(p[1]).min(p[2]) >= 0.998);
    }
    // The end bins hold the clipped pixels, and a spike there would flatten everything between.
    let peak = counts
        .iter()
        .flat_map(|c| &c[1..HISTOGRAM_BINS - 1])
        .copied()
        .max()
        .unwrap_or(0)
        .max(1) as f32;
    const BARS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let mut lines: Vec<Line> = (0..HISTOGRAM_ROWS)
        .rev()
        .map(|row| {
            let spans = (0..HISTOGRAM_BINS)
                .map(|bin| {
                    let fill = counts.map(|c| {
                        (c[bin] as f32 / peak * HISTOGRAM_ROWS as f32 - row as f32).clamp(0.0, 1.0)
                    });
                    let most = fill[0].max(fill[1]).max(fill[2]);
                    if most == 0.0 {
                        return Span::new(" ", DIM);
                    }
                    let [r, g, b] = fill.map(|f| if f > 0.0 { 230 } else { 50 });
                    let glyph = BARS[((most * 8.0).round() as usize).max(1)];
                    Span::new(glyph.to_string(), (r, g, b))
                })
                .collect();
            Line(spans)
        })
        .collect();
    let total = img.pixels.len().max(1) as f32;
    let mut warning = Vec::new();
    if dark as f32 / total > CLIPPED_SHARE {
        warning.push(Span::new("◀ shadows clip ", (0x79, 0xc0, 0xff)));
    }
    if bright as f32 / total > CLIPPED_SHARE {
        warning.push(Span::new("highlights clip ▶", (0xff, 0x7b, 0x72)));
    }
    lines.push(Line(warning));
    lines
}

/// A bar with a knob at `position`, from 0 to 1.
fn bar(position: f32) -> String {
    const WIDTH: usize = 16;
    let at = ((position.clamp(0.0, 1.0) * (WIDTH - 1) as f32).round() as usize).min(WIDTH - 1);
    format!("{}●{}", "━".repeat(at), "─".repeat(WIDTH - 1 - at))
}

/// The tone curve in braille dots, two across and four down to a character.
fn plot_curve(points: [f32; 5]) -> Vec<String> {
    const W: usize = 24;
    const H: usize = 6;
    const BITS: [[u32; 2]; 4] = [[0x01, 0x08], [0x02, 0x10], [0x04, 0x20], [0x40, 0x80]];
    let f = pipeline::curve_fn(points);
    let mut grid = [[0u32; W]; H];
    for i in 0..W * 2 {
        let y = f(i as f32 / (W * 2 - 1) as f32).clamp(0.0, 1.0);
        let dot = ((1.0 - y) * (H * 4 - 1) as f32).round_ties_even() as usize;
        grid[dot / 4][i / 2] |= BITS[dot % 4][i % 2];
    }
    grid.iter()
        .map(|row| {
            row.iter()
                .map(|&b| char::from_u32(0x2800 + b).unwrap())
                .collect()
        })
        .collect()
}

fn aspect_name(aspect: Aspect) -> String {
    match aspect {
        Aspect::Free => "free".into(),
        Aspect::Original => "original".into(),
        Aspect::Ratio(a, b) => format!("{a}:{b}"),
    }
}

/// A photo open for developing: the sliders, their history and what the screen shows.
pub struct Develop {
    path: PathBuf,
    /// The photo at preview size as linear RGB, once loaded.
    proxy: Option<Arc<Image>>,
    /// The preview's width over the full photo's, which sharpening's radius is scaled by.
    scale: f32,
    settings: Settings,
    /// What resetting a slider goes back to: the defaults, with a raw's white as shot.
    base: Settings,
    undo: Vec<Settings>,
    redo: Vec<Settings>,
    panel: usize,
    row: usize,
    band: usize,
    before: bool,
    /// Edited since the last export.
    dirty: bool,
    quit_armed: bool,
    /// The last rendered picture, its histogram and the generation it was rendered for.
    shown: Option<(ImageData, Vec<Line>, u64)>,
    /// When the preview went out of date, if it is.
    changed: Option<Instant>,
    generation: u64,
    jobs: Vec<Job>,
    /// Exports still being written, whose names are taken.
    exporting: HashSet<PathBuf>,
    pub message: Option<String>,
}

impl Develop {
    pub fn new(path: PathBuf) -> Develop {
        Develop {
            jobs: vec![Job::Load { path: path.clone() }],
            path,
            proxy: None,
            scale: 1.0,
            settings: Settings::default(),
            base: Settings::default(),
            undo: Vec::new(),
            redo: Vec::new(),
            panel: 0,
            row: 0,
            band: 0,
            before: false,
            dirty: false,
            quit_armed: false,
            shown: None,
            changed: None,
            generation: 0,
            exporting: HashSet::new(),
            message: None,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn dirty(&self) -> bool {
        self.dirty
    }

    pub fn is_loading(&self) -> bool {
        self.proxy.is_none()
    }

    pub fn shown(&self) -> Option<(&ImageData, &[Line])> {
        self.shown
            .as_ref()
            .map(|(image, lines, _)| (image, lines.as_slice()))
    }

    /// Jobs to start: loading and exports at once, the preview once the keys have paused.
    pub fn take_jobs(&mut self, now: Instant) -> Vec<Job> {
        if let (Some(proxy), Some(at)) = (&self.proxy, self.changed)
            && now >= at + DEBOUNCE
        {
            self.changed = None;
            self.generation += 1;
            self.jobs.push(Job::Render {
                path: self.path.clone(),
                generation: self.generation,
                proxy: Arc::clone(proxy),
                settings: self.settings,
                scale: self.scale,
                before: self.before,
                framing: self.panel == CROP,
            });
        }
        std::mem::take(&mut self.jobs)
    }

    /// How long until the preview is due, if it is waiting.
    pub fn wait(&self, now: Instant) -> Option<Duration> {
        self.proxy.as_ref()?;
        Some((self.changed? + DEBOUNCE).saturating_duration_since(now))
    }

    pub fn finish(&mut self, done: Done) {
        match done {
            Done::Loaded { path, result } if path == self.path => match result {
                Ok((proxy, scale, as_shot)) => {
                    if let Some(shot) = as_shot {
                        // White balance moved while loading was a shift; it becomes kelvin from here.
                        let history = self.undo.iter_mut().chain(&mut self.redo);
                        for s in [&mut self.settings, &mut self.base]
                            .into_iter()
                            .chain(history)
                        {
                            [s.temp, s.tint] = shot;
                            s.as_shot = Some(shot);
                        }
                    }
                    self.proxy = Some(Arc::new(proxy));
                    self.scale = scale;
                    self.changed = Some(Instant::now() - DEBOUNCE);
                }
                Err(e) => self.message = Some(format!("cannot develop: {e}")),
            },
            Done::Rendered {
                path,
                generation,
                result,
            } if path == self.path => match result {
                Ok((image, histogram)) => {
                    if self.shown.as_ref().is_none_or(|s| s.2 < generation) {
                        self.shown = Some((image, histogram, generation));
                    }
                }
                Err(e) => self.message = Some(e),
            },
            Done::Exported {
                dest,
                settings,
                result,
            } => {
                self.exporting.remove(&dest);
                match result {
                    Ok(()) => {
                        if settings == self.settings {
                            self.dirty = false;
                        }
                        self.message = Some(format!(
                            "exported {}",
                            dest.file_name().unwrap_or_default().to_string_lossy()
                        ));
                    }
                    Err(e) => self.message = Some(format!("not exported: {e}")),
                }
            }
            _ => {}
        }
    }

    /// Handles a key. True when the panel should close.
    pub fn press(&mut self, key: Key) -> bool {
        self.message = None;
        if key.ctrl {
            if key.code == KeyCode::Char('r') {
                self.redo();
            }
            return false;
        }
        match (key.code, key.typed_char()) {
            (KeyCode::Tab, _) => self.switch_panel(1),
            (KeyCode::BackTab, _) => self.switch_panel(PANELS.len() - 1),
            (KeyCode::Down, _) | (_, Some('j')) => self.arrow(0, 1),
            (KeyCode::Up, _) | (_, Some('k')) => self.arrow(0, -1),
            (KeyCode::Left, _) | (_, Some('h')) => self.arrow(-1, 0),
            (KeyCode::Right, _) | (_, Some('l')) => self.arrow(1, 0),
            (_, Some('J')) => self.arrow(0, 10),
            (_, Some('K')) => self.arrow(0, -10),
            (_, Some('H')) => self.arrow(-10, 0),
            (_, Some('L')) => self.arrow(10, 0),
            (_, Some('0')) => self.reset(),
            (_, Some('u')) => self.undo(),
            (_, Some('\\')) => {
                self.before = !self.before;
                self.message = Some(if self.before { "before" } else { "after" }.into());
                self.stale();
            }
            (_, Some('w')) => self.export(),
            (_, Some('[')) => self.bracket(-1),
            (_, Some(']')) => self.bracket(1),
            (_, Some('a')) => self.cycle_aspect(),
            (_, Some('r')) => self.rotate(),
            (KeyCode::Esc, _) | (_, Some('q')) => return self.quit(),
            _ => {}
        }
        false
    }

    /// Whether white balance is in kelvin, which it is for a raw that knows its white.
    fn kelvin(&self) -> bool {
        self.settings.as_shot.is_some()
    }

    fn sliders(&self) -> Vec<&'static Slider> {
        let kelvin = self.kelvin();
        sliders(self.panel)
            .iter()
            .map(|s| match s.field {
                Field::Temp if kelvin => &KELVIN,
                Field::Tint if kelvin => &RAW_TINT,
                _ => s,
            })
            .collect()
    }

    fn stale(&mut self) {
        self.changed.get_or_insert_with(Instant::now);
    }

    fn change(&mut self, new: Settings) {
        if new == self.settings {
            return;
        }
        self.undo.push(self.settings);
        self.redo.clear();
        self.settings = new;
        self.dirty = true;
        self.quit_armed = false;
        self.stale();
    }

    fn undo(&mut self) {
        if let Some(previous) = self.undo.pop() {
            self.redo
                .push(std::mem::replace(&mut self.settings, previous));
            self.dirty = true;
            self.stale();
        }
    }

    fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.settings, next));
            self.dirty = true;
            self.stale();
        }
    }

    fn switch_panel(&mut self, by: usize) {
        let was_crop = self.panel == CROP;
        self.panel = (self.panel + by) % PANELS.len();
        self.row = 0;
        if was_crop != (self.panel == CROP) {
            self.stale();
        }
    }

    /// The size of the preview after turning and straightening, which the crop is a share of.
    fn turned_size(&self) -> Option<(usize, usize)> {
        let proxy = self.proxy.as_ref()?;
        let s = &self.settings;
        let (w, h) = if s.rot90 % 2 == 1 {
            (proxy.height, proxy.width)
        } else {
            (proxy.width, proxy.height)
        };
        Some(geometry::straightened_size(w, h, s.straighten))
    }

    fn arrow(&mut self, dx: i32, dy: i32) {
        let s = self.settings;
        if self.panel == CROP {
            let (dxf, dyf) = (
                dx.signum() as f32 * CROP_STEP,
                dy.signum() as f32 * CROP_STEP,
            );
            let crop = if dx.abs() == 10 || dy.abs() == 10 {
                let Some((w, h)) = self.turned_size() else {
                    return;
                };
                let ratio = geometry::aspect_ratio(s.aspect, w, h);
                geometry::resize_crop(s.crop, dxf, dyf, ratio, w, h)
            } else {
                geometry::move_crop(s.crop, dxf, dyf)
            };
            return self.change(Settings { crop, ..s });
        }
        let sliders = self.sliders();
        if dy != 0 {
            let n = sliders.len() as i32;
            self.row = (self.row as i32 + dy.signum()).rem_euclid(n) as usize;
            return;
        }
        let slider = sliders[self.row];
        let mut new = s;
        let v = slot(&mut new, slider.field, self.band);
        *v = if self.kelvin() && slider.field == Field::Temp {
            step_kelvin(*v, dx)
        } else {
            let moved = *v + dx as f32 * slider.step;
            ((moved * 10_000.0).round() / 10_000.0).clamp(slider.min, slider.max) + 0.0
        };
        self.change(new);
    }

    fn reset(&mut self) {
        let s = self.settings;
        if self.panel == CROP {
            let d = Settings::default();
            return self.change(Settings {
                crop: d.crop,
                straighten: d.straighten,
                rot90: d.rot90,
                aspect: d.aspect,
                ..s
            });
        }
        let field = self.sliders()[self.row].field;
        let mut new = s;
        *slot(&mut new, field, self.band) = value(&self.base, field, self.band);
        self.change(new);
    }

    fn bracket(&mut self, by: i32) {
        if self.panel == HSL {
            self.band = (self.band as i32 + by).rem_euclid(HSL_BANDS.len() as i32) as usize;
        } else if self.panel == CROP {
            let s = self.settings;
            let angle = ((s.straighten + 0.5 * by as f32) * 100.0).round() / 100.0;
            self.change(Settings {
                straighten: angle.clamp(-45.0, 45.0),
                crop: FULL,
                aspect: Aspect::Free,
                ..s
            });
        }
    }

    fn cycle_aspect(&mut self) {
        if self.panel != CROP {
            return;
        }
        let Some((w, h)) = self.turned_size() else {
            return;
        };
        let s = self.settings;
        let at = ASPECTS.iter().position(|&a| a == s.aspect).unwrap_or(0);
        let aspect = ASPECTS[(at + 1) % ASPECTS.len()];
        let crop = match geometry::aspect_ratio(aspect, w, h) {
            Some(ratio) => geometry::fit_aspect(FULL, ratio, w, h),
            None => FULL,
        };
        self.change(Settings { aspect, crop, ..s });
    }

    fn rotate(&mut self) {
        if self.panel == CROP {
            let s = self.settings;
            self.change(Settings {
                rot90: (s.rot90 + 1) % 4,
                crop: FULL,
                aspect: Aspect::Free,
                ..s
            });
        }
    }

    fn export(&mut self) {
        // Names are taken here, on the main thread, so exports started together never collide.
        let dest = load::export_path(&self.path, &self.exporting);
        self.exporting.insert(dest.clone());
        self.message = Some(format!(
            "exporting {}…",
            dest.file_name().unwrap_or_default().to_string_lossy()
        ));
        self.jobs.push(Job::Export {
            path: self.path.clone(),
            dest,
            settings: self.settings,
        });
    }

    fn quit(&mut self) -> bool {
        if self.dirty && !self.quit_armed {
            self.quit_armed = true;
            self.message = Some("not exported: q again closes, w exports".into());
            return false;
        }
        true
    }

    /// The side panel: tabs, the sliders of the current one and what its keys do.
    pub fn panel_lines(&self) -> Vec<Line> {
        let tabs = PANELS
            .iter()
            .enumerate()
            .flat_map(|(i, name)| {
                let mut tab = Span::new(*name, if i == self.panel { FG } else { DIM });
                tab.bold = i == self.panel;
                [tab, Span::new(" ", DIM)]
            })
            .collect();
        let mut lines = vec![Line(tabs), Line::default()];
        let s = &self.settings;
        if self.panel == HSL {
            lines.push(Line(vec![
                Span::new("Band  ", DIM),
                Span::new(HSL_BANDS[self.band], FG),
                Span::new("  [ ]", DIM),
            ]));
        }
        let kelvin = self.kelvin();
        for (i, slider) in self.sliders().into_iter().enumerate() {
            let v = value(s, slider.field, self.band);
            let shown = if kelvin && slider.field == Field::Temp {
                format!("{v:.0} K")
            } else if slider.step < 1.0 {
                format!("{v:+.2}")
            } else {
                format!("{v:+.0}")
            };
            let selected = i == self.row;
            let mut label = Span::new(
                format!("{}{:<11} ", if selected { "▶" } else { " " }, slider.label),
                if selected { FG } else { DIM },
            );
            label.bold = selected;
            lines.push(Line(vec![
                label,
                Span::new(bar(position(slider, v, kelvin)), FG),
                Span::new(format!(" {shown}"), FG),
            ]));
        }
        if self.panel == 1 {
            lines.push(Line::default());
            lines.extend(
                plot_curve(s.curve)
                    .into_iter()
                    .map(|row| Line(vec![Span::new(row, FG)])),
            );
        }
        if self.panel == CROP {
            let rows = [
                ("Aspect      ", aspect_name(s.aspect), "  a"),
                ("Straighten  ", format!("{:+.1}°", s.straighten), "  [ ]"),
                (
                    "Rotation    ",
                    format!("{}°", u32::from(s.rot90) * 90),
                    "  r",
                ),
            ];
            for (label, value, keys) in rows {
                lines.push(Line(vec![
                    Span::new(label, DIM),
                    Span::new(value, FG),
                    Span::new(keys, DIM),
                ]));
            }
            lines.push(Line::default());
            for hint in [
                "h j k l  move the frame",
                "H J K L  resize it",
                "0        reset the crop",
            ] {
                lines.push(Line(vec![Span::new(hint, DIM)]));
            }
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Presses every key, saying whether any of them closed the panel.
    fn press(develop: &mut Develop, keys: &str) -> bool {
        let mut closed = false;
        for key in Key::parse_seq(keys).unwrap() {
            closed |= develop.press(key);
        }
        closed
    }

    fn loaded(w: usize, h: usize) -> Develop {
        let mut develop = Develop::new(PathBuf::from("/photos/a.raf"));
        develop.take_jobs(Instant::now());
        develop.finish(Done::Loaded {
            path: PathBuf::from("/photos/a.raf"),
            result: Ok((Image::new(w, h, vec![[0.2; 3]; w * h]), 0.5, None)),
        });
        develop
    }

    #[test]
    fn opening_asks_for_the_photo_then_renders_it_at_once() {
        let mut develop = Develop::new(PathBuf::from("/photos/a.raf"));
        let jobs = develop.take_jobs(Instant::now());
        assert!(matches!(jobs.as_slice(), [Job::Load { .. }]));
        assert!(develop.is_loading());
        develop.finish(Done::Loaded {
            path: PathBuf::from("/photos/a.raf"),
            result: Ok((Image::new(4, 2, vec![[0.2; 3]; 8]), 0.5, None)),
        });
        let jobs = develop.take_jobs(Instant::now());
        assert!(matches!(jobs.as_slice(), [Job::Render { scale, .. }] if *scale == 0.5));
    }

    #[test]
    fn the_preview_waits_for_the_keys_to_pause() {
        let mut develop = loaded(4, 2);
        let now = Instant::now();
        develop.take_jobs(now + DEBOUNCE);
        press(&mut develop, "lll");
        assert!(develop.take_jobs(Instant::now()).is_empty());
        assert!(develop.wait(Instant::now()).is_some());
        let jobs = develop.take_jobs(Instant::now() + DEBOUNCE);
        assert!(matches!(jobs.as_slice(), [Job::Render { settings, .. }] if settings.temp == 3.0));
        assert_eq!(develop.wait(Instant::now()), None);
    }

    #[test]
    fn sliders_move_by_their_step_stay_in_range_and_reset() {
        let mut develop = loaded(4, 2);
        press(&mut develop, "jjL");
        assert_eq!(develop.settings.exposure, 0.5);
        press(&mut develop, &"L".repeat(20));
        assert_eq!(develop.settings.exposure, 5.0);
        press(&mut develop, "0");
        assert_eq!(develop.settings.exposure, 0.0);
        press(&mut develop, "kkk");
        assert_eq!(develop.row, BASIC.len() - 1, "moving up from the top wraps");
    }

    #[test]
    fn undo_and_redo_walk_the_history() {
        let mut develop = loaded(4, 2);
        press(&mut develop, "lll");
        press(&mut develop, "uu");
        assert_eq!(develop.settings.temp, 1.0);
        press(&mut develop, "<c-r>");
        assert_eq!(develop.settings.temp, 2.0);
        press(&mut develop, "h");
        press(&mut develop, "<c-r>");
        assert_eq!(
            develop.settings.temp, 1.0,
            "a new change drops what could be redone"
        );
    }

    #[test]
    fn a_raw_white_balance_is_in_kelvin_from_the_white_it_was_shot_with() {
        let mut develop = Develop::new(PathBuf::from("/photos/a.raf"));
        develop.take_jobs(Instant::now());
        develop.finish(Done::Loaded {
            path: PathBuf::from("/photos/a.raf"),
            result: Ok((
                Image::new(4, 2, vec![[0.2; 3]; 8]),
                0.5,
                Some([5200.0, 8.0]),
            )),
        });
        let text = |d: &Develop| d.panel_lines().iter().map(Line::text).collect::<Vec<_>>();
        assert!(
            text(&develop).iter().any(|l| l.contains("5200 K")),
            "{:?}",
            text(&develop)
        );
        press(&mut develop, "l");
        assert_eq!(
            develop.settings.temp, 5250.0,
            "two mired is about 50 K at 5200 K"
        );
        press(&mut develop, "H");
        assert!(develop.settings.temp < 5000.0, "{}", develop.settings.temp);
        press(&mut develop, "0");
        assert_eq!(develop.settings.temp, 5200.0, "reset goes back to as shot");
        press(&mut develop, "jl");
        assert_eq!(develop.settings.tint, 9.0);
        press(&mut develop, &"L".repeat(20));
        assert_eq!(develop.settings.tint, white::TINT_RANGE);
        assert_eq!(step_kelvin(2000.0, -1), 2000.0, "stays in range");
        assert_eq!(step_kelvin(2000.0, 1), 2010.0, "moves at least 10 K");
    }

    #[test]
    fn hsl_sliders_move_the_chosen_band() {
        let mut develop = loaded(4, 2);
        press(&mut develop, "<tab><tab>]]l");
        assert_eq!(develop.settings.hsl_h[2], 1.0);
        assert_eq!(develop.settings.hsl_h[0], 0.0);
        press(&mut develop, "[[[");
        assert_eq!(develop.band, HSL_BANDS.len() - 1);
    }

    #[test]
    fn crop_keys_move_resize_turn_and_frame() {
        let mut develop = loaded(300, 200);
        press(&mut develop, "<tab><tab><tab><tab>");
        assert_eq!(develop.panel, CROP);
        press(&mut develop, "aa");
        assert_eq!(develop.settings.aspect, Aspect::Ratio(1, 1));
        let [x0, y0, x1, y1] = develop.settings.crop;
        assert!(((x1 - x0) * 300.0 - (y1 - y0) * 200.0).abs() < 0.5);
        press(&mut develop, "h");
        assert!(develop.settings.crop[0] < x0);
        press(&mut develop, "H");
        assert!(develop.settings.crop[2] < x1 - 0.01 + 1e-6);
        press(&mut develop, "r");
        assert_eq!(develop.settings.rot90, 1);
        assert_eq!(develop.settings.crop, FULL);
        press(&mut develop, "]0");
        assert_eq!(develop.settings.straighten, 0.0);
        assert_eq!(develop.settings.rot90, 0);
    }

    #[test]
    fn closing_with_edits_asks_twice_and_an_export_clears_that() {
        let mut develop = loaded(4, 2);
        assert!(press(&mut develop, "q"), "nothing edited closes at once");
        press(&mut develop, "l");
        assert!(!press(&mut develop, "q"));
        assert!(press(&mut develop, "q"));
        let mut develop = loaded(4, 2);
        press(&mut develop, "lw");
        let jobs = develop.take_jobs(Instant::now());
        let Some(Job::Export { dest, settings, .. }) = jobs.into_iter().find(|j| !j.is_render())
        else {
            panic!("no export");
        };
        assert_eq!(dest, PathBuf::from("/photos/a_edit.jpg"));
        press(&mut develop, "w");
        let second = develop.take_jobs(Instant::now());
        assert!(
            matches!(second.as_slice(), [Job::Export { dest, .. }] if dest.ends_with("a_edit-2.jpg"))
        );
        develop.finish(Done::Exported {
            dest,
            settings,
            result: Ok(()),
        });
        assert!(!develop.dirty());
        assert!(press(&mut develop, "q"));
    }

    #[test]
    fn rendering_frames_the_crop_and_counts_only_what_it_keeps() {
        let proxy = Image::new(10, 10, vec![[0.2; 3]; 100]);
        let s = Settings {
            crop: [0.0, 0.0, 0.5, 1.0],
            ..Settings::default()
        };
        let (framed, _) = render(&proxy, &s, 1.0, false, true);
        assert_eq!((framed.width(), framed.height()), (10, 10));
        let rgb = framed.to_rgb8();
        assert!(rgb.get_pixel(9, 5).0[0] < rgb.get_pixel(0, 5).0[0]);
        let (cropped, _) = render(&proxy, &s, 1.0, false, false);
        assert_eq!((cropped.width(), cropped.height()), (5, 10));
    }

    #[test]
    fn the_histogram_warns_about_clipping() {
        let text = |lines: Vec<Line>| lines.last().unwrap().text();
        let black = Image::new(4, 4, vec![[0.0; 3]; 16]);
        assert!(text(histogram(&black)).contains("shadows"));
        let white = Image::new(4, 4, vec![[1.0; 3]; 16]);
        assert!(text(histogram(&white)).contains("highlights"));
        let grey = Image::new(4, 4, vec![[0.5; 3]; 16]);
        assert_eq!(text(histogram(&grey)), "");
        assert_eq!(histogram(&grey).len(), HISTOGRAM_ROWS + 1);
    }

    #[test]
    fn the_curve_plot_is_a_diagonal_when_straight() {
        let plot = plot_curve([0.0, 25.0, 50.0, 75.0, 100.0]);
        assert_eq!(plot.len(), 6);
        assert!(
            plot[5].starts_with('⣀') || plot[5].starts_with('⡀'),
            "{plot:?}"
        );
        assert!(plot[0].ends_with('⠉') || plot[0].ends_with('⠈'), "{plot:?}");
    }
}
