//! `DefaultClothing`: the clothing item names `dressInOutfit` picks for an
//! outfit's default pants and top, from Lua's `DefaultClothing` table
//! (`Pants`, `TShirt`, `TShirtDecal`, `Vest`, each with `hue`, `texture`
//! and `tint` lists). The binding fills it on `checkDirty`.

use pz_core::random::outfit_rng;

/// One `DefaultClothing.Clothing`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DefaultClothingLists {
    pub hue: Vec<String>,
    pub texture: Vec<String>,
    pub tint: Vec<String>,
}

/// `DefaultClothing`.
#[derive(Clone, Debug, PartialEq)]
pub struct DefaultClothing {
    pub pants: DefaultClothingLists,
    pub tshirt: DefaultClothingLists,
    pub tshirt_decal: DefaultClothingLists,
    pub vest: DefaultClothingLists,
    /// Set when the Lua table may have changed; the owner reloads before
    /// the next pick.
    pub dirty: bool,
}

impl Default for DefaultClothing {
    fn default() -> Self {
        DefaultClothing {
            pants: Default::default(),
            tshirt: Default::default(),
            tshirt_decal: Default::default(),
            vest: Default::default(),
            dirty: true,
        }
    }
}

fn pick(list: &[String]) -> Option<String> {
    outfit_rng::pick_random(list).cloned()
}

impl DefaultClothing {
    pub fn pick_pants_hue(&self) -> Option<String> {
        pick(&self.pants.hue)
    }
    pub fn pick_pants_texture(&self) -> Option<String> {
        pick(&self.pants.texture)
    }
    pub fn pick_pants_tint(&self) -> Option<String> {
        pick(&self.pants.tint)
    }
    pub fn pick_tshirt_texture(&self) -> Option<String> {
        pick(&self.tshirt.texture)
    }
    pub fn pick_tshirt_tint(&self) -> Option<String> {
        pick(&self.tshirt.tint)
    }
    pub fn pick_tshirt_decal_texture(&self) -> Option<String> {
        pick(&self.tshirt_decal.texture)
    }
    pub fn pick_tshirt_decal_tint(&self) -> Option<String> {
        pick(&self.tshirt_decal.tint)
    }
    pub fn pick_vest_texture(&self) -> Option<String> {
        pick(&self.vest.texture)
    }
    pub fn pick_vest_tint(&self) -> Option<String> {
        pick(&self.vest.tint)
    }
}
