//! The Lua object for `zombie.core.textures.Texture`, shared by every crate
//! that hands textures to Lua or receives them (`pz-ui`, `pz-lua-api`).

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

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
        format!(
            "Texture{{ name:\"{}\", w:{}, h:{} }}",
            t.name, t.width, t.height
        )
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

/// The actual mutable Texture allocation shared by scripts, items and Lua.
pub type TextureRef = Rc<LuaTexture>;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextureHostError {
    Unavailable(&'static str),
    Borrowed(&'static str),
}
impl std::fmt::Display for TextureHostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(name) => write!(f, "texture capability unavailable: {name}"),
            Self::Borrowed(name) => write!(f, "texture host borrowed: {name}"),
        }
    }
}
impl std::error::Error for TextureHostError {}
pub type TextureResult<T> = Result<T, TextureHostError>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureLookup {
    Try,
    Shared,
}
pub type TextureRefLoader = Rc<dyn Fn(TextureLookup, &str) -> TextureResult<Option<TextureRef>>>;
pub type TextureObserver = fn(&TextureRef);
thread_local! {
    static REF_LOADER: RefCell<Option<TextureRefLoader>> = const { RefCell::new(None) };
    static RETIRED_LOADERS: RefCell<Vec<TextureRefLoader>> = const { RefCell::new(Vec::new()) };
    static TEXTURE_OBSERVER: Cell<Option<TextureObserver>> = const { Cell::new(None) };
    // Native Texture.<clinit> sets this public static true. It is distinct
    // from FileIO logging conditions inside getSharedTextureInternal.
    static WARN_FAIL_FIND_TEXTURE: Cell<bool> = const { Cell::new(true) };
}
/// Returns displaced ownership after the provider slot guard has ended.
/// The caller retires it outside any enclosing script/world host guards.
pub fn install_texture_ref_loader(loader: Option<TextureRefLoader>) -> Option<TextureRefLoader> {
    REF_LOADER.with(|slot| std::mem::replace(&mut *slot.borrow_mut(), loader))
}
pub fn warn_fail_find_texture() -> bool {
    WARN_FAIL_FIND_TEXTURE.with(Cell::get)
}
pub fn set_warn_fail_find_texture(value: bool) {
    WARN_FAIL_FIND_TEXTURE.with(|flag| flag.set(value));
}
pub fn install_texture_observer(observer: Option<TextureObserver>) -> Option<TextureObserver> {
    TEXTURE_OBSERVER.with(|slot| slot.replace(observer))
}
pub fn observe_texture_ref(texture: &TextureRef) {
    let observer = TEXTURE_OBSERVER.with(Cell::get);
    if let Some(observer) = observer {
        observer(texture);
    }
}
fn capture_texture(lookup: TextureLookup, name: &str) -> TextureResult<Option<TextureRef>> {
    let loader = REF_LOADER
        .with(|slot| {
            slot.try_borrow()
                .map(|loader| loader.clone())
                .map_err(|_| TextureHostError::Borrowed("texture provider slot"))
        })?
        .ok_or(TextureHostError::Unavailable("texture provider"))?;
    // Never call a host while its provider registry is borrowed. Providers
    // similarly release FS/manager/cache guards before returning this handle.
    let result = loader(lookup, name);
    if let Ok(Some(texture)) = &result {
        observe_texture_ref(texture);
    }
    // A provider/observer may replace the installed provider while called.
    // Keep the last consumed host closure alive beyond an outer script/world
    // guard; ordinary current-provider captures retain their TLS owner.
    if Rc::strong_count(&loader) == 1 {
        RETIRED_LOADERS.with(|retired| retired.borrow_mut().push(loader));
    }
    result
}
/// Drain only at an outer host safe boundary, outside provider/script/world guards.
pub fn take_retired_texture_loaders() -> Vec<TextureRefLoader> {
    RETIRED_LOADERS.with(|retired| std::mem::take(&mut *retired.borrow_mut()))
}
pub fn try_get_texture_ref(name: &str) -> TextureResult<Option<TextureRef>> {
    capture_texture(TextureLookup::Try, name)
}
pub fn get_shared_texture_ref(name: &str) -> TextureResult<Option<TextureRef>> {
    capture_texture(TextureLookup::Shared, name)
}
/// A descriptor captured at a particular draw/write phase, not a second
/// persistent mutable Texture authority. Pointer getters do not need it.
pub fn try_texture_descriptor(texture: &TextureRef) -> TextureResult<Texture> {
    texture
        .0
        .try_borrow()
        .map(|body| body.clone())
        .map_err(|_| TextureHostError::Borrowed("actual texture body"))
}

#[cfg(test)]
mod tests;
