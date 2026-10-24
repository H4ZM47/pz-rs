//! `zombie.core.skinnedmodel.visual.HumanVisual`: a character's or
//! corpse's skin, hair, beard, blood/dirt/holes per `BloodBodyPartType`
//! and body item visuals, with its save format.
//!
//! The style and skin lookups the original makes (`HairStyles`,
//! `BeardStyles`, `HairOutfitDefinitions`, `PopTemplateManager`,
//! `SurvivorDesc.getRandomSkinColor`, `OutfitRNG`) come through
//! [`HumanStyles`]; the owner's sex and zombie state through
//! [`HumanVisualOwner`]. Outfits are kept by name.

use super::{load_rgb, non_blank, save_rgb, ItemVisual, Rgb};
use crate::io::{Reader, SaveResult, Writer};

/// `IHumanVisual`: what the visual asks its owner.
pub trait HumanVisualOwner {
    fn is_female(&self) -> bool;
    fn is_zombie(&self) -> bool;
    fn is_skeleton(&self) -> bool;
}

/// The skin list `getSkinTexture` picks from (`PopTemplateManager`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkinList {
    Female,
    Male,
    SkeletonFemaleZombie,
    SkeletonMaleZombie,
    /// `femaleSkinsZombie1..3` (rot stage 1 to 3).
    FemaleZombie(i32),
    MaleZombie(i32),
}

/// Style tables and random draws `HumanVisual`'s getters use. The
/// defaults find no styles and draw 0.
pub trait HumanStyles {
    /// `OutfitRNG.Next(bound)`.
    fn next(&self, _bound: i32) -> i32 {
        0
    }
    /// `OutfitRNG.Next(0f, 1f)`.
    fn next_unit(&self) -> f32 {
        0.0
    }
    /// The `PopTemplateManager` skin texture list.
    fn skins(&self, _list: SkinList) -> &[String] {
        &[]
    }
    /// `HairStyles.FindFemaleStyle(name) != null` / `FindMaleStyle`.
    fn has_hair_style(&self, _female: bool, _name: Option<&str>) -> bool {
        false
    }
    /// `HairStyles.getRandomFemaleStyle(outfit)` / `getRandomMaleStyle`.
    fn random_hair_style(&self, _female: bool, _outfit: Option<&str>) -> Option<String> {
        None
    }
    /// `BeardStyles.FindStyle(name) != null`.
    fn has_beard_style(&self, _name: Option<&str>) -> bool {
        false
    }
    /// `BeardStyles.getRandomStyle(outfit)`.
    fn random_beard_style(&self, _outfit: Option<&str>) -> Option<String> {
        None
    }
    /// `HairOutfitDefinitions.getRandomHaircutColor(outfit)`.
    fn random_haircut_color(&self, _outfit: Option<&str>) -> Option<Rgb> {
        None
    }
    /// `SurvivorDesc.getRandomSkinColor()`.
    fn random_skin_color(&self) -> Rgb {
        [1.0, 1.0, 1.0]
    }
    /// `IsoWorld.getWorldAgeDays()`.
    fn world_age_days(&self) -> f32 {
        0.0
    }
}

/// [`HumanStyles`] with every default.
pub struct NoStyles;
impl HumanStyles for NoStyles {}

const BLACK: Rgb = [0.0, 0.0, 0.0];

/// `HumanVisual`. Colours are `ImmutableColor`s without alpha (the save
/// format has none; loaded colours are opaque).
#[derive(Clone, Debug, PartialEq)]
pub struct HumanVisual {
    pub hair_color: Option<Rgb>,
    pub beard_color: Option<Rgb>,
    /// `ImmutableColor.white` after `clear()`; `None` makes
    /// `getSkinColor` draw a random skin colour.
    pub skin_color: Option<Rgb>,
    pub body_hair: i8,
    pub skin_texture: i8,
    pub zombie_rot_stage: i8,
    pub skin_texture_name: Option<String>,
    pub beard_model: Option<String>,
    pub hair_model: Option<String>,
    /// `BloodBodyPartType.MAX.index()` entries each.
    pub blood: Vec<i8>,
    pub dirt: Vec<i8>,
    pub holes: Vec<i8>,
    pub body_visuals: Vec<ItemVisual>,
    pub non_attached_hair: Option<String>,
    pub natural_hair_color: Option<Rgb>,
    pub natural_beard_color: Option<Rgb>,
    /// `outfit`, by name (not saved here).
    pub outfit: Option<String>,
    /// `forceModelScript` (not saved).
    pub force_model_script: Option<String>,
}

