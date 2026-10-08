//! White balance as a colour temperature in kelvin and a tint, as Lightroom shows it for raws.
//! The conversions are those of Adobe's DNG SDK: Robertson's method on the CIE 1960 uv diagram,
//! with a tint of one unit for 1/3000 of uv off the Planckian locus, positive towards magenta.

/// Kelvin and tint.
pub type White = [f32; 2];

pub const KELVIN_MIN: f32 = 2000.0;
pub const KELVIN_MAX: f32 = 50_000.0;
pub const TINT_RANGE: f32 = 150.0;
const TINT_SCALE: f64 = -3000.0;

/// Robertson's isotemperature lines: reciprocal megakelvin, u, v and the line's slope.
const ROBERTSON: [[f64; 4]; 31] = [
    [0.0, 0.18006, 0.26352, -0.24341],
    [10.0, 0.18066, 0.26589, -0.25479],
    [20.0, 0.18133, 0.26846, -0.26876],
    [30.0, 0.18208, 0.27119, -0.28539],
    [40.0, 0.18293, 0.27407, -0.30470],
    [50.0, 0.18388, 0.27709, -0.32675],
    [60.0, 0.18494, 0.28021, -0.35156],
    [70.0, 0.18611, 0.28342, -0.37915],
    [80.0, 0.18740, 0.28668, -0.40955],
    [90.0, 0.18880, 0.28997, -0.44278],
    [100.0, 0.19032, 0.29326, -0.47888],
    [125.0, 0.19462, 0.30141, -0.58204],
    [150.0, 0.19962, 0.30921, -0.70471],
    [175.0, 0.20525, 0.31647, -0.84901],
    [200.0, 0.21142, 0.32312, -1.0182],
    [225.0, 0.21807, 0.32909, -1.2168],
    [250.0, 0.22511, 0.33439, -1.4512],
    [275.0, 0.23247, 0.33904, -1.7298],
    [300.0, 0.24010, 0.34308, -2.0637],
    [325.0, 0.24792, 0.34655, -2.4681],
    [350.0, 0.25591, 0.34951, -2.9641],
    [375.0, 0.26400, 0.35200, -3.5814],
    [400.0, 0.27218, 0.35407, -4.3633],
    [425.0, 0.28039, 0.35577, -5.3762],
    [450.0, 0.28863, 0.35714, -6.7262],
    [475.0, 0.29685, 0.35823, -8.5955],
    [500.0, 0.30505, 0.35907, -11.324],
    [525.0, 0.31320, 0.35968, -15.628],
    [550.0, 0.32129, 0.36011, -23.325],
    [575.0, 0.32931, 0.36038, -40.770],
    [600.0, 0.33724, 0.36051, -116.45],
];

/// The unit vector along a slope.
fn direction(slope: f64) -> (f64, f64) {
    let len = (1.0 + slope * slope).sqrt();
    (1.0 / len, slope / len)
}

/// The chromaticity xy of a white of this temperature and tint.
pub fn to_xy([kelvin, tint]: White) -> [f64; 2] {
    let r = 1e6 / f64::from(kelvin);
    let offset = f64::from(tint) / TINT_SCALE;
    let i = (0..30).find(|&i| r < ROBERTSON[i + 1][0]).unwrap_or(29);
    let (a, b) = (ROBERTSON[i], ROBERTSON[i + 1]);
    let f = (b[0] - r) / (b[0] - a[0]);
    let mut u = a[1] * f + b[1] * (1.0 - f);
    let mut v = a[2] * f + b[2] * (1.0 - f);
    let (du1, dv1) = direction(a[3]);
    let (du2, dv2) = direction(b[3]);
    let (du, dv) = (du1 * f + du2 * (1.0 - f), dv1 * f + dv2 * (1.0 - f));
    let len = (du * du + dv * dv).sqrt();
    u += du / len * offset;
    v += dv / len * offset;
    let d = u - 4.0 * v + 2.0;
    [1.5 * u / d, v / d]
}

