//! The Lua object for `zombie.core.textures.Texture`, shared by every crate
//! that hands textures to Lua or receives them (`pz-ui`, `pz-lua-api`).

use std::any::Any;
use std::cell::RefCell;

use pz_kahlua::LuaObject;

use crate::Texture;

pub const TEXTURE_CLASS: &str = "zombie.core.textures.Texture";

/// A [`Texture`] as Lua sees it. Setters such as `setOffsetX` mutate it in
/// place, as on the Java object.
#[derive(Debug)]
pub struct LuaTexture(pub RefCell<Texture>);

impl LuaTexture {
    pub fn new(t: Texture) -> Self {
        LuaTexture(RefCell::new(t))
    }

    /// A copy of the texture.
    pub fn get(&self) -> Texture {
        self.0.borrow().clone()
    }
}

impl LuaObject for LuaTexture {
    fn class_name(&self) -> &str {
        TEXTURE_CLASS
    }
    fn to_lua_string(&self) -> String {
        // `Texture.toString()`.
        let t = self.0.borrow();
        format!("Texture{{ name:\"{}\", w:{}, h:{} }}", t.name, t.width, t.height)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn super_classes(&self) -> &[&'static str] {
        &["zombie.asset.Asset"]
    }
}