fn level(b: i8) -> f32 {
    (b as u8) as f32 / 255.0
}

fn to_byte(amount: f32) -> i8 {
    (amount.clamp(0.0, 1.0) * 255.0) as i32 as i8
}

impl HumanVisual {
    /// `new HumanVisual(owner)` (also the state after `clear()`, but for
    /// the outfit), with `parts` = `BloodBodyPartType.MAX.index()`.
    pub fn new(parts: usize) -> Self {
        HumanVisual {
            hair_color: None,
            beard_color: None,
            skin_color: Some([1.0, 1.0, 1.0]),
            body_hair: -1,
            skin_texture: -1,
            zombie_rot_stage: -1,
            skin_texture_name: None,
            beard_model: None,
            hair_model: None,
            blood: vec![0; parts],
            dirt: vec![0; parts],
            holes: vec![0; parts],
            body_visuals: Vec::new(),
            non_attached_hair: None,
            natural_hair_color: None,
            natural_beard_color: None,
            outfit: None,
            force_model_script: None,
        }
    }

    /// `load(input, worldVersion)` (`clear()` first).
    pub fn load(r: &mut Reader<'_>, parts: usize) -> SaveResult<Self> {
        let mut v = HumanVisual::new(parts);
        let f1 = r.i8()? as u8;
        if f1 & 4 != 0 {
            v.hair_color = Some(load_rgb(r)?);
        }
        if f1 & 2 != 0 {
            v.beard_color = Some(load_rgb(r)?);
        }
        if f1 & 8 != 0 {
            v.skin_color = Some(load_rgb(r)?);
        }
        v.body_hair = r.i8()?;
        v.skin_texture = r.i8()?;
        v.zombie_rot_stage = r.i8()?;
        if f1 & 0x40 != 0 {
            v.skin_texture_name = Some(r.string()?);
        }
        if f1 & 0x10 != 0 {
            v.beard_model = Some(r.string()?);
        }
        if f1 & 0x20 != 0 {
            v.hair_model = Some(r.string()?);
        }
        for arr in [&mut v.blood, &mut v.dirt, &mut v.holes] {
            let n = r.i8()?;
            for i in 0..n.max(0) as usize {
                let a = r.i8()?;
                if i < arr.len() {
                    arr[i] = a;
                }
            }
        }
        let n = r.i8()?;
        for _ in 0..n.max(0) {
            v.body_visuals.push(ItemVisual::load(r, parts)?);
        }
        v.non_attached_hair = non_blank(r.string()?);
        let f2 = r.i8()? as u8;
        if f2 & 4 != 0 {
            v.natural_hair_color = Some(load_rgb(r)?);
        }
        if f2 & 2 != 0 {
            v.natural_beard_color = Some(load_rgb(r)?);
        }
        Ok(v)
    }

    /// `save(output)`.
    pub fn save(&self, w: &mut Writer) {
        let mut f1: u8 = 0;
        if self.hair_color.is_some() {
            f1 |= 4;
        }
        if self.beard_color.is_some() {
            f1 |= 2;
        }
        if self.skin_color.is_some() {
            f1 |= 8;
        }
        if self.beard_model.is_some() {
            f1 |= 0x10;
        }
        if self.hair_model.is_some() {
            f1 |= 0x20;
        }
        if self.skin_texture_name.is_some() {
            f1 |= 0x40;
        }
        w.i8(f1 as i8);
        for c in [&self.hair_color, &self.beard_color, &self.skin_color]
            .into_iter()
            .flatten()
        {
            save_rgb(w, c);
        }
        w.i8(self.body_hair);
        w.i8(self.skin_texture);
        w.i8(self.zombie_rot_stage);
        for s in [&self.skin_texture_name, &self.beard_model, &self.hair_model]
            .into_iter()
            .flatten()
        {
            w.string(s);
        }
        for arr in [&self.blood, &self.dirt, &self.holes] {
            w.i8(arr.len() as i8);
            for &a in arr {
                w.i8(a);
            }
        }
        w.i8(self.body_visuals.len() as i8);
        for v in &self.body_visuals {
            v.save(w);
        }
        w.opt_string(self.non_attached_hair.as_deref());
        let mut f2: u8 = 0;
        if self.natural_hair_color.is_some() {
            f2 |= 4;
        }
        if self.natural_beard_color.is_some() {
            f2 |= 2;
        }
        w.i8(f2 as i8);
        for c in [&self.natural_hair_color, &self.natural_beard_color]
            .into_iter()
            .flatten()
        {
            save_rgb(w, c);
        }
    }

