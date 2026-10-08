//! Times the develop pipeline on a preview-sized picture, one group of sliders at a time.
//! Run: cargo run --release --example developbench -- [width height]
use std::time::Instant;

use tx::develop::{
    Image,
    pipeline::{Settings, process},
};

fn main() {
    let mut args = std::env::args()
        .skip(1)
        .map(|a| a.parse::<usize>().unwrap());
    let (w, h) = (args.next().unwrap_or(1500), args.next().unwrap_or(1000));
    let mut seed = 1u32;
    let pixels = (0..w * h)
        .map(|i| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let n = (seed & 63) as f32 / 255.0;
            [
                (i % w) as f32 / w as f32 * 0.7 + n,
                (i / w) as f32 / h as f32 * 0.7 + n,
                n,
            ]
        })
        .collect();
    let img = Image::new(w, h, pixels);
    let d = Settings::default();
    let mut hsl = d;
    hsl.hsl_s[3] = -50.0;
    let cases = [
        ("defaults", d),
        (
            "basic",
            Settings {
                temp: 20.0,
                exposure: 0.5,
                contrast: 30.0,
                shadows: 40.0,
                vibrance: 20.0,
                ..d
            },
        ),
        (
            "curve",
            Settings {
                curve: [0.0, 20.0, 55.0, 80.0, 100.0],
                ..d
            },
        ),
        ("hsl", hsl),
        ("clarity", Settings { clarity: 50.0, ..d }),
        ("texture", Settings { texture: 50.0, ..d }),
        (
            "sharpen",
            Settings {
                sharpen: 80.0,
                sharpen_radius: 3.0,
                ..d
            },
        ),
        (
            "straighten",
            Settings {
                straighten: 5.0,
                ..d
            },
        ),
    ];
    println!("{w}x{h}");
    for (name, s) in cases {
        process(img.clone(), &s, 1.0);
        let start = Instant::now();
        const RUNS: u32 = 10;
        for _ in 0..RUNS {
            process(img.clone(), &s, 1.0);
        }
        println!("{name:>12}: {:?}", start.elapsed() / RUNS);
    }
}
