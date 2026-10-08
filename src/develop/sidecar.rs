//! The edits of a photo kept beside it in a hidden XMP file, as Lightroom keeps its own, so opening
//! the photo again brings them back. The file is named after the whole photo name, so a raw and a
//! JPEG of the same shot keep separate edits.
//!
//! The values are tx's own, under its own namespace: Lightroom's sliders compute different things,
//! and passing these off as its `crs:` settings would show it a different picture.

use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
};

use super::{geometry::Aspect, pipeline::Settings};

const NAMESPACE: &str = "https://github.com/NikodemQ/explorer/ns/develop/1.0/";

/// `.<photo name>.xmp` beside the photo.
pub fn path_for(photo: &Path) -> PathBuf {
    let name = photo.file_name().unwrap_or_default().to_string_lossy();
    photo.with_file_name(format!(".{name}.xmp"))
}

/// The edits saved for `photo`, if there are any.
pub fn read(photo: &Path) -> Option<Settings> {
    std::fs::read_to_string(path_for(photo))
        .ok()
        .map(|text| from_xmp(&text))
}

/// Saves the edits of `photo`, or removes the file when there are none left to keep.
pub fn write(photo: &Path, settings: &Settings, unedited: &Settings) -> io::Result<()> {
    let path = path_for(photo);
    if settings == unedited {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    crate::save::write_file(&path, to_xmp(settings).as_bytes())
}

fn list(values: &[f32]) -> String {
    values
        .iter()
        .map(f32::to_string)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Every setting as an attribute. Numbers are written in Rust's shortest form that reads back exact.
fn to_xmp(s: &Settings) -> String {
    let mut fields = vec![
        ("Temperature", s.temp.to_string()),
        ("Tint", s.tint.to_string()),
        ("Exposure", s.exposure.to_string()),
        ("Contrast", s.contrast.to_string()),
        ("Highlights", s.highlights.to_string()),
        ("Shadows", s.shadows.to_string()),
        ("Whites", s.whites.to_string()),
        ("Blacks", s.blacks.to_string()),
        ("ToneCurve", list(&s.curve)),
        ("HueAdjustments", list(&s.hsl_h)),
        ("SaturationAdjustments", list(&s.hsl_s)),
        ("LuminanceAdjustments", list(&s.hsl_l)),
        ("Vibrance", s.vibrance.to_string()),
        ("Saturation", s.saturation.to_string()),
        ("Clarity", s.clarity.to_string()),
        ("Texture", s.texture.to_string()),
        ("Sharpness", s.sharpen.to_string()),
        ("SharpenRadius", s.sharpen_radius.to_string()),
        ("Rotation", s.rot90.to_string()),
        ("Straighten", s.straighten.to_string()),
        ("Crop", list(&s.crop)),
        ("Aspect", s.aspect.name()),
    ];
    if let Some([kelvin, tint]) = s.as_shot {
        fields.push(("AsShotTemperature", kelvin.to_string()));
        fields.push(("AsShotTint", tint.to_string()));
    }
    let attributes: String = fields
        .iter()
        .map(|(name, value)| format!("\n    tx:{name}=\"{value}\""))
        .collect();
    format!(
        r#"<x:xmpmeta xmlns:x="adobe:ns:meta/" x:xmptk="tx">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:tx="{NAMESPACE}"
    tx:Version="1"{attributes}/>
 </rdf:RDF>
</x:xmpmeta>
"#
    )
}

/// The `tx:` attributes in an XMP text, wherever they sit.
fn attributes(text: &str) -> HashMap<&str, &str> {
    let mut found = HashMap::new();
    let mut rest = text;
    while let Some(at) = rest.find("tx:") {
        rest = &rest[at + 3..];
        let Some(eq) = rest.find("=\"") else { break };
        let name = &rest[..eq];
        let value = &rest[eq + 2..];
        let Some(end) = value.find('"') else { break };
        if name.chars().all(|c| c.is_ascii_alphanumeric()) {
            found.insert(name, &value[..end]);
        }
        rest = &value[end..];
    }
    found
}

/// The settings in an XMP text. Anything missing or unreadable keeps its default.
fn from_xmp(text: &str) -> Settings {
    let found = attributes(text);
    let number = |name: &str| found.get(name).and_then(|v| v.parse::<f32>().ok());
    fn array<const N: usize>(value: Option<&&str>) -> Option<[f32; N]> {
        let values: Vec<f32> = value?
            .split_whitespace()
            .map(str::parse)
            .collect::<Result<_, _>>()
            .ok()?;
        values.try_into().ok()
    }
    let mut s = Settings::default();
    let numbers: [(&str, &mut f32); 15] = [
        ("Temperature", &mut s.temp),
        ("Tint", &mut s.tint),
        ("Exposure", &mut s.exposure),
        ("Contrast", &mut s.contrast),
        ("Highlights", &mut s.highlights),
        ("Shadows", &mut s.shadows),
        ("Whites", &mut s.whites),
        ("Blacks", &mut s.blacks),
        ("Vibrance", &mut s.vibrance),
        ("Saturation", &mut s.saturation),
        ("Clarity", &mut s.clarity),
        ("Texture", &mut s.texture),
        ("Sharpness", &mut s.sharpen),
        ("SharpenRadius", &mut s.sharpen_radius),
        ("Straighten", &mut s.straighten),
    ];
    for (name, slot) in numbers {
        if let Some(v) = number(name) {
            *slot = v;
        }
    }
    if let (Some(kelvin), Some(tint)) = (number("AsShotTemperature"), number("AsShotTint")) {
        s.as_shot = Some([kelvin, tint]);
    }
    if let Some(curve) = array(found.get("ToneCurve")) {
        s.curve = curve;
    }
    if let Some(v) = array(found.get("HueAdjustments")) {
        s.hsl_h = v;
    }
    if let Some(v) = array(found.get("SaturationAdjustments")) {
        s.hsl_s = v;
    }
    if let Some(v) = array(found.get("LuminanceAdjustments")) {
        s.hsl_l = v;
    }
    if let Some(crop) = array(found.get("Crop")) {
        s.crop = crop;
    }
    if let Some(turns) = found.get("Rotation").and_then(|v| v.parse::<u8>().ok()) {
        s.rot90 = turns % 4;
    }
    if let Some(aspect) = found.get("Aspect").and_then(|v| Aspect::parse(v)) {
        s.aspect = aspect;
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every field away from its default, so one left out of the file shows up.
    fn edited() -> Settings {
        Settings {
            temp: 4350.0,
            tint: -12.5,
            as_shot: Some([3134.4268, -13.218312]),
            exposure: 0.35,
            contrast: 20.0,
            highlights: -40.0,
            shadows: 30.0,
            whites: 5.0,
            blacks: -6.0,
            curve: [0.0, 20.0, 55.0, 80.0, 100.0],
            hsl_h: [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0],
            hsl_s: [-1.0; 8],
            hsl_l: [0.0, 0.0, 10.0, 0.0, 0.0, -20.0, 0.0, 0.0],
            vibrance: 15.0,
            saturation: -5.0,
            clarity: 25.0,
            texture: 10.0,
            sharpen: 40.0,
            sharpen_radius: 1.3,
            rot90: 3,
            straighten: -1.5,
            crop: [0.1, 0.05, 0.9, 0.95],
            aspect: Aspect::Ratio(3, 2),
        }
    }

    #[test]
    fn every_setting_comes_back_exactly() {
        let s = edited();
        assert_eq!(from_xmp(&to_xmp(&s)), s);
        let shifts = Settings { as_shot: None, ..s };
        assert_eq!(from_xmp(&to_xmp(&shifts)), shifts);
    }

    #[test]
    fn the_file_is_xmp_another_tool_can_read() {
        let text = to_xmp(&edited());
        assert!(text.starts_with("<x:xmpmeta xmlns:x=\"adobe:ns:meta/\""));
        assert!(text.contains(&format!("xmlns:tx=\"{NAMESPACE}\"")));
        assert!(text.contains("tx:Aspect=\"3:2\""));
        assert!(text.trim_end().ends_with("</x:xmpmeta>"));
    }

    #[test]
    fn missing_or_broken_values_keep_their_defaults() {
        let s = from_xmp(r#"<x tx:Exposure="1.5" tx:Contrast="lots" tx:Crop="0 0 1"/>"#);
        assert_eq!(s.exposure, 1.5);
        assert_eq!(s.contrast, 0.0);
        assert_eq!(s.crop, Settings::default().crop);
        assert_eq!(from_xmp("not xmp at all"), Settings::default());
    }

    #[test]
    fn the_file_is_hidden_beside_the_photo_and_goes_when_nothing_is_edited() {
        let dir = crate::testdir::tempdir();
        let photo = dir.path().join("DSCF7890.RAF");
        let sidecar = dir.path().join(".DSCF7890.RAF.xmp");
        assert_eq!(path_for(&photo), sidecar);
        assert_eq!(read(&photo), None);
        let unedited = Settings::default();
        write(&photo, &edited(), &unedited).unwrap();
        assert_eq!(read(&photo), Some(edited()));
        write(&photo, &unedited, &unedited).unwrap();
        assert!(!sidecar.exists());
        write(&photo, &unedited, &unedited).unwrap();
    }
}
