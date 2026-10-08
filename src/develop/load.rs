//! Reading a photo as linear RGB to develop, and writing the result.

use std::{
    collections::HashSet,
    io,
    path::{Path, PathBuf},
};

use image::{DynamicImage, ImageBuffer, ImageReader, Rgb, Rgb32FImage, metadata::Orientation};
use rawler::imgop::develop::{Intermediate, ProcessingStep, RawDevelop};
use rayon::prelude::*;

use super::{Image, pipeline::srgb_to_linear};

/// Camera raw formats, by extension. The rest is read by the image library.
const RAW_EXTENSIONS: [&str; 17] = [
    "3fr", "arw", "cr2", "cr3", "dng", "erf", "iiq", "mos", "nef", "nrw", "orf", "pef", "raf",
    "rw2", "rwl", "sr2", "srw",
];

/// Everything a raw converter does but the sRGB gamma: linear RGB in sRGB primaries, with the white
/// balance the camera chose.
const RAW_STEPS: [ProcessingStep; 7] = [
    ProcessingStep::Rescale,
    ProcessingStep::Demosaic,
    ProcessingStep::FujiRotate,
    ProcessingStep::CropActiveArea,
    ProcessingStep::WhiteBalance,
    ProcessingStep::Calibrate,
    ProcessingStep::CropDefault,
];

const JPEG_QUALITY: u8 = 92;

/// The photo at `path`, upright, as linear RGB in sRGB primaries. Raw values may go above one.
pub fn load(path: &Path) -> Result<Image, String> {
    std::fs::metadata(path).map_err(|e| e.to_string())?;
    if is_raw(path) {
        return load_raw(path);
    }
    let decoded = ImageReader::open(path)
        .and_then(|r| r.with_guessed_format())
        .map_err(|e| e.to_string())
        .and_then(crate::preview::decode_limited);
    let (mut image, orientation, icc) = match decoded {
        Ok(found) => found,
        // HEIC and the like, which the system reads upright and in sRGB.
        #[cfg(target_os = "macos")]
        Err(e) => std::fs::read(path)
            .ok()
            .and_then(|data| crate::imageio::thumbnail(&data, u32::from(u16::MAX)))
            .map(|image| (image, Orientation::NoTransforms, None))
            .ok_or(e)?,
        #[cfg(not(target_os = "macos"))]
        Err(e) => return Err(e),
    };
    image.apply_orientation(orientation);
    Ok(linear(image, icc.as_deref()))
}