/// The temperature and tint of a white with chromaticity xy.
pub fn from_xy([x, y]: [f64; 2]) -> White {
    let d = 1.5 - x + 6.0 * y;
    let (u, v) = (2.0 * x / d, 3.0 * y / d);
    let mut last = (0.0, 0.0, 0.0);
    for i in 1..31 {
        let (du, dv) = direction(ROBERTSON[i][3]);
        let dt = -(u - ROBERTSON[i][1]) * dv + (v - ROBERTSON[i][2]) * du;
        if dt <= 0.0 || i == 30 {
            let dt = -dt.min(0.0);
            let f = if i == 1 { 0.0 } else { dt / (last.0 + dt) };
            let (a, b) = (ROBERTSON[i - 1], ROBERTSON[i]);
            let kelvin = 1e6 / (a[0] * f + b[0] * (1.0 - f));
            let uu = u - (a[1] * f + b[1] * (1.0 - f));
            let vv = v - (a[2] * f + b[2] * (1.0 - f));
            let (du, dv) = (du * (1.0 - f) + last.1 * f, dv * (1.0 - f) + last.2 * f);
            let len = (du * du + dv * dv).sqrt();
            let tint = (uu * du / len + vv * dv / len) * TINT_SCALE;
            return [kelvin as f32, tint as f32];
        }
        last = (dt, du, dv);
    }
    unreachable!("the loop returns at its last line")
}

type Matrix = [[f64; 3]; 3];

const SRGB_TO_XYZ: Matrix = [
    [0.4124564, 0.3575761, 0.1804375],
    [0.2126729, 0.7151522, 0.0721750],
    [0.0193339, 0.1191920, 0.9503041],
];
const BRADFORD: Matrix = [
    [0.8951, 0.2664, -0.1614],
    [-0.7502, 1.7135, 0.0367],
    [0.0389, -0.0685, 1.0296],
];

fn mul(a: &Matrix, b: &Matrix) -> Matrix {
    std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| a[i][k] * b[k][j]).sum()))
}

fn apply(m: &Matrix, v: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| (0..3).map(|k| m[i][k] * v[k]).sum())
}

fn invert(m: &Matrix) -> Option<Matrix> {
    let c = |i: usize, j: usize| {
        let (r0, r1) = ((i + 1) % 3, (i + 2) % 3);
        let (c0, c1) = ((j + 1) % 3, (j + 2) % 3);
        m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0]
    };
    let det: f64 = (0..3).map(|j| m[0][j] * c(0, j)).sum();
    (det.abs() > 1e-12).then(|| std::array::from_fn(|i| std::array::from_fn(|j| c(j, i) / det)))
}

fn xyz_of([x, y]: [f64; 2]) -> [f64; 3] {
    [x / y, 1.0, (1.0 - x - y) / y]
}

/// The linear sRGB matrix that rebalances a picture balanced for `shot` as if it had been balanced
/// for `wanted`: a Bradford adaptation from one white to the other, scaled to keep grey as bright.
pub fn rebalance(wanted: White, shot: White) -> [[f32; 3]; 3] {
    let cone = |white: White| apply(&BRADFORD, xyz_of(to_xy(white)));
    let (from, to) = (cone(wanted), cone(shot));
    let scale: Matrix = std::array::from_fn(|i| {
        std::array::from_fn(|j| if i == j { to[i] / from[i] } else { 0.0 })
    });
    let bradford_inv = invert(&BRADFORD).expect("invertible");
    let srgb_inv = invert(&SRGB_TO_XYZ).expect("invertible");
    let m = mul(
        &srgb_inv,
        &mul(&bradford_inv, &mul(&scale, &mul(&BRADFORD, &SRGB_TO_XYZ))),
    );
    let grey = apply(&m, [1.0; 3]);
    let luma = 0.2126 * grey[0] + 0.7152 * grey[1] + 0.0722 * grey[2];
    m.map(|row| row.map(|v| (v / luma) as f32))
}

