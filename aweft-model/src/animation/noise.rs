/// Minimal Perlin-noise FBM implementation for `FMod_Noise`.
///
/// Uses a simple hash-based approach rather than true gradient Perlin
/// to avoid external dependencies while still producing usable organic noise.

// ── hash permutation ─────────────────────────────────────────────────────

const PERM: [u8; 512] = {
    let p: [u8; 256] = [
        151, 160, 137, 91, 90, 15, 131, 13, 201, 95, 96, 53, 194, 233, 7, 225,
        140, 36, 103, 30, 69, 142, 8, 99, 37, 240, 21, 10, 23, 190, 6, 148,
        247, 120, 234, 75, 0, 26, 197, 62, 94, 252, 219, 203, 117, 35, 11, 32,
        57, 177, 33, 88, 237, 149, 56, 87, 174, 20, 125, 136, 171, 168, 68, 175,
        74, 165, 71, 134, 139, 48, 27, 166, 77, 146, 158, 231, 83, 111, 229, 122,
        60, 211, 133, 230, 220, 105, 92, 41, 55, 46, 245, 40, 244, 102, 143, 54,
        65, 25, 63, 161, 1, 216, 80, 73, 209, 76, 132, 187, 208, 89, 18, 169,
        200, 196, 135, 130, 116, 188, 159, 86, 164, 100, 109, 198, 173, 186, 3, 64,
        52, 217, 226, 250, 124, 123, 5, 202, 38, 147, 118, 126, 255, 82, 85, 212,
        207, 206, 59, 227, 47, 16, 58, 17, 182, 189, 28, 42, 223, 183, 170, 213,
        119, 248, 152, 2, 44, 154, 163, 70, 221, 153, 101, 155, 167, 43, 172, 9,
        129, 22, 39, 253, 19, 98, 108, 110, 79, 113, 224, 232, 178, 185, 112, 104,
        218, 246, 97, 228, 251, 34, 242, 193, 238, 210, 144, 12, 191, 179, 162, 241,
        81, 51, 145, 235, 249, 14, 239, 107, 49, 192, 214, 31, 181, 199, 106, 157,
        184, 84, 204, 176, 115, 121, 50, 45, 127, 4, 150, 254, 138, 236, 205, 93,
        222, 114, 67, 29, 24, 72, 243, 141, 128, 195, 78, 66, 215, 61, 156, 180,
    ];
    let mut out = [0u8; 512];
    let mut i = 0;
    while i < 256 {
        out[i] = p[i];
        out[i + 256] = p[i];
        i += 1;
    }
    out
};

fn hash(i: i32) -> u8 {
    PERM[(i as u8 as usize).wrapping_add(161)]
}

// ── gradient ──────────────────────────────────────────────────────────────

fn gradient(hash: u8, x: f64, y: f64) -> f64 {
    let h = hash & 7;
    let u = if h < 4 { x } else { y };
    let v = if h < 4 { y } else { x };
    let (a, b) = if h & 1 == 0 { (u, v) } else { (-u, -v) };
    if h & 2 == 0 { a } else { a + b }
}

// ── 2D Perlin ─────────────────────────────────────────────────────────────

fn perlin_noise(x: f64, y: f64) -> f64 {
    let xi = x.floor() as i32;
    let yi = y.floor() as i32;
    let xf = x - xi as f64;
    let yf = y - yi as f64;

    let u = fade(xf);
    let v = fade(yf);

    let aa = hash(hash(xi) as i32 + yi);
    let ab = hash(hash(xi) as i32 + yi + 1);
    let ba = hash(hash(xi + 1) as i32 + yi);
    let bb = hash(hash(xi + 1) as i32 + yi + 1);

    let x1 = lerp(gradient(aa, xf, yf), gradient(ba, xf - 1.0, yf), u);
    let x2 = lerp(gradient(ab, xf, yf - 1.0), gradient(bb, xf - 1.0, yf - 1.0), u);
    let value = lerp(x1, x2, v);

    // Normalize to roughly [0, 1]
    value * 0.5 + 0.5
}

fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

// ── Fractal Brownian Motion ───────────────────────────────────────────────

/// Compute FBM noise at (x, y) with given parameters.
///
/// Returns a value roughly in [0, 1] (or a bit beyond depending on normalization).
pub fn perlin_fbm(
    x: f64,
    y: f64,
    octaves: u8,
    roughness: f64,
    lacunarity: f64,
    normalize: bool,
) -> f64 {
    let mut value = 0.0f64;
    let mut amplitude = 1.0f64;
    let mut frequency = 1.0f64;
    let mut max_amp = 0.0f64;

    for _ in 0..octaves.max(1) {
        value += amplitude * perlin_noise(x * frequency, y * frequency);
        max_amp += amplitude;
        amplitude *= roughness;
        frequency *= lacunarity;
    }

    // Final octave with remaining amplitude
    if octaves == 0 {
        value = perlin_noise(x, y);
        max_amp = 1.0;
    }

    if normalize && max_amp > 0.0 {
        value / max_amp
    } else {
        value
    }
}
