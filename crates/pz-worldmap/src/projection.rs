//! `zombie.worldMap.MapProjection`: the web-mercator zoom arithmetic the
//! map's zoom levels are expressed in.

pub const EARTH_RADIUS_METERS: f64 = 6378137.0;
pub const EARTH_HALF_CIRCUMFERENCE_METERS: f64 = 2.0037508342789244E7;
pub const EARTH_CIRCUMFERENCE_METERS: f64 = 4.007501668557849E7;
pub const MAX_LATITUDE_DEGREES: f64 = 85.05112878;

/// `metersPerTileAtZoom(zoom)`.
pub fn meters_per_tile_at_zoom(zoom: i32) -> f64 {
    EARTH_CIRCUMFERENCE_METERS / (1i32.wrapping_shl(zoom as u32)) as f64
}

/// `metersPerPixelAtZoom(zoom, tileSize)`.
pub fn meters_per_pixel_at_zoom(zoom: f64, tile_size: f64) -> f64 {
    EARTH_CIRCUMFERENCE_METERS / (exp2(zoom) * tile_size)
}

/// `zoomAtMetersPerPixel(metersPerPixel, tileSize)`.
pub fn zoom_at_meters_per_pixel(meters_per_pixel: f64, tile_size: f64) -> f64 {
    log2(EARTH_CIRCUMFERENCE_METERS / (meters_per_pixel * tile_size))
}

/// `exp2(n)`: `Math.pow(2, n)`.
pub fn exp2(n: f64) -> f64 {
    2f64.powf(n)
}

/// `log2(n)`: `Math.log(n) / Math.log(2)`.
pub fn log2(n: f64) -> f64 {
    n.ln() / 2f64.ln()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_round_trip() {
        let mpp = meters_per_pixel_at_zoom(14.5, 600.0);
        assert!((zoom_at_meters_per_pixel(mpp, 600.0) - 14.5).abs() < 1e-9);
        assert_eq!(meters_per_tile_at_zoom(1), EARTH_HALF_CIRCUMFERENCE_METERS);
    }
}