fn is_raw(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| RAW_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

// ponytail: a raw format that makes rawler panic takes tx down with it, catch_unwind if it happens.
fn load_raw(path: &Path) -> Result<Image, String> {
    let raw = match rawler::decode_file(path) {
        // rawler reads Canon's small raws fourteen times too dark; the camera's own picture is right.
        Ok(raw) if !raw.camera.mode.starts_with("sRaw") => raw,
        _ => return Ok(linear(crate::preview::embedded_picture(path)?, None)),
    };
    let developed = RawDevelop::new_with(&RAW_STEPS)
        .develop_intermediate(&raw)
        .map_err(|e| e.to_string())?;
    let image = match developed {
        Intermediate::ThreeColor(px) => Image::new(px.width, px.height, px.data),
        Intermediate::Monochrome(px) => Image::new(
            px.width,
            px.height,
            px.data.iter().map(|&v| [v; 3]).collect(),
        ),
        Intermediate::FourColor(_) => return Err("unsupported colour filter".into()),
    };
    // rawler does not know how every camera stores this; the preview's reading does.
    Ok(orient(image, crate::preview::raw_orientation(path)))
}

fn orient(image: Image, orientation: Orientation) -> Image {
    if orientation == Orientation::NoTransforms {
        return image;
    }
    let mut image = DynamicImage::ImageRgb32F(to_buffer(image));
    image.apply_orientation(orientation);
    from_buffer(image.into_rgb32f())
}

fn to_buffer(image: Image) -> Rgb32FImage {
    let (w, h) = (image.width as u32, image.height as u32);
    ImageBuffer::from_raw(w, h, image.pixels.into_flattened()).expect("sized to fit")
}

fn from_buffer(buffer: Rgb32FImage) -> Image {
    let (w, h) = (buffer.width() as usize, buffer.height() as usize);
    Image::new(w, h, buffer.into_raw().as_chunks::<3>().0.to_vec())
}

/// A decoded picture as linear RGB, converted to sRGB first when it carries another colour profile,
/// such as a phone's Display P3.
fn linear(image: DynamicImage, icc: Option<&[u8]>) -> Image {
    let transform = icc.and_then(|icc| {
        let profile = qcms::Profile::new_from_slice(icc, false)?;
        let srgb = qcms::Profile::new_sRGB();
        qcms::Transform::new(
            &profile,
            &srgb,
            qcms::DataType::RGB8,
            qcms::Intent::Perceptual,
        )
    });
    let eight_bit = matches!(
        image,
        DynamicImage::ImageLuma8(_)
            | DynamicImage::ImageLumaA8(_)
            | DynamicImage::ImageRgb8(_)
            | DynamicImage::ImageRgba8(_)
    );
    // ponytail: a profile drops a 16-bit picture to 8 bits, as termilight does; qcms only takes 8.
    if eight_bit || transform.is_some() {
        let mut rgb = image.into_rgb8();
        if let Some(transform) = transform {
            transform.apply(&mut rgb);
        }
        let lut: [f32; 256] = std::array::from_fn(|v| srgb_to_linear(v as f32 / 255.0));
        let (w, h) = (rgb.width() as usize, rgb.height() as usize);
        let pixels = rgb
            .pixels()
            .map(|p| p.0.map(|v| lut[usize::from(v)]))
            .collect();
        return Image::new(w, h, pixels);
    }
    let mut image = from_buffer(image.into_rgb32f());
    for p in &mut image.pixels {
        *p = p.map(srgb_to_linear);
    }
    image
}

/// The same picture no longer than `long_side` either way, each pixel the average of those it covers.
pub fn downscale(image: Image, long_side: usize) -> Image {
    let (w, h) = (image.width, image.height);
    if w.max(h) <= long_side {
        return image;
    }
    let k = long_side as f64 / w.max(h) as f64;
    let nw = ((w as f64 * k).round() as usize).max(1);
    let nh = ((h as f64 * k).round() as usize).max(1);
    // The pixels from `i` of `n` cells across `size` pixels, at least one.
    let span =
        |i: usize, n: usize, size: usize| i * size / n..((i + 1) * size / n).max(i * size / n + 1);
    let mut pixels = vec![[0.0; 3]; nw * nh];
    pixels.par_chunks_mut(nw).enumerate().for_each(|(y, row)| {
        let ys = span(y, nh, h);
        for (x, out) in row.iter_mut().enumerate() {
            let xs = span(x, nw, w);
            let mut sum = [0.0f32; 3];
            for sy in ys.clone() {
                for p in &image.pixels[sy * w + xs.start..sy * w + xs.end] {
                    sum = std::array::from_fn(|c| sum[c] + p[c]);
                }
            }
            let n = (ys.len() * xs.len()) as f32;
            *out = sum.map(|v| v / n);
        }
    });
    Image::new(nw, nh, pixels)
}

/// Where an export of `photo` goes: `<name>_edit.jpg` beside it, numbered when that is taken.
pub fn export_path(photo: &Path, taken: &HashSet<PathBuf>) -> PathBuf {
    let stem = photo.file_stem().unwrap_or_default().to_string_lossy();
    unique_path(&photo.with_file_name(format!("{stem}_edit.jpg")), taken)
}

/// `path`, or else the first of `<stem>-2`, `-3`… that neither exists nor is in `taken`.
pub fn unique_path(path: &Path, taken: &HashSet<PathBuf>) -> PathBuf {
    let free = |p: &PathBuf| !taken.contains(p) && std::fs::symlink_metadata(p).is_err();
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    std::iter::once(path.to_path_buf())
        .chain((2..).map(|n| path.with_file_name(format!("{stem}-{n}{ext}"))))
        .find(free)
        .expect("numbers run out after the disk does")
}

/// Writes sRGB values as a JPEG, or as a 16-bit TIFF when the name ends in `.tif` or `.tiff`.
pub fn export(image: &Image, path: &Path) -> io::Result<()> {
    let tiff = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "tif" | "tiff"));
    let (w, h) = (image.width as u32, image.height as u32);
    let values = image.pixels.iter().flatten().map(|c| c.clamp(0.0, 1.0));
    let mut bytes = Vec::new();
    let written = if tiff {
        let data = values.map(|c| (c * 65535.0 + 0.5) as u16).collect();
        let buffer: ImageBuffer<Rgb<u16>, Vec<u16>> = ImageBuffer::from_raw(w, h, data).unwrap();
        buffer.write_to(&mut io::Cursor::new(&mut bytes), image::ImageFormat::Tiff)
    } else {
        let data = values.map(|c| (c * 255.0 + 0.5) as u8).collect();
        let buffer = image::RgbImage::from_raw(w, h, data).unwrap();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, JPEG_QUALITY)
            .encode_image(&buffer)
    };
    written.map_err(io::Error::other)?;
    crate::save::write_file(path, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{develop::pipeline::linear_to_srgb, testdir::tempdir};

    fn near(a: f32, b: f32, tolerance: f32) -> bool {
        (a - b).abs() <= tolerance
    }

    #[test]
    fn the_gamma_comes_back() {
        for i in 0..=100 {
            let x = i as f32 / 100.0;
            assert!(near(srgb_to_linear(linear_to_srgb(x)), x, 1e-5));
        }
        assert!(near(srgb_to_linear(0.5), 0.2140, 1e-3));
    }

    #[test]
    fn a_png_comes_in_as_linear_rgb_whatever_its_channels() {
        let dir = tempdir();
        let path = dir.path().join("a.png");
        image::RgbImage::from_pixel(6, 4, Rgb([128; 3]))
            .save(&path)
            .unwrap();
        let img = load(&path).unwrap();
        assert_eq!((img.width, img.height), (6, 4));
        assert!(near(img.pixels[0][0], 0.2158, 1e-3));
        image::GrayImage::new(5, 3).save(&path).unwrap();
        assert_eq!(load(&path).unwrap().pixels.len(), 15);
        image::RgbaImage::new(5, 3).save(&path).unwrap();
        assert_eq!(load(&path).unwrap().pixels.len(), 15);
    }

    #[test]
    fn sixteen_bit_grey_keeps_its_scale() {
        let dir = tempdir();
        let path = dir.path().join("g16.png");
        ImageBuffer::<image::Luma<u16>, _>::from_pixel(4, 4, image::Luma([32768]))
            .save(&path)
            .unwrap();
        let img = load(&path).unwrap();
        assert!(near(img.pixels[0][1], srgb_to_linear(0.5), 2e-3));
    }

    #[test]
    fn a_phone_photo_turned_in_its_header_comes_in_upright() {
        use image::ImageEncoder;
        let dir = tempdir();
        let path = dir.path().join("phone.jpg");
        // A TIFF block with one entry: orientation 6, a quarter turn clockwise.
        let exif = [
            b"II*\0\x08\0\0\0\x01\0".as_slice(),
            &[0x12, 0x01, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0],
            &[0, 0, 0, 0],
        ]
        .concat();
        let mut bytes = Vec::new();
        let mut encoder = image::codecs::jpeg::JpegEncoder::new(&mut bytes);
        encoder.set_exif_metadata(exif).unwrap();
        encoder
            .write_image(&[0; 30 * 20 * 3], 30, 20, image::ExtendedColorType::Rgb8)
            .unwrap();
        std::fs::write(&path, bytes).unwrap();
        let img = load(&path).unwrap();
        assert_eq!((img.width, img.height), (20, 30));
    }

    #[test]
    fn missing_and_broken_files_are_errors() {
        let dir = tempdir();
        assert!(load(&dir.path().join("none.jpg")).is_err());
        let bad = dir.path().join("broken.jpg");
        std::fs::write(&bad, b"not a jpeg").unwrap();
        assert!(load(&bad).is_err());
        let raw = dir.path().join("broken.nef");
        std::fs::write(&raw, b"not a raw").unwrap();
        assert!(load(&raw).is_err());
    }

    #[test]
    fn a_display_p3_profile_is_converted_to_srgb() {
        use image::ImageEncoder;
        let Ok(icc) = std::fs::read("/System/Library/ColorSync/Profiles/Display P3.icc") else {
            return;
        };
        let dir = tempdir();
        let path = dir.path().join("p3.png");
        let mut bytes = Vec::new();
        let mut encoder = image::codecs::png::PngEncoder::new(&mut bytes);
        encoder.set_icc_profile(icc).unwrap();
        encoder
            .write_image(
                &[200, 100, 50].repeat(4),
                2,
                2,
                image::ExtendedColorType::Rgb8,
            )
            .unwrap();
        std::fs::write(&path, bytes).unwrap();
        let naive = [200.0, 100.0, 50.0].map(|v: f32| srgb_to_linear(v / 255.0));
        let p = load(&path).unwrap().pixels[0];
        // P3 red is redder than sRGB can show, so in sRGB it has more red and less green.
        assert!(p[0] > naive[0] + 0.01 && p[1] < naive[1], "{p:?} {naive:?}");
    }

    #[test]
    fn export_names_count_up_past_taken_ones() {
        let dir = tempdir();
        let photo = dir.path().join("photo.raf");
        let first = dir.path().join("photo_edit.jpg");
        let mut taken = HashSet::new();
        assert_eq!(export_path(&photo, &taken), first);
        std::fs::write(&first, b"").unwrap();
        assert_eq!(
            export_path(&photo, &taken),
            dir.path().join("photo_edit-2.jpg")
        );
        taken.insert(dir.path().join("photo_edit-2.jpg"));
        assert_eq!(
            export_path(&photo, &taken),
            dir.path().join("photo_edit-3.jpg")
        );
    }

    #[test]
    fn exports_are_jpeg_or_sixteen_bit_tiff() {
        let dir = tempdir();
        let img = Image::new(6, 4, vec![[0.5; 3]; 24]);
        let jpeg = dir.path().join("o.jpg");
        export(&img, &jpeg).unwrap();
        let back = image::open(&jpeg).unwrap();
        assert_eq!((back.width(), back.height()), (6, 4));
        let tiff = dir.path().join("o.TIF");
        export(&img, &tiff).unwrap();
        let back = image::open(&tiff).unwrap().into_rgb16();
        assert_eq!(back.get_pixel(0, 0).0, [32768; 3]);
    }

    #[test]
    fn downscaling_keeps_the_shape_and_leaves_small_pictures_alone() {
        let img = Image::new(600, 300, vec![[0.25; 3]; 600 * 300]);
        let small = downscale(img.clone(), 120);
        assert_eq!((small.width, small.height), (120, 60));
        assert!(near(small.pixels[0][0], 0.25, 1e-6));
        assert_eq!(downscale(img.clone(), 1000), img);
    }
}