/// The white a raw was shot with, from the camera's white balance multipliers and its matrix from
/// XYZ to camera colours. `None` when they do not give a plausible white.
pub fn as_shot(multipliers: [f32; 3], xyz_to_camera: &[f32]) -> Option<White> {
    let m: Matrix =
        std::array::from_fn(|i| std::array::from_fn(|j| f64::from(xyz_to_camera[i * 3 + j])));
    // The camera's reading of white, which its multipliers turn into equal channels.
    let neutral = multipliers.map(|k| 1.0 / f64::from(k));
    let [x, y, z] = apply(&invert(&m)?, neutral);
    let sum = x + y + z;
    let white = from_xy([x / sum, y / sum]);
    let plausible = white.iter().all(|v| v.is_finite())
        && (KELVIN_MIN..=KELVIN_MAX).contains(&white[0])
        && white[1].abs() <= TINT_RANGE;
    plausible.then_some(white)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_whites_have_their_known_temperatures() {
        // CIE illuminant A, a tungsten lamp, sits on the Planckian locus at 2856 K.
        let [k, tint] = from_xy([0.44757, 0.40745]);
        assert!((k - 2856.0).abs() < 15.0 && tint.abs() < 3.0, "{k} {tint}");
        // Daylight D65 is a little above the locus, a few units of tint towards green.
        let [k, tint] = from_xy([0.31271, 0.32902]);
        assert!((k - 6504.0).abs() < 30.0 && tint.abs() < 12.0, "{k} {tint}");
    }

    #[test]
    fn temperatures_and_tints_come_back() {
        for kelvin in [2000.0, 2856.0, 4000.0, 5500.0, 6500.0, 9000.0, 20_000.0] {
            for tint in [-100.0, -10.0, 0.0, 25.0, 150.0] {
                let [k, t] = from_xy(to_xy([kelvin, tint]));
                assert!((k / kelvin - 1.0).abs() < 1e-3, "{kelvin} {tint}: {k} {t}");
                assert!((t - tint).abs() < 0.5, "{kelvin} {tint}: {k} {t}");
            }
        }
    }

    #[test]
    fn the_same_white_changes_nothing() {
        let m = rebalance([5000.0, 10.0], [5000.0, 10.0]);
        for (i, row) in m.iter().enumerate() {
            for (j, v) in row.iter().enumerate() {
                assert!((v - if i == j { 1.0 } else { 0.0 }).abs() < 1e-5, "{m:?}");
            }
        }
    }

    #[test]
    fn a_higher_temperature_warms_and_a_higher_tint_turns_magenta() {
        let grey = |m: [[f32; 3]; 3]| m.map(|row| row.iter().sum::<f32>());
        let warm = grey(rebalance([7000.0, 0.0], [5000.0, 0.0]));
        assert!(warm[0] > warm[2], "{warm:?}");
        let magenta = grey(rebalance([5000.0, 40.0], [5000.0, 0.0]));
        assert!(
            magenta[1] < magenta[0] && magenta[1] < magenta[2],
            "{magenta:?}"
        );
    }

    #[test]
    fn the_camera_white_of_daylight_is_about_6500_k() {
        // With the sRGB matrix as the camera's, D65 white needs no multipliers.
        let xyz_to_srgb = invert(&SRGB_TO_XYZ).unwrap();
        let flat: Vec<f32> = xyz_to_srgb.iter().flatten().map(|&v| v as f32).collect();
        let [k, tint] = as_shot([1.0, 1.0, 1.0], &flat).unwrap();
        assert!((k - 6504.0).abs() < 50.0 && tint.abs() < 12.0, "{k} {tint}");
        let [warmer, _] = as_shot([1.0, 1.0, 2.0], &flat).unwrap();
        assert!(
            warmer < 6000.0,
            "boosting blue means the light was orange: {warmer}"
        );
    }
}
