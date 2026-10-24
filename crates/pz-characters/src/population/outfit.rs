//! `Outfit` and `ClothingItemReference` (entries of
//! `media/clothing/clothing.xml`), their random draws (`Randomize`), and
//! the clothing choice `HumanVisual.dressInOutfit` makes from a
//! randomized outfit ([`dress_in_outfit`]).
//!
//! `ClothingItem` assets are `pz-inventory`'s; they are looked up by guid
//! through [`ClothingItemSource`].

use super::xml::{jaxb_float, Element};
use super::Population;
use pz_core::random::outfit_rng;
use pz_fs::java::is_null_or_whitespace;
use pz_inventory::visual::Rgb;
use pz_inventory::{ClothingItem, ClothingItems};
use std::borrow::Cow;

/// `ImmutableColor.white` without alpha.
pub const WHITE: Rgb = [1.0, 1.0, 1.0];

/// `OutfitManager.getClothingItem(guid)`.
pub trait ClothingItemSource {
    /// `None` when the guid names no clothing item; `Some(None)` for an
    /// asset that failed to load (present but not ready, with default
    /// fields).
    fn clothing_item(&self, guid: &str) -> Option<Option<&ClothingItem>>;
}

impl ClothingItemSource for ClothingItems {
    fn clothing_item(&self, guid: &str) -> Option<Option<&ClothingItem>> {
        self.get_by_guid(guid)
    }
}

/// What `Randomize` and `dressInOutfit` read besides the outfit.
pub struct OutfitContext<'a> {
    pub clothing_items: &'a dyn ClothingItemSource,
    /// `ClothingDecals`, `HairStyles`, `BeardStyles`,
    /// `HairOutfitDefinitions` (and `DefaultClothing` for the dress).
    pub population: &'a Population,
    /// `SurvivorDesc.HairCommonColors`.
    pub hair_common_colors: &'a [Rgb],
    /// `IsoWorld.getWorldAgeDays()`.
    pub world_age_days: f32,
    /// `SandboxOptions.noBlackClothes`.
    pub no_black_clothes: bool,
}

impl OutfitContext<'_> {
    /// A clothing item with failed assets as the defaults.
    fn item(&self, guid: Option<&str>) -> Option<Cow<'_, ClothingItem>> {
        match self.clothing_items.clothing_item(guid?)? {
            Some(i) => Some(Cow::Borrowed(i)),
            None => Some(Cow::Owned(ClothingItem::new(""))),
        }
    }

    /// A ready clothing item.
    fn ready_item(&self, guid: Option<&str>) -> Option<&ClothingItem> {
        self.clothing_items.clothing_item(guid?)?
    }
}

/// `ClothingItemReference.RandomData`.
#[derive(Clone, Debug, PartialEq)]
pub struct ClothingItemRandomData {
    pub active: bool,
    pub hue: f32,
    pub tint: Rgb,
    pub base_texture: Option<String>,
    /// `pickedItemRef.itemGuid`: `None` while `pickedItemRef` is null.
    pub picked_item_guid: Option<Option<String>>,
    pub texture_choice: Option<String>,
    pub decal: Option<String>,
}

impl Default for ClothingItemRandomData {
    fn default() -> Self {
        ClothingItemRandomData {
            active: true,
            hue: 0.0,
            tint: WHITE,
            base_texture: None,
            picked_item_guid: None,
            texture_choice: None,
            decal: None,
        }
    }
}

/// `ClothingItemReference`: a clothing item (by guid) with alternatives.
#[derive(Clone, Debug, PartialEq)]
pub struct ClothingItemReference {
    pub probability: f32,
    pub item_guid: Option<String>,
    pub sub_items: Vec<ClothingItemReference>,
    pub randomized: bool,
    pub immutable: bool,
    pub random_data: ClothingItemRandomData,
}

impl Default for ClothingItemReference {
    fn default() -> Self {
        ClothingItemReference {
            probability: 1.0,
            item_guid: None,
            sub_items: Vec::new(),
            randomized: false,
            immutable: false,
            random_data: ClothingItemRandomData::default(),
        }
    }
}

