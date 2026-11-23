//! IsoPuddlesCompute's ground interaction level, sampled at the square centre.
use super::PuddlesGeometry;
use pz_render::image::ImageData;

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a * (1.0 - t) + b * t
}

fn hash([x, y, z]: [f32; 3]) -> f32 {
    // The game's JOML Vector3f.dot uses right-associated products.
    let u = x * 17.1 + (y * 31.7 + z * 32.6);
    let v = x * 29.5 + (y * 13.3 + z * 42.6);
    let h = ((f64::from(u) * 2.1 + 1.1).sin() + (f64::from(v) * 2.5 + 1.5).sin()) as f32;
    h - h.floor()
}

fn noise(point: [f32; 3]) -> f32 {
    let [x, y, z] = point.map(f32::floor);
    let [fx, fy, fz] = point.map(|p| {
        let f = p - p.floor();
        f * f * (4.5 - 3.5 * f)
    });
    let row = |y, z| mix(hash([x, y, z]), hash([x + 1.0, y, z]), fx);
    let plane = |z| mix(row(y, z), row(y + 1.0, z), fy);
    mix(plane(z), plane(z + 1.0), fz) * 0.4
}

fn level(sample: f32, size: f32, exponent: f64) -> f32 {
    ((f64::from(sample).powf(exponent).min(1.0) + f64::from(sample)) as f32) * size - 0.24
}

pub fn directions(geometry: &PuddlesGeometry) -> [f32; 3] {
    [&geometry.pdne, &geometry.pdnw, &geometry.pda].map(|v| (v[0] + v[2]) * 0.5)
}

/// PerlinNoise uses the single-octave branch when computePuddle samples a square.
pub fn perlin_level(uv: [f32; 2], [ne, nw, all]: [f32; 3], size: f32) -> f32 {
    let [u, v] = uv.map(|p| p * 10.0);
    let mut scale = size * 1.02;
    for (weight, sign) in [(ne, 1.0), (nw, -1.0)] {
        let angle = (f64::from(u) + f64::from(v) * 2.0 * sign) * f64::from(3.1415f32);
        scale = (f64::from(scale) + f64::from(weight) * angle.sin() * angle.cos() * 2.0) as f32;
    }
    scale = (f64::from(scale) + f64::from(all) * 0.3) as f32;
    let a = (scale * noise([u, 0.0, v * 2.0])).min(0.7);
    let b = noise([u * 0.7, 1.0, v * 0.7]).min(0.7);
    let value = a + b;
    level(value * value, size, 0.3)
}

/// Low-quality puddles sample the red, green and blue channels of puddles_hm.
pub fn texture_level(
    uv: [f32; 2],
    [ne, nw, _]: [f32; 3],
    size: f32,
    image: &ImageData,
) -> Option<f32> {
    let [u, v] = uv.map(|p| (p * 0.5).rem_euclid(1.0));
    let x = (u * image.width as f32) as usize;
    let y = (v * image.height as f32) as usize;
    // ImageData.getPixelDiscard leaves all four components zero outside the backing image.
    if x >= image.width_hw as usize || y >= image.height_hw as usize {
        return Some(-0.24);
    }
    let offset = (x + y * image.width_hw as usize) * 4;
    let pixel = image.data.pixels.get(offset..offset + 3)?;
    let [r, g, b] = [pixel[0], pixel[1], pixel[2]].map(|c| f32::from(c) / 255.0);
    Some(level(mix(mix(b, r, ne), g, nw), size, f64::from(0.3f32)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interaction_levels_match_installed_game_methods() {
        for (uv, dir, size, expected) in [
            ([0., 0.], [0., 0., 1.], 1., 1.2653996),
            ([-13.7, 48.31], [1., 0., 0.], 0.66, 0.28904575),
            ([0.15, -0.27], [0., 1., 0.], 0.4, -0.026230365),
            ([19.31, -0.758], [0.5, 0.5, 0.5], 1., 1.2660447),
        ] {
            let result = perlin_level(uv, dir, size);
            assert!(
                (result - expected).abs() < 0.00001,
                "{uv:?}: {result} vs {expected}"
            );
        }
    }
}