    /// `clear()` (the outfit stays).
    pub fn clear(&mut self) {
        let parts = self.blood.len();
        let outfit = self.outfit.take();
        *self = HumanVisual::new(parts);
        self.outfit = outfit;
    }

    /// `copyFrom(other)`: settles the other's lazy values first (which
    /// changes it, as in the original), then copies everything.
    pub fn copy_from(
        &mut self,
        other: &mut HumanVisual,
        owner: &dyn HumanVisualOwner,
        styles: &dyn HumanStyles,
    ) {
        other.get_hair_color(styles);
        other.get_natural_hair_color(styles);
        other.get_natural_beard_color(styles);
        other.get_hair_model(owner, styles);
        other.get_beard_model(owner, styles);
        other.get_skin_texture(owner, styles);
        let parts = self.blood.len();
        *self = other.clone();
        for a in [&mut self.blood, &mut self.dirt, &mut self.holes] {
            a.resize(parts, 0);
        }
    }

    // ----- Skin -----

    /// `setSkinColor(color)`.
    pub fn set_skin_color(&mut self, c: Option<Rgb>) {
        self.skin_color = c;
    }

    /// `getSkinColor()`: a random skin colour when unset.
    pub fn get_skin_color(&mut self, styles: &dyn HumanStyles) -> Rgb {
        *self
            .skin_color
            .get_or_insert_with(|| styles.random_skin_color())
    }

    /// `setBodyHairIndex` / `getBodyHairIndex`.
    pub fn set_body_hair_index(&mut self, i: i32) {
        self.body_hair = i as i8;
    }

    pub fn get_body_hair_index(&self) -> i32 {
        self.body_hair as i32
    }

    /// `setSkinTextureIndex` / `getSkinTextureIndex`.
    pub fn set_skin_texture_index(&mut self, i: i32) {
        self.skin_texture = i as i8;
    }

    pub fn get_skin_texture_index(&self) -> i32 {
        self.skin_texture as i32
    }

    /// `setSkinTextureName`.
    pub fn set_skin_texture_name(&mut self, name: Option<&str>) {
        self.skin_texture_name = name.map(str::to_string);
    }

    /// `lerp(start, end, delta)` (delta clamped to `[0, 1]`).
    pub fn lerp(start: f32, end: f32, delta: f32) -> f32 {
        let delta = delta.clamp(0.0, 1.0);
        start + (end - start) * delta
    }

    /// `pickRandomZombieRotStage()`.
    pub fn pick_random_zombie_rot_stage(styles: &dyn HumanStyles) -> i32 {
        let days = (styles.world_age_days() as i32).max(0);
        let (mut s1_second, mut s2_second) = (20.0f32, 30.0f32);
        if days >= 180 {
            s1_second = 0.0;
            s2_second = 10.0;
        }
        let delta = (days as f32 - 20.0) / 70.0;
        let c1 = Self::lerp(100.0, s1_second, delta);
        let c2 = Self::lerp(10.0, s2_second, delta);
        let roll = styles.next(100) as f32;
        if roll < c1 {
            1
        } else if roll < c2 + c1 {
            2
        } else {
            3
        }
    }

