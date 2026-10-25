//! The data of `zombie.core.skinnedmodel.visual.HumanVisual` a
//! `SurvivorDesc` holds, and the colour values it uses.
//!
//! TODO(Outfits): outfits, clothing items, hair/beard style lookups
//! (`HairStyles`, `BeardStyles`, `HairOutfitDefinitions`), skin textures
//! (`PopTemplateManager`) and models are not ported; the getters that
//! consult them return the stored value.

/// `zombie.core.ImmutableColor` (values only).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImmutableColor {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl ImmutableColor {
    /// `ImmutableColor.white`.
    pub const WHITE: ImmutableColor = ImmutableColor { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };

    /// `new ImmutableColor(r, g, b, a)`: components capped at 1.
    pub fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        ImmutableColor { r: r.min(1.0), g: g.min(1.0), b: b.min(1.0), a: a.min(1.0) }
    }
}

/// `zombie.core.Color` (values only).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    /// `new Color(r, g, b, a)`: components clamped to `[0, 1]`.
    pub fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        let c = |v: f32| v.clamp(0.0, 1.0);
        Color { r: c(r), g: c(g), b: c(b), a: c(a) }
    }
}

/// `BloodBodyPartType.MAX.index()`.
pub const BLOOD_BODY_PART_COUNT: usize = 17;

/// One entry of `HumanVisual.bodyVisuals` (an `ItemVisual`): the parts the
/// item-type lookups compare.
#[derive(Clone, Debug, PartialEq)]
pub struct BodyVisual {
    pub item_type: String,
    pub clothing_item_name: String,
}

/// `HumanVisual`'s state.
#[derive(Clone, Debug, PartialEq)]
pub struct HumanVisual {
    /// `null` until set: `getSkinColor` then picks a random skin colour.
    /// TODO(OutfitRNG): `SurvivorDesc.getRandomSkinColor`.
    pub skin_color: Option<ImmutableColor>,
    pub skin_texture: i32,
    pub skin_texture_name: Option<String>,
    pub zombie_rot_stage: i32,
    pub hair_color: Option<ImmutableColor>,
    pub beard_color: Option<ImmutableColor>,
    pub natural_hair_color: Option<ImmutableColor>,
    pub natural_beard_color: Option<ImmutableColor>,
    pub hair_model: Option<String>,
    pub beard_model: Option<String>,
    pub body_hair: i32,
    pub blood: [u8; BLOOD_BODY_PART_COUNT],
    pub dirt: [u8; BLOOD_BODY_PART_COUNT],
    pub holes: [u8; BLOOD_BODY_PART_COUNT],
    pub body_visuals: Vec<BodyVisual>,
    /// TODO(Outfits): the outfit; its name here.
    pub outfit: Option<String>,
    pub non_attached_hair: Option<String>,
    pub force_model_script: Option<String>,
}

impl Default for HumanVisual {
    fn default() -> Self {
        HumanVisual {
            skin_color: Some(ImmutableColor::WHITE),
            skin_texture: -1,
            skin_texture_name: None,
            zombie_rot_stage: -1,
            hair_color: None,
            beard_color: None,
            natural_hair_color: None,
            natural_beard_color: None,
            hair_model: None,
            beard_model: None,
            body_hair: -1,
            blood: [0; BLOOD_BODY_PART_COUNT],
            dirt: [0; BLOOD_BODY_PART_COUNT],
            holes: [0; BLOOD_BODY_PART_COUNT],
            body_visuals: Vec::new(),
            outfit: None,
            non_attached_hair: None,
            force_model_script: None,
        }
    }
}

impl HumanVisual {
    /// `setHairColor`: a beard without colour takes a copy of the previous
    /// hair colour (opaque black if none).
    pub fn set_hair_color(&mut self, color: Option<ImmutableColor>) {
        if self.beard_color.is_none() {
            self.beard_color = Some(self.hair_color.unwrap_or(ImmutableColor { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }));
        }
        self.hair_color = color;
    }

    /// `setNonAttachedHair`: blank becomes `null`.
    pub fn set_non_attached_hair(&mut self, hair: Option<&str>) {
        self.non_attached_hair = (!pz_scripts::java::is_null_or_whitespace(hair)).then(|| hair.unwrap_or_default().to_string());
    }

    /// `lerp`.
    pub fn lerp(start: f32, end: f32, delta: f32) -> f32 {
        let delta = delta.clamp(0.0, 1.0);
        start + (end - start) * delta
    }

    /// `clear`.
    pub fn clear(&mut self) {
        *self = HumanVisual { outfit: self.outfit.take(), ..HumanVisual::default() };
    }

    /// `removeBodyVisualFromItemType`.
    pub fn remove_body_visual_from_item_type(&mut self, item_type: &str) -> Option<BodyVisual> {
        let i = self.body_visuals.iter().position(|v| v.item_type == item_type)?;
        Some(self.body_visuals.remove(i))
    }

    /// `hasBodyVisualFromItemType`.
    pub fn has_body_visual_from_item_type(&self, item_type: &str) -> bool {
        self.body_visuals.iter().any(|v| v.item_type == item_type)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_and_clear() {
        assert_eq!(ImmutableColor::new(2.0, 0.5, -1.0, 1.0), ImmutableColor { r: 1.0, g: 0.5, b: -1.0, a: 1.0 });
        assert_eq!(Color::new(2.0, 0.5, -1.0, 1.0), Color { r: 1.0, g: 0.5, b: 0.0, a: 1.0 });
        let mut v = HumanVisual::default();
        let red = ImmutableColor::new(1.0, 0.0, 0.0, 1.0);
        v.set_hair_color(Some(red));
        assert_eq!((v.hair_color, v.beard_color), (Some(red), Some(ImmutableColor::new(0.0, 0.0, 0.0, 1.0))));
        v.beard_color = None;
        v.set_hair_color(None);
        assert_eq!(v.beard_color, Some(red));
        v.body_hair = 0;
        v.set_non_attached_hair(Some("  "));
        assert!(v.non_attached_hair.is_none());
        v.body_visuals.push(BodyVisual { item_type: "M.A".into(), clothing_item_name: "A".into() });
        assert!(v.has_body_visual_from_item_type("M.A"));
        v.outfit = Some("O".into());
        v.clear();
        assert_eq!(v.outfit.as_deref(), Some("O"));
        assert_eq!(v.body_hair, -1);
        assert!(v.body_visuals.is_empty());
        assert_eq!(HumanVisual::lerp(1.0, 3.0, 2.0), 3.0);
    }
}