impl ClothingItemReference {
    pub fn from_element(e: &Element) -> Self {
        ClothingItemReference {
            probability: e
                .children_named("probability")
                .last()
                .map_or(1.0, |p| jaxb_float(&p.leaf_text())),
            item_guid: e.string_field("itemGUID"),
            sub_items: e
                .children_named("subItems")
                .map(Self::from_element)
                .collect(),
            randomized: e.bool_field("bRandomized").unwrap_or(false),
            ..Default::default()
        }
    }

    /// `setModID(modID)`: prefixes the guids (a `null` guid becomes
    /// `"<mod>-null"`, as Java's concatenation does).
    pub fn set_mod_id(&mut self, mod_id: &str) {
        self.item_guid = Some(format!(
            "{mod_id}-{}",
            self.item_guid.as_deref().unwrap_or("null")
        ));
        for s in &mut self.sub_items {
            s.set_mod_id(mod_id);
        }
    }

    /// `clone()`: probability, guid and cloned sub-items only.
    pub fn clone_ref(&self) -> Self {
        ClothingItemReference {
            probability: self.probability,
            item_guid: self.item_guid.clone(),
            sub_items: self.sub_items.iter().map(Self::clone_ref).collect(),
            ..Default::default()
        }
    }

    /// The guid `getClothingItem()` looks up: the picked alternative's.
    pub fn picked_guid(&self) -> Option<&str> {
        match &self.random_data.picked_item_guid {
            Some(g) => g.as_deref(),
            None => self.item_guid.as_deref(),
        }
    }

    /// `randomize()`.
    pub fn randomize(&mut self, ctx: &OutfitContext<'_>) -> Result<(), String> {
        if self.immutable {
            return Err("trying to randomize an immutable ClothingItemReference".into());
        }
        self.random_data = ClothingItemRandomData::default();
        for s in &mut self.sub_items {
            s.randomize(ctx)?;
        }
        // `pickRandomItemInternal`.
        self.random_data.picked_item_guid = Some(if self.sub_items.is_empty() {
            self.item_guid.clone()
        } else {
            match outfit_rng::next(self.sub_items.len() as i32 + 1) {
                0 => self.item_guid.clone(),
                i => self.sub_items[i as usize - 1]
                    .random_data
                    .picked_item_guid
                    .clone()
                    .flatten(),
            }
        });
        self.randomized = true;
        let Some(item) = ctx.item(self.picked_guid()) else {
            self.random_data.active = false;
            return Ok(());
        };
        let d = &mut self.random_data;
        d.active = outfit_rng::next_float(0.0, 1.0) <= self.probability;
        if item.allow_random_hue {
            d.hue = outfit_rng::next(200) as f32 / 100.0 - 1.0;
        }
        d.tint = if item.allow_random_tint {
            outfit_rng::random_immutable_color(ctx.no_black_clothes)
        } else {
            WHITE
        };
        d.base_texture = outfit_rng::pick_random(&item.base_textures).cloned();
        d.texture_choice = outfit_rng::pick_random(&item.texture_choices).cloned();
        if let Some(group) = item
            .decal_group
            .as_deref()
            .filter(|g| !is_null_or_whitespace(g))
        {
            d.decal = ctx
                .population
                .clothing_decals
                .as_ref()
                .and_then(|c| c.get_random_decal(group));
        }
        Ok(())
    }
}

/// `Outfit.RandomData`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OutfitRandomData {
    pub hair_color: Option<Rgb>,
    pub male_hair_name: Option<String>,
    pub female_hair_name: Option<String>,
    pub beard_name: Option<String>,
    pub top_tint: Option<Rgb>,
    pub pants_tint: Option<Rgb>,
    pub pants_hue: f32,
    pub has_top: bool,
    pub has_tshirt: bool,
    pub has_tshirt_decal: bool,
    pub top_texture: Option<String>,
    pub pants_texture: Option<String>,
}