    /// `getSkinTexture()`: the set name, else an entry of the owner's skin
    /// list (a zombie draws its rot stage first). `None` when the list is
    /// empty (the original throws).
    pub fn get_skin_texture(
        &mut self,
        owner: &dyn HumanVisualOwner,
        styles: &dyn HumanStyles,
    ) -> Option<String> {
        if let Some(n) = &self.skin_texture_name {
            return Some(n.clone());
        }
        let female = owner.is_female();
        let mut body_hair = "";
        let list = if owner.is_zombie() && owner.is_skeleton() {
            if female {
                SkinList::SkeletonFemaleZombie
            } else {
                SkinList::SkeletonMaleZombie
            }
        } else if owner.is_zombie() {
            if !(1..=3).contains(&self.zombie_rot_stage) {
                self.zombie_rot_stage = Self::pick_random_zombie_rot_stage(styles) as i8;
            }
            let s = self.zombie_rot_stage as i32;
            if female {
                SkinList::FemaleZombie(s)
            } else {
                SkinList::MaleZombie(s)
            }
        } else {
            if !female && self.body_hair >= 0 {
                body_hair = "a";
            }
            if female {
                SkinList::Female
            } else {
                SkinList::Male
            }
        };
        let textures = styles.skins(list);
        let n = textures.len() as i32;
        let mut t = self.skin_texture as i32;
        if t == n {
            t -= 1;
        } else if t < 0 || t > n {
            t = styles.next(n);
        }
        self.skin_texture = t as i8;
        textures
            .get(t.max(0) as usize)
            .map(|s| format!("{s}{body_hair}"))
    }

    // ----- Hair and beard -----

    /// `setHairColor(color)`: a beard without colour takes the previous
    /// hair colour (`new ImmutableColor(null)` is opaque black).
    pub fn set_hair_color(&mut self, c: Option<Rgb>) {
        if self.beard_color.is_none() {
            self.beard_color = Some(self.hair_color.unwrap_or(BLACK));
        }
        self.hair_color = c;
    }

    /// `getHairColor()`: a random haircut colour for the outfit when unset.
    pub fn get_hair_color(&mut self, styles: &dyn HumanStyles) -> Option<Rgb> {
        if self.hair_color.is_none() {
            self.hair_color = styles.random_haircut_color(self.outfit.as_deref());
        }
        self.hair_color
    }

    /// `setBeardColor` / `getBeardColor` (the hair colour when unset).
    pub fn set_beard_color(&mut self, c: Option<Rgb>) {
        self.beard_color = c;
    }

    pub fn get_beard_color(&mut self, styles: &dyn HumanStyles) -> Option<Rgb> {
        if self.beard_color.is_none() {
            self.beard_color = self.get_hair_color(styles);
        }
        self.beard_color
    }

    /// `setNaturalHairColor` / `getNaturalHairColor` (the hair colour
    /// when unset).
    pub fn set_natural_hair_color(&mut self, c: Option<Rgb>) {
        self.natural_hair_color = c;
    }

    pub fn get_natural_hair_color(&mut self, styles: &dyn HumanStyles) -> Option<Rgb> {
        if self.natural_hair_color.is_none() {
            self.natural_hair_color = self.get_hair_color(styles);
        }
        self.natural_hair_color
    }

    /// `setNaturalBeardColor` / `getNaturalBeardColor` (the natural hair
    /// colour when unset).
    pub fn set_natural_beard_color(&mut self, c: Option<Rgb>) {
        self.natural_beard_color = c;
    }

    pub fn get_natural_beard_color(&mut self, styles: &dyn HumanStyles) -> Option<Rgb> {
        if self.natural_beard_color.is_none() {
            self.natural_beard_color = self.get_natural_hair_color(styles);
        }
        self.natural_beard_color
    }

    /// `setHairModel` / `getHairModel` (a random style of the owner's sex
    /// when the stored one is not a known style).
    pub fn set_hair_model(&mut self, m: Option<&str>) {
        self.hair_model = m.map(str::to_string);
    }

    pub fn get_hair_model(
        &mut self,
        owner: &dyn HumanVisualOwner,
        styles: &dyn HumanStyles,
    ) -> Option<String> {
        let female = owner.is_female();
        if !styles.has_hair_style(female, self.hair_model.as_deref()) {
            self.hair_model = styles.random_hair_style(female, self.outfit.as_deref());
        }
        self.hair_model.clone()
    }

