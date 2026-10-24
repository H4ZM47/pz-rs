//! `zombie.core.utils.Bits` packing used by the erosion save data.

pub use pz_world::save::io::unpack_unit as unpack_byte_to_float_unit;

/// `Bits.packFloatUnitToByte` (release build: out-of-range values are
/// clamped instead of throwing).
pub fn pack_float_unit_to_byte(f: f32) -> i8 {
    let f = if !(0.0..=1.0).contains(&f) { f.clamp(0.0, 1.0) } else { f };
    // `(byte)(float)`: float -> int (truncation), then the low 8 bits.
    (f * 255.0 + -128.0) as i32 as i8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_unpack() {
        assert_eq!(pack_float_unit_to_byte(0.0), -128);
        assert_eq!(pack_float_unit_to_byte(1.0), 127);
        assert_eq!(pack_float_unit_to_byte(0.5), 0);
        assert_eq!(pack_float_unit_to_byte(2.0), 127);
        assert_eq!(unpack_byte_to_float_unit(-128), 0.0);
        assert_eq!(unpack_byte_to_float_unit(127), 1.0);
        for i in -128..=127i8 {
            assert_eq!(pack_float_unit_to_byte(unpack_byte_to_float_unit(i)), i);
        }
    }
}