/// `Outfit`.
#[derive(Clone, Debug, PartialEq)]
pub struct Outfit {
    pub name: String,
    pub top: bool,
    pub pants: bool,
    pub top_textures: Vec<String>,
    pub pants_textures: Vec<String>,
    pub items: Vec<ClothingItemReference>,
    pub allow_pants_hue: bool,
    pub allow_pants_tint: bool,
    pub allow_top_tint: bool,
    pub allow_tshirt_decal: bool,
    pub mod_id: Option<String>,
    pub immutable: bool,
    pub random_data: OutfitRandomData,
}

impl Default for Outfit {
    fn default() -> Self {
        Outfit {
            name: "Outfit".into(),
            top: true,
            pants: true,
            top_textures: Vec::new(),
            pants_textures: Vec::new(),
            items: Vec::new(),
            allow_pants_hue: true,
            allow_pants_tint: false,
            allow_top_tint: true,
            allow_tshirt_decal: true,
            mod_id: None,
            immutable: false,
            random_data: OutfitRandomData::default(),
        }
    }
}

impl Outfit {
    pub fn from_element(e: &Element) -> Self {
        let d = Outfit::default();
        Outfit {
            name: e.string_field("m_Name").unwrap_or(d.name),
            top: e.bool_field("m_Top").unwrap_or(d.top),
            pants: e.bool_field("m_Pants").unwrap_or(d.pants),
            top_textures: e.string_list("m_TopTextures"),
            pants_textures: e.string_list("m_PantsTextures"),
            items: e
                .children_named("m_items")
                .map(ClothingItemReference::from_element)
                .collect(),
            allow_pants_hue: e.bool_field("m_AllowPantsHue").unwrap_or(d.allow_pants_hue),
            allow_pants_tint: e
                .bool_field("m_AllowPantsTint")
                .unwrap_or(d.allow_pants_tint),
            allow_top_tint: e.bool_field("m_AllowTopTint").unwrap_or(d.allow_top_tint),
            allow_tshirt_decal: e
                .bool_field("m_AllowTShirtDecal")
                .unwrap_or(d.allow_tshirt_decal),
            ..d
        }
    }

    /// `setModID(modID)`.
    pub fn set_mod_id(&mut self, mod_id: &str) {
        self.mod_id = Some(mod_id.to_string());
        for i in &mut self.items {
            i.set_mod_id(mod_id);
        }
    }

    /// `clone()`: a mutable copy without mod id or random data.
    pub fn clone_outfit(&self) -> Outfit {
        Outfit {
            name: self.name.clone(),
            top: self.top,
            pants: self.pants,
            top_textures: self.top_textures.clone(),
            pants_textures: self.pants_textures.clone(),
            items: self
                .items
                .iter()
                .map(ClothingItemReference::clone_ref)
                .collect(),
            allow_pants_hue: self.allow_pants_hue,
            allow_pants_tint: self.allow_pants_tint,
            allow_top_tint: self.allow_top_tint,
            allow_tshirt_decal: self.allow_tshirt_decal,
            ..Outfit::default()
        }
    }