    /// `setBeardModel` / `getBeardModel` (women have none; else a random
    /// style when the stored one is unknown).
    pub fn set_beard_model(&mut self, m: Option<&str>) {
        self.beard_model = m.map(str::to_string);
    }

    pub fn get_beard_model(
        &mut self,
        owner: &dyn HumanVisualOwner,
        styles: &dyn HumanStyles,
    ) -> Option<String> {
        if owner.is_female() {
            self.beard_model = None;
        } else if !styles.has_beard_style(self.beard_model.as_deref()) {
            self.beard_model = styles.random_beard_style(self.outfit.as_deref());
        }
        self.beard_model.clone()
    }

    /// `getNonAttachedHair` / `setNonAttachedHair` (blank becomes null).
    pub fn get_non_attached_hair(&self) -> Option<&str> {
        self.non_attached_hair.as_deref()
    }

    pub fn set_non_attached_hair(&mut self, h: Option<&str>) {
        self.non_attached_hair = h.and_then(|h| non_blank(h.to_string()));
    }

    /// `setForceModelScript`.
    pub fn set_force_model_script(&mut self, s: Option<&str>) {
        self.force_model_script = s.map(str::to_string);
    }

    // ----- Blood, dirt and holes (by `BloodBodyPartType` index) -----

    /// `setBlood(part, amount)`.
    pub fn set_blood(&mut self, part: usize, amount: f32) {
        self.blood[part] = to_byte(amount);
    }

    /// `getBlood(part)`.
    pub fn get_blood(&self, part: usize) -> f32 {
        level(self.blood[part])
    }

    /// `setDirt(part, amount)`.
    pub fn set_dirt(&mut self, part: usize, amount: f32) {
        self.dirt[part] = to_byte(amount);
    }

    /// `getDirt(part)`.
    pub fn get_dirt(&self, part: usize) -> f32 {
        level(self.dirt[part])
    }

    /// `setHole(part)`.
    pub fn set_hole(&mut self, part: usize) {
        self.holes[part] = -1;
    }

    /// `getHole(part)`.
    pub fn get_hole(&self, part: usize) -> f32 {
        level(self.holes[part])
    }

    /// `removeBlood()` / `removeDirt()`.
    pub fn remove_blood(&mut self) {
        self.blood.fill(0);
    }

    pub fn remove_dirt(&mut self) {
        self.dirt.fill(0);
    }

    /// `randomBlood()` / `randomDirt()`.
    pub fn random_blood(&mut self, styles: &dyn HumanStyles) {
        for i in 0..self.blood.len() {
            self.set_blood(i, styles.next_unit());
        }
    }

    pub fn random_dirt(&mut self, styles: &dyn HumanStyles) {
        for i in 0..self.dirt.len() {
            self.set_dirt(i, styles.next_unit());
        }
    }

    /// `getTotalBlood()`.
    pub fn get_total_blood(&self) -> f32 {
        self.blood.iter().fold(0.0, |a, &b| a + level(b))
    }

    // ----- Body visuals -----

    /// `addBodyVisualFromClothingItemName` with the lookups done: the
    /// script's full type for the clothing item; `None` when a body
    /// visual of that clothing item exists. (`ClothingItemReference.randomize`
    /// is not modelled: the new visual's values stay unset.)
    pub fn add_body_visual(
        &mut self,
        item_type: &str,
        clothing_item_name: &str,
    ) -> Option<&mut ItemVisual> {
        if clothing_item_name.trim().is_empty()
            || self
                .body_visuals
                .iter()
                .any(|v| v.clothing_item_name == clothing_item_name)
        {
            return None;
        }
        let mut v = ItemVisual::new(item_type);
        v.clothing_item_name = clothing_item_name.to_string();
        self.body_visuals.push(v);
        self.body_visuals.last_mut()
    }

