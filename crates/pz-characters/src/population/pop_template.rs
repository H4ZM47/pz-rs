//! `PopTemplateManager`'s skin texture lists. The model assembly it also
//! does (`populateCharacterModelSlot`, `addClothingItem`, hidden/alt item
//! models) belongs to the renderer. TODO(ModelManager).

use pz_inventory::visual::human::SkinList;

/// `PopTemplateManager`'s skin lists, empty until [`Self::init`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PopTemplateManager {
    pub male_skins: Vec<String>,
    pub female_skins: Vec<String>,
    pub male_skins_zombie1: Vec<String>,
    pub female_skins_zombie1: Vec<String>,
    pub male_skins_zombie2: Vec<String>,
    pub female_skins_zombie2: Vec<String>,
    pub male_skins_zombie3: Vec<String>,
    pub female_skins_zombie3: Vec<String>,
    pub cow_skins: Vec<String>,
    pub rat_skins: Vec<String>,
    pub skeleton_male_skins_zombie: Vec<String>,
    pub skeleton_female_skins_zombie: Vec<String>,
}

impl PopTemplateManager {
    pub const SKELETON_BURNED_SKIN_INDEX: i32 = 0;
    pub const SKELETON_NORMAL_SKIN_INDEX: i32 = 1;
    pub const SKELETON_MUSCLE_SKIN_INDEX: i32 = 2;

    /// A manager after `init()`.
    pub fn initialized() -> Self {
        let mut p = Self::default();
        p.init();
        p
    }

    /// `init()`: appends the fixed texture names (calling it twice
    /// duplicates them, as in the original).
    pub fn init(&mut self) {
        for i in 1..=5 {
            self.male_skins.push(format!("MaleBody0{i}"));
        }
        for i in 1..=5 {
            self.female_skins.push(format!("FemaleBody0{i}"));
        }
        for i in 1..=4 {
            self.male_skins_zombie1.push(format!("M_ZedBody0{i}_level1"));
            self.female_skins_zombie1.push(format!("F_ZedBody0{i}_level1"));
            self.male_skins_zombie2.push(format!("M_ZedBody0{i}_level2"));
            self.female_skins_zombie2.push(format!("F_ZedBody0{i}_level2"));
            self.male_skins_zombie3.push(format!("M_ZedBody0{i}_level3"));
            self.female_skins_zombie3.push(format!("F_ZedBody0{i}_level3"));
        }
        for list in [&mut self.skeleton_male_skins_zombie, &mut self.skeleton_female_skins_zombie] {
            list.extend(["SkeletonBurned", "Skeleton", "SkeletonMuscle"].map(String::from));
        }
        self.cow_skins.extend(["Cow_Black", "Cow_Purple"].map(String::from));
        self.rat_skins.push("Rat".into());
    }

    /// The list `HumanVisual.getSkinTexture` picks from. A rot stage
    /// outside 1..=3 has no list here (the original keeps the living
    /// list); this returns an empty slice for it.
    pub fn skins(&self, list: SkinList) -> &[String] {
        match list {
            SkinList::Female => &self.female_skins,
            SkinList::Male => &self.male_skins,
            SkinList::SkeletonFemaleZombie => &self.skeleton_female_skins_zombie,
            SkinList::SkeletonMaleZombie => &self.skeleton_male_skins_zombie,
            SkinList::FemaleZombie(1) => &self.female_skins_zombie1,
            SkinList::FemaleZombie(2) => &self.female_skins_zombie2,
            SkinList::FemaleZombie(3) => &self.female_skins_zombie3,
            SkinList::MaleZombie(1) => &self.male_skins_zombie1,
            SkinList::MaleZombie(2) => &self.male_skins_zombie2,
            SkinList::MaleZombie(3) => &self.male_skins_zombie3,
            SkinList::FemaleZombie(_) | SkinList::MaleZombie(_) => &[],
        }
    }

    /// The texture at `index` of `list` (`textures.get(skinTexture)`), or
    /// `None` out of range. `HumanVisual::get_skin_texture` does the index
    /// fix-up and body-hair suffix.
    pub fn skin_texture(&self, index: i32, list: SkinList) -> Option<&str> {
        usize::try_from(index).ok().and_then(|i| self.skins(list).get(i)).map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists() {
        let p = PopTemplateManager::initialized();
        assert_eq!(p.skin_texture(0, SkinList::Male), Some("MaleBody01"));
        assert_eq!(p.skin_texture(4, SkinList::Female), Some("FemaleBody05"));
        assert_eq!(p.skin_texture(3, SkinList::FemaleZombie(2)), Some("F_ZedBody04_level2"));
        assert_eq!(p.skin_texture(PopTemplateManager::SKELETON_MUSCLE_SKIN_INDEX, SkinList::SkeletonMaleZombie), Some("SkeletonMuscle"));
        assert_eq!(p.skin_texture(5, SkinList::Male), None);
        assert_eq!(p.skin_texture(-1, SkinList::Male), None);
        assert!(p.skins(SkinList::MaleZombie(0)).is_empty());
    }
}
