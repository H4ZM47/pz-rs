//! `ItemVisual` (also part of `HumanVisual`) and `HumanVisual`
//! ([`human`]).

pub mod human;
pub use human::HumanVisual;

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

/// `OutfitRNG` and `ClothingDecals.getRandomDecal`, which
/// `ItemVisual.pickUninitializedValues` draws from.
pub trait VisualRng {
    /// `OutfitRNG.Next(bound)`.
    fn next(&self, _bound: i32) -> i32 {
        0
    }
    /// `OutfitRNG.randomImmutableColor()`.
    fn random_color(&self) -> Rgb {
        [1.0, 1.0, 1.0]
    }
    /// `ClothingDecals.instance.getRandomDecal(group)`.
    fn random_decal(&self, _group: &str) -> Option<String> {
        None
    }
}

/// [`VisualRng`] with every default.
pub struct NoVisualRng;
impl VisualRng for NoVisualRng {}

const BLOOD: usize = 0;
const DIRT: usize = 1;
const HOLES: usize = 2;
const BASIC_PATCHES: usize = 3;
const DENIM_PATCHES: usize = 4;
const LEATHER_PATCHES: usize = 5;

impl ItemVisual {
    /// `new ItemVisual()` with `setItemType(fullType)`.
    pub fn new(full_type: &str) -> Self {
        ItemVisual {
            full_type: full_type.to_string(),
            alternate_model_name: String::new(),
            clothing_item_name: String::new(),
            tint: None,
            base_texture: -1,
            texture_choice: -1,
            hue: f32::INFINITY,
            decal: None,
            arrays: Default::default(),
        }
    }

    fn get(&self, array: usize, part: usize) -> f32 {
        self.arrays[array]
            .as_ref()
            .and_then(|a| a.get(part))
            .map_or(0.0, |&b| (b as u8) as f32 / 255.0)
    }

    fn set(&mut self, array: usize, part: usize, value: i8, max: usize) {
        let a = self.arrays[array].get_or_insert_with(|| vec![0; max]);
        if let Some(x) = a.get_mut(part) {
            *x = value;
        }
    }

    fn count(&self, array: usize) -> i32 {
        self.arrays[array]
            .as_ref()
            .map_or(0, |a| a.iter().filter(|&&b| b != 0).count() as i32)
    }

    /// `getBlood(part)`.
    pub fn get_blood(&self, part: usize) -> f32 {
        self.get(BLOOD, part)
    }

    /// `getDirt(part)`.
    pub fn get_dirt(&self, part: usize) -> f32 {
        self.get(DIRT, part)
    }

    /// `getHole(part)`.
    pub fn get_hole(&self, part: usize) -> f32 {
        self.get(HOLES, part)
    }

    /// `getBasicPatch(part)`.
    pub fn get_basic_patch(&self, part: usize) -> f32 {
        self.get(BASIC_PATCHES, part)
    }

    /// `getDenimPatch(part)`.
    pub fn get_denim_patch(&self, part: usize) -> f32 {
        self.get(DENIM_PATCHES, part)
    }

    /// `getLeatherPatch(part)`.
    pub fn get_leather_patch(&self, part: usize) -> f32 {
        self.get(LEATHER_PATCHES, part)
    }

    /// `setBlood(part, amount)` (`max`: `BloodBodyPartType.MAX.index()`).
    pub fn set_blood(&mut self, part: usize, amount: f32, max: usize) {
        let v = (amount.clamp(0.0, 1.0) * 255.0) as i32 as i8;
        self.set(BLOOD, part, v, max);
    }

    /// `setDirt(part, amount)`.
    pub fn set_dirt(&mut self, part: usize, amount: f32, max: usize) {
        let v = (amount.clamp(0.0, 1.0) * 255.0) as i32 as i8;
        self.set(DIRT, part, v, max);
    }

    /// `setHole(part)`.
    pub fn set_hole(&mut self, part: usize, max: usize) {
        self.set(HOLES, part, -1, max);
    }

