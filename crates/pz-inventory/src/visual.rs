//! `ItemVisual` (also part of `HumanVisual`).

use crate::io::{color_from_byte, color_to_byte, Reader, SaveResult, Writer};

/// An `ImmutableColor` built from bytes (components are `byte / 255f`).
pub type Rgb = [f32; 3];

pub fn load_rgb(r: &mut Reader<'_>) -> SaveResult<Rgb> {
    Ok([
        color_from_byte(r.i8()?),
        color_from_byte(r.i8()?),
        color_from_byte(r.i8()?),
    ])
}

pub fn save_rgb(w: &mut Writer, c: &Rgb) {
    for &v in c {
        w.i8(color_to_byte(v));
    }
}

/// `StringUtils.isNullOrWhitespace` folded to `None`.
pub fn non_blank(s: String) -> Option<String> {
    if s.trim().is_empty() {
        None
    } else {
        Some(s)
    }
}

/// `ItemVisual`.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemVisual {
    pub full_type: String,
    pub alternate_model_name: String,
    pub clothing_item_name: String,
    pub tint: Option<Rgb>,
    pub base_texture: i8,
    pub texture_choice: i8,
    pub hue: f32,
    pub decal: Option<String>,
    /// `blood`, `dirt`, `holes`, `basicPatches`, `denimPatches`,
    /// `leatherPatches`: `None` until a non-empty array is read.
    pub arrays: [Option<Vec<i8>>; 6],
}

impl ItemVisual {
    pub fn load(r: &mut Reader<'_>, blood_body_parts: usize) -> SaveResult<Self> {
        let f1 = r.i8()? as u8;
        let mut v = ItemVisual {
            full_type: r.string()?,
            alternate_model_name: r.string()?,
            clothing_item_name: r.string()?,
            tint: None,
            base_texture: -1,
            texture_choice: -1,
            hue: f32::INFINITY,
            decal: None,
            arrays: Default::default(),
        };
        if f1 & 1 != 0 {
            v.tint = Some(load_rgb(r)?);
        }
        if f1 & 2 != 0 {
            v.base_texture = r.i8()?;
        }
        if f1 & 4 != 0 {
            v.texture_choice = r.i8()?;
        }
        if f1 & 8 != 0 {
            v.hue = r.f32()?;
        }
        if f1 & 0x10 != 0 {
            v.decal = Some(r.string()?);
        }
        for arr in v.arrays.iter_mut() {
            let n = r.i8()?;
            if n > 0 && arr.is_none() {
                *arr = Some(vec![0; blood_body_parts]);
            }
            for i in 0..n.max(0) as usize {
                let a = r.i8()?;
                if let Some(arr) = arr {
                    if i < arr.len() {
                        arr[i] = a;
                    }
                }
            }
        }
        Ok(v)
    }

    pub fn save(&self, w: &mut Writer) {
        let decal = self.decal.as_deref().filter(|d| !d.trim().is_empty());
        let mut f1: u8 = 0;
        if self.tint.is_some() {
            f1 |= 1;
        }
        if self.base_texture != -1 {
            f1 |= 2;
        }
        if self.texture_choice != -1 {
            f1 |= 4;
        }
        if self.hue != f32::INFINITY {
            f1 |= 8;
        }
        if decal.is_some() {
            f1 |= 0x10;
        }
        w.i8(f1 as i8);
        w.string(&self.full_type);
        w.string(&self.alternate_model_name);
        w.string(&self.clothing_item_name);
        if let Some(c) = &self.tint {
            save_rgb(w, c);
        }
        if self.base_texture != -1 {
            w.i8(self.base_texture);
        }
        if self.texture_choice != -1 {
            w.i8(self.texture_choice);
        }
        if self.hue != f32::INFINITY {
            w.f32(self.hue);
        }
        if let Some(d) = decal {
            w.string(d);
        }
        for arr in &self.arrays {
            match arr {
                Some(a) => {
                    w.i8(a.len() as i8);
                    for &x in a {
                        w.i8(x);
                    }
                }
                None => w.i8(0),
            }
        }
    }
}