    /// `Randomize()`: every item, then hair colour, female hair, male
    /// hair, beard, top/pants tints, pants hue, top/t-shirt/decal flags and
    /// the top/pants textures. A missing hair/beard table or hair colour
    /// list leaves that field `None` (the original throws).
    pub fn randomize(&mut self, ctx: &OutfitContext<'_>) -> Result<(), String> {
        if self.immutable {
            return Err("trying to randomize an immutable Outfit".into());
        }
        for i in &mut self.items {
            i.randomize(ctx)?;
        }
        let pop = ctx.population;
        let defs = &pop.hair_outfit_definitions;
        let name = Some(self.name.as_str());
        let r = &mut self.random_data;
        r.hair_color = defs.get_random_haircut_color(name, ctx.hair_common_colors);
        r.female_hair_name = pop
            .hair_styles
            .as_ref()
            .map(|h| h.get_random_female_style(defs, name, ctx.world_age_days));
        r.male_hair_name = pop
            .hair_styles
            .as_ref()
            .map(|h| h.get_random_male_style(defs, name, ctx.world_age_days));
        r.beard_name = pop
            .beard_styles
            .as_ref()
            .and_then(|b| b.get_random_style(defs, name));
        r.top_tint = Some(outfit_rng::random_immutable_color(ctx.no_black_clothes));
        r.pants_tint = Some(outfit_rng::random_immutable_color(ctx.no_black_clothes));
        r.pants_hue = if outfit_rng::next(4) == 0 {
            outfit_rng::next(200) as f32 / 100.0 - 1.0
        } else {
            0.0
        };
        r.has_top = outfit_rng::next(16) != 0;
        r.has_tshirt = outfit_rng::next(2) == 0;
        r.has_tshirt_decal = outfit_rng::next(4) == 0;
        if self.top {
            r.has_top = true;
        }
        r.top_texture = outfit_rng::pick_random(&self.top_textures).cloned();
        r.pants_texture = outfit_rng::pick_random(&self.pants_textures).cloned();
        Ok(())
    }

    /// `GetMask().isTorsoVisible()`: no active item's masks cover
    /// `CharacterMask.Part.Torso` (1) or its subdivisions `Chest` (12) and
    /// `Waist` (13).
    pub fn is_torso_visible(&self, ctx: &OutfitContext<'_>) -> bool {
        let (mut chest, mut waist) = (true, true);
        for r in self.items.iter().filter(|r| r.random_data.active) {
            let Some(item) = ctx.item(r.picked_guid()) else {
                continue;
            };
            for &m in &item.masks {
                match m {
                    1 => (chest, waist) = (false, false),
                    12 => chest = false,
                    13 => waist = false,
                    _ => {}
                }
            }
        }
        chest && waist
    }

    /// `findItemByGUID(guid)`.
    pub fn find_item_by_guid(&self, guid: &str) -> Option<&ClothingItemReference> {
        self.items
            .iter()
            .find(|r| r.item_guid.as_deref() == Some(guid))
    }
}

/// One clothing item `dressInOutfit` adds (`addClothingItem`).
#[derive(Clone, Debug, PartialEq)]
pub struct DressItem {
    /// The clothing item name (`null` when `DefaultClothing` had nothing
    /// to pick).
    pub clothing_item_name: Option<String>,
    /// The index in `outfit.items` of the reference whose random data the
    /// item visual takes; `None` for the default pants/top. The original's
    /// `addClothingItem` adds nothing for a reference whose
    /// `random_data.active` is false.
    pub item_ref: Option<usize>,
    /// The `spawnWith` items added after it, which copy its visual; the
    /// original adds them only when adding this item succeeded.
    pub spawn_with: Vec<String>,
}