    /// `removeBodyVisualFromItemType(itemType)`.
    pub fn remove_body_visual_from_item_type(&mut self, item_type: &str) -> Option<ItemVisual> {
        let i = self
            .body_visuals
            .iter()
            .position(|v| v.full_type == item_type)?;
        Some(self.body_visuals.remove(i))
    }

    /// `hasBodyVisualFromItemType(itemType)`.
    pub fn has_body_visual_from_item_type(&self, item_type: &str) -> bool {
        self.body_visuals.iter().any(|v| v.full_type == item_type)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Owner(bool, bool);
    impl HumanVisualOwner for Owner {
        fn is_female(&self) -> bool {
            self.0
        }
        fn is_zombie(&self) -> bool {
            self.1
        }
        fn is_skeleton(&self) -> bool {
            false
        }
    }

    struct Styles(Vec<String>);
    impl HumanStyles for Styles {
        fn next(&self, bound: i32) -> i32 {
            bound - 1
        }
        fn skins(&self, l: SkinList) -> &[String] {
            match l {
                SkinList::Male | SkinList::MaleZombie(1) => &self.0,
                _ => &[],
            }
        }
        fn has_hair_style(&self, _f: bool, n: Option<&str>) -> bool {
            n == Some("Bob")
        }
        fn random_hair_style(&self, _f: bool, outfit: Option<&str>) -> Option<String> {
            Some(format!("R{}", outfit.unwrap_or("")))
        }
        fn random_haircut_color(&self, _o: Option<&str>) -> Option<Rgb> {
            Some([0.5, 0.5, 0.5])
        }
    }

    #[test]
    fn round_trip_and_levels() {
        let mut v = HumanVisual::new(4);
        v.set_hair_color(Some([1.0, 0.0, 0.0]));
        assert_eq!(v.beard_color, Some(BLACK));
        v.hair_model = Some("Bob".into());
        v.set_blood(2, 0.5);
        v.set_hole(3);
        v.body_visuals.push(ItemVisual::new("Base.Shirt"));
        v.non_attached_hair = Some("x".into());
        let mut w = Writer::new();
        v.save(&mut w);
        let bytes = w.buf.clone();
        let back = HumanVisual::load(&mut Reader::new(&bytes), 4).unwrap();
        assert_eq!(back, v);
        assert_eq!(back.get_blood(2), 127.0 / 255.0);
        assert_eq!(back.get_hole(3), 1.0);
        assert!((back.get_total_blood() - 127.0 / 255.0).abs() < 1e-6);
        assert!(back.has_body_visual_from_item_type("Base.Shirt"));
    }

    #[test]
    fn lazy_getters() {
        let s = Styles(vec!["Skin1".into(), "Skin2".into()]);
        let mut v = HumanVisual::new(2);
        v.outfit = Some("Cop".into());
        assert_eq!(v.get_natural_beard_color(&s), Some([0.5, 0.5, 0.5]));
        assert_eq!(
            v.get_hair_model(&Owner(false, false), &s).as_deref(),
            Some("RCop")
        );
        v.hair_model = Some("Bob".into());
        assert_eq!(
            v.get_hair_model(&Owner(false, false), &s).as_deref(),
            Some("Bob")
        );
        v.beard_model = Some("Full".into());
        assert_eq!(v.get_beard_model(&Owner(true, false), &s), None);
        v.body_hair = 0;
        assert_eq!(
            v.get_skin_texture(&Owner(false, false), &s).as_deref(),
            Some("Skin2a")
        );
        assert_eq!(v.skin_texture, 1);
        // A zombie draws a rot stage (roll 99 at day 0: stage 1).
        let mut z = HumanVisual::new(2);
        assert_eq!(
            z.get_skin_texture(&Owner(false, true), &s).as_deref(),
            Some("Skin2")
        );
        assert_eq!(z.zombie_rot_stage, 1);
        assert_eq!(HumanVisual::lerp(1.0, 3.0, 2.0), 3.0);
        z.clear();
        assert_eq!(z.zombie_rot_stage, -1);
        assert!(z.add_body_visual("Base.A", "A").is_some());
        assert!(z.add_body_visual("Base.B", "A").is_none());
        assert!(z.remove_body_visual_from_item_type("Base.A").is_some());
    }
}