    /// `setBasicPatch(part)`.
    pub fn set_basic_patch(&mut self, part: usize, max: usize) {
        self.set(BASIC_PATCHES, part, -1, max);
    }

    /// `setDenimPatch(part)`.
    pub fn set_denim_patch(&mut self, part: usize, max: usize) {
        self.set(DENIM_PATCHES, part, -1, max);
    }

    /// `setLeatherPatch(part)`.
    pub fn set_leather_patch(&mut self, part: usize, max: usize) {
        self.set(LEATHER_PATCHES, part, -1, max);
    }

    /// `removeHole(part)`.
    pub fn remove_hole(&mut self, part: usize) {
        if let Some(x) = self.arrays[HOLES].as_mut().and_then(|a| a.get_mut(part)) {
            *x = 0;
        }
    }

    /// `removePatch(part)`.
    pub fn remove_patch(&mut self, part: usize) {
        for a in [BASIC_PATCHES, DENIM_PATCHES, LEATHER_PATCHES] {
            if let Some(x) = self.arrays[a].as_mut().and_then(|a| a.get_mut(part)) {
                *x = 0;
            }
        }
    }

    /// `removeBlood()`.
    pub fn remove_blood(&mut self) {
        if let Some(a) = &mut self.arrays[BLOOD] {
            a.fill(0);
        }
    }

    /// `removeDirt()`.
    pub fn remove_dirt(&mut self) {
        if let Some(a) = &mut self.arrays[DIRT] {
            a.fill(0);
        }
    }

    /// `getHolesNumber()`.
    pub fn get_holes_number(&self) -> i32 {
        self.count(HOLES)
    }

    /// `getBasicPatchesNumber()`.
    pub fn get_basic_patches_number(&self) -> i32 {
        self.count(BASIC_PATCHES)
    }

    /// `getTotalBlood()`.
    pub fn get_total_blood(&self) -> f32 {
        self.arrays[BLOOD]
            .as_ref()
            .map_or(0.0, |a| a.iter().map(|&b| (b as u8) as f32 / 255.0).sum())
    }

    /// `setHue(hue)` (clamped to `[-1, 1]`).
    pub fn set_hue(&mut self, hue: f32) {
        self.hue = hue.clamp(-1.0, 1.0);
    }

    /// `pickUninitializedValues(clothingItem)` (`getHue`, `getTint`,
    /// `getBaseTexture`, `getTextureChoice`, `getDecal`). `custom_color`
    /// is the item's `isCustomColor()` (the tint then stays as is).
    pub fn pick_uninitialized_values(
        &mut self,
        c: &crate::assets::ClothingItem,
        custom_color: bool,
        rng: &dyn VisualRng,
    ) {
        if c.allow_random_hue {
            if self.hue == f32::INFINITY {
                self.hue = rng.next(200) as f32 / 100.0 - 1.0;
            }
        } else {
            self.hue = 0.0;
        }
        if !custom_color {
            if c.allow_random_tint {
                if self.tint.is_none() {
                    self.tint = Some(rng.random_color());
                }
            } else {
                self.tint = Some([1.0, 1.0, 1.0]);
            }
        }
        if c.base_textures.is_empty() {
            self.base_texture = -1;
        } else if self.base_texture < 0 || self.base_texture as usize >= c.base_textures.len() {
            self.base_texture = rng.next(c.base_textures.len() as i32) as i8;
        }
        if c.texture_choices.is_empty() {
            self.texture_choice = -1;
        } else if self.texture_choice < 0 || self.texture_choice as usize >= c.texture_choices.len()
        {
            self.texture_choice = rng.next(c.texture_choices.len() as i32) as i8;
        }
        match c.decal_group.as_deref() {
            Some(g) if !g.trim().is_empty() => {
                if self.decal.is_none() {
                    self.decal = rng.random_decal(g);
                }
            }
            _ => self.decal = None,
        }
    }
}