/// `HumanVisual.dressInOutfit`'s choices, in order: default pants, default
/// top (t-shirt, decal t-shirt or vest), then each ready item with its
/// `spawnWith` items. Clears `pants`, `top` and the top/pants textures
/// afterwards, as the original does.
pub fn dress_in_outfit(outfit: &mut Outfit, ctx: &OutfitContext<'_>) -> Vec<DressItem> {
    let defaults = &ctx.population.default_clothing;
    let mut out = Vec::new();
    let plain = |name: Option<String>| DressItem {
        clothing_item_name: name,
        item_ref: None,
        spawn_with: Vec::new(),
    };
    if outfit.pants {
        let name = if outfit.allow_pants_hue {
            defaults.pick_pants_hue()
        } else if outfit.allow_pants_tint {
            defaults.pick_pants_tint()
        } else {
            defaults.pick_pants_texture()
        };
        out.push(plain(name));
    }
    if outfit.top && outfit.random_data.has_top {
        let name = if outfit.random_data.has_tshirt {
            if outfit.random_data.has_tshirt_decal
                && outfit.is_torso_visible(ctx)
                && outfit.allow_tshirt_decal
            {
                if outfit.allow_top_tint {
                    defaults.pick_tshirt_decal_tint()
                } else {
                    defaults.pick_tshirt_decal_texture()
                }
            } else if outfit.allow_top_tint {
                defaults.pick_tshirt_tint()
            } else {
                defaults.pick_tshirt_texture()
            }
        } else if outfit.allow_top_tint {
            defaults.pick_vest_tint()
        } else {
            defaults.pick_vest_texture()
        };
        out.push(plain(name));
    }
    let mod_id = outfit.mod_id.as_deref().unwrap_or("game");
    for (i, r) in outfit.items.iter().enumerate() {
        let Some(item) = ctx.ready_item(r.picked_guid()) else {
            continue;
        };
        let spawn_with = item
            .spawn_with
            .iter()
            .filter_map(|s| {
                ctx.ready_item(Some(&format!("{mod_id}-{s}")))
                    .map(|w| w.name.clone())
            })
            .collect();
        out.push(DressItem {
            clothing_item_name: Some(item.name.clone()),
            item_ref: Some(i),
            spawn_with,
        });
    }
    outfit.pants = false;
    outfit.top = false;
    outfit.random_data.top_texture = None;
    outfit.random_data.pants_texture = None;
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::population::xml::parse_root;

    fn items() -> ClothingItems {
        let mut c = ClothingItems::default();
        let mut shirt = ClothingItem::new("Shirt");
        shirt.allow_random_tint = true;
        shirt.base_textures = vec!["a".into(), "b".into()];
        shirt.masks = vec![12];
        shirt.spawn_with = vec!["tie".into()];
        c.insert("Shirt", "game-shirt", Some(shirt));
        c.insert("Tie", "game-tie", Some(ClothingItem::new("Tie")));
        c.insert("Broken", "game-broken", None);
        c
    }

    #[test]
    fn binds_and_randomizes() {
        let doc = b"<outfitManager><m_MaleOutfits><m_Name>Test</m_Name><m_Top>false</m_Top><m_items><probability>0.5</probability>\
            <itemGUID>shirt</itemGUID><subItems><itemGUID>broken</itemGUID></subItems></m_items><m_items><itemGUID>none</itemGUID>\
            </m_items></m_MaleOutfits></outfitManager>";
        let root = parse_root(doc, "outfitManager").unwrap();
        let mut o = Outfit::from_element(root.children_named("m_MaleOutfits").next().unwrap());
        assert_eq!(o.items[0].probability, 0.5);
        o.set_mod_id("game");
        assert_eq!(
            o.items[0].sub_items[0].item_guid.as_deref(),
            Some("game-broken")
        );
        let o2 = o.clone_outfit();
        assert_eq!(o2.mod_id, None);
        let mut o = o2;
        let ci = items();
        let mut pop = Population::default();
        pop.default_clothing.pants.hue = vec!["Trousers".into()];
        let ctx = OutfitContext {
            clothing_items: &ci,
            population: &pop,
            hair_common_colors: &[],
            world_age_days: 0.0,
            no_black_clothes: false,
        };
        outfit_rng::set_seed(7);
        o.randomize(&ctx).unwrap();
        assert!(o.items.iter().all(|r| r.randomized));
        assert!(!o.items[1].random_data.active);
        assert!(o.random_data.top_tint.is_some());
        let dressed = dress_in_outfit(&mut o, &ctx);
        assert_eq!(dressed[0].clothing_item_name.as_deref(), Some("Trousers"));
        assert!(!o.pants && !o.top);
        let picked_shirt = o.items[0].picked_guid() == Some("game-shirt");
        assert_eq!(dressed.len(), 1 + usize::from(picked_shirt));
        if picked_shirt {
            assert_eq!(dressed[1].spawn_with, ["Tie"]);
            assert!(!o.is_torso_visible(&ctx) || !o.items[0].random_data.active);
        }
    }
}
