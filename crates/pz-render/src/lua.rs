//! The Lua object for `zombie.core.textures.Texture`, shared by every crate
//! that hands textures to Lua or receives them (`pz-ui`, `pz-lua-api`).

use std::any::Any;
use std::cell::RefCell;

use pz_kahlua::LuaObject;

use crate::Texture;

pub const TEXTURE_CLASS: &str = "zombie.core.textures.Texture";
pub const VIDEO_TEXTURE_CLASS: &str = "zombie.core.textures.VideoTexture";

/// A [`Texture`] as Lua sees it. Setters such as `setOffsetX` mutate it in
/// place, as on the Java object. The second field is the Java class:
/// [`TEXTURE_CLASS`], or a subclass such as [`VIDEO_TEXTURE_CLASS`] that
/// adds methods of its own and inherits the `Texture` ones.
#[derive(Debug)]
pub struct LuaTexture(pub RefCell<Texture>, pub &'static str);

impl LuaTexture {
    pub fn new(t: Texture) -> Self {
        LuaTexture(RefCell::new(t), TEXTURE_CLASS)
    }

    /// A texture object of a `Texture` subclass.
    pub fn with_class(t: Texture, class: &'static str) -> Self {
        LuaTexture(RefCell::new(t), class)
    }

    /// A copy of the texture.
    pub fn get(&self) -> Texture {
        self.0.borrow().clone()
    }
}

impl LuaObject for LuaTexture {
    fn class_name(&self) -> &str {
        self.1
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
        if self.1 == TEXTURE_CLASS {
            &["zombie.asset.Asset"]
        } else {
            &[TEXTURE_CLASS, "zombie.asset.Asset"]
        }
    }
}

type SharedLoader = Box<dyn FnMut(&str) -> Option<Texture>>;

thread_local! {
    static LOADER: RefCell<Option<SharedLoader>> = RefCell::new(None);
}

/// Installs the `Texture.getSharedTexture` used by code without a
/// [`crate::TextureManager`] at hand (the Java method is static).
pub fn install_shared_texture_loader(f: impl FnMut(&str) -> Option<Texture> + 'static) {
    LOADER.with(|l| *l.borrow_mut() = Some(Box::new(f)));
}

/// `Texture.getSharedTexture(name)` through the installed loader.
pub fn get_shared_texture(name: &str) -> Option<Texture> {
    LOADER.with(|l| l.borrow_mut().as_mut().and_then(|f| f(name)))
}
