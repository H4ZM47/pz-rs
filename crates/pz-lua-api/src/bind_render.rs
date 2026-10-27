//! `zombie.core.textures.Texture` and the texture globals of
//! `LuaManager.GlobalObject` (`getTexture`, `tryGetTexture`).
//!
//! The original keeps its texture tables in statics; here they live in one
//! per-thread [`TextureManager`] that the client backend also reads (see
//! [`textures`]). The same texture is always the same Lua object, so Lua
//! `==` compares like Java references.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use pz_fs::GameFs;
use pz_kahlua::{NativeCall, Value};
use pz_render::{
    Texture, TextureHostError, TextureLookup, TextureManager, TextureRef, TextureResult,
};

use crate::expose::{expose_class, int_arg, str_arg, with_this};
use crate::LuaManager;

pub const TEXTURE: &str = "zombie.core.textures.Texture";
pub const ASSET: &str = "zombie.asset.Asset";

pub use pz_render::LuaTexture;

/// Texture identity: image id, region bits, name.
type TextureKey = (Option<u32>, [u32; 4], Rc<str>);

thread_local! {
    static TEXTURES: Rc<RefCell<TextureManager>> = Rc::new(RefCell::new(TextureManager::new()));
    /// Lua objects by texture identity (image id + region + name).
    static OBJECTS: RefCell<HashMap<TextureKey, TextureRef>> = RefCell::new(HashMap::new());
}

thread_local! {
    /// The file system `register` was given, for [`try_get_texture`].
    static FS: RefCell<Option<Rc<RefCell<GameFs>>>> = const { RefCell::new(None) };
}

/// Legacy descriptor projection; actual producers use typed render APIs.
pub fn try_get_texture(name: &str) -> Option<Texture> {
    pz_render::try_get_texture_ref(name)
        .expect("legacy texture projection requires installed unborrowed host")
        .map(|texture| {
            pz_render::try_texture_descriptor(&texture)
                .expect("legacy texture projection requires unborrowed body")
        })
}
/// Legacy descriptor projection; actual producers use typed render APIs.
pub fn get_shared_texture(name: &str) -> Option<Texture> {
    pz_render::get_shared_texture_ref(name)
        .expect("legacy texture projection requires installed unborrowed host")
        .map(|texture| {
            pz_render::try_texture_descriptor(&texture)
                .expect("legacy texture projection requires unborrowed body")
        })
}

/// The texture tables shared by Lua and the renderer.
pub fn textures() -> Rc<RefCell<TextureManager>> {
    TEXTURES.with(Rc::clone)
}

/// Canonical actual allocation. The key identifies the imported Texture
/// descriptor; later cache hits preserve mutations of its actual body.
fn cached_texture_ref(t: Option<Texture>) -> TextureResult<Option<TextureRef>> {
    let Some(t) = t else { return Ok(None) };
    let key = (t.id, t.uv.map(f32::to_bits), t.name.clone());
    OBJECTS.with(|objects| {
        let mut objects = objects
            .try_borrow_mut()
            .map_err(|_| TextureHostError::Borrowed("texture identity cache"))?;
        Ok(Some(
            objects
                .entry(key)
                .or_insert_with(|| Rc::new(LuaTexture::new(t)))
                .clone(),
        ))
    })
}
pub fn texture_ref(t: Option<Texture>) -> TextureResult<Option<TextureRef>> {
    let texture = cached_texture_ref(t)?;
    if let Some(texture) = &texture {
        pz_render::observe_texture_ref(texture);
    }
    Ok(texture)
}
pub fn texture_value_ref(texture: Option<&TextureRef>) -> Value {
    texture.map_or(Value::Nil, |texture| Value::Object(texture.clone()))
}
/// Legacy descriptor import. Actual item/script getters use texture_value_ref.
pub fn texture_value(t: Option<Texture>) -> Value {
    let texture = texture_ref(t).expect("unborrowed legacy texture descriptor import");
    texture_value_ref(texture.as_ref())
}
pub fn texture_error(error: TextureHostError) -> pz_kahlua::LuaError {
    let class = match error {
        TextureHostError::Borrowed(_) => "pz.host.Borrowed",
        TextureHostError::Unavailable(_) => "pz.host.Unsupported",
    };
    pz_kahlua::LuaError::Runtime {
        class,
        message: error.to_string(),
    }
}
/// Install before ScriptManager.Load, using the same FS and image manager
/// later used by Lua. All actual capture accesses are fallible.
pub fn install_texture_capture(fs: Rc<RefCell<GameFs>>) {
    // Metadata installation is available before the first Lua registry. The
    // weak pending observer retries registration rather than replacing assets.
    crate::component_gc::install_texture_observer();
    let old_fs = FS.with(|slot| std::mem::replace(&mut *slot.borrow_mut(), Some(fs.clone())));
    let provider: pz_render::TextureRefLoader = Rc::new(move |lookup, name| {
        let descriptor = {
            let fs = fs
                .try_borrow()
                .map_err(|_| TextureHostError::Borrowed("texture file system"))?;
            let manager = textures();
            let mut manager = manager
                .try_borrow_mut()
                .map_err(|_| TextureHostError::Borrowed("texture manager"))?;
            match lookup {
                TextureLookup::Try => manager.try_get_texture(&fs, name),
                TextureLookup::Shared => manager.shared_texture(&fs, name),
            }
        };
        cached_texture_ref(descriptor)
    });
    let old_provider = pz_render::install_texture_ref_loader(Some(provider));
    let mut retired = pz_entity::RetiredComponents::new();
    retired.push_payload(Box::new((old_fs, old_provider)));
    pz_inventory::component_host::retire_components(retired);
}
fn get<R>(c: &NativeCall<'_>, f: impl FnOnce(&Texture) -> R) -> pz_kahlua::LuaResult<R> {
    with_this::<LuaTexture, _>(c, |t| {
        t.0.try_borrow()
            .map(|texture| f(&texture))
            .map_err(|_| texture_error(TextureHostError::Borrowed("actual texture body")))
    })?
}

pub fn register(mgr: &mut LuaManager) {
    let fs = mgr.fs.clone();
    install_texture_capture(fs.clone());
    let f = fs.clone();
    // Existing descriptor-only UI consumers remain an explicit compatibility
    // boundary; new captured producers exclusively use the typed provider.
    pz_render::install_shared_texture_loader(move |name| {
        textures().borrow_mut().shared_texture(&f.borrow(), name)
    });
    let thread = &mut mgr.thread;

    let asset = expose_class(thread, ASSET, None);
    asset.method("isReady", |c| {
        let r = get(c, |t| t.ready)?;
        Ok(c.push(r))
    });
    asset.method("isFailure", |c| {
        let r = get(c, |t| !t.ready && t.id.is_some())?;
        Ok(c.push(r))
    });
    asset.method("isEmpty", |c| {
        let r = get(c, |t| !t.ready && t.id.is_none())?;
        Ok(c.push(r))
    });

    let b = expose_class(thread, TEXTURE, Some(ASSET));
    let f = fs.clone();
    b.static_fn("getSharedTexture", move |c| {
        let name = str_arg(c, 0).unwrap_or_else(|| Rc::from(""));
        let v = if c.nargs() >= 2 {
            let tm = textures();
            let t = {
                let fs = f.try_borrow().map_err(|_| {
                    texture_error(TextureHostError::Borrowed("texture file system"))
                })?;
                let mut manager = tm
                    .try_borrow_mut()
                    .map_err(|_| texture_error(TextureHostError::Borrowed("texture manager")))?;
                manager.shared_texture_flags(&fs, &name, int_arg(c, 1))
            };
            let texture = texture_ref(t).map_err(texture_error)?;
            texture_value_ref(texture.as_ref())
        } else {
            let texture = pz_render::get_shared_texture_ref(&name).map_err(texture_error)?;
            texture_value_ref(texture.as_ref())
        };
        Ok(c.push(v))
    });
    b.static_fn("trygetTexture", move |c| {
        let name = str_arg(c, 0).unwrap_or_else(|| Rc::from(""));
        let texture = pz_render::try_get_texture_ref(&name).map_err(texture_error)?;
        Ok(c.push(texture_value_ref(texture.as_ref())))
    });
    b.static_fn("getWhite", |c| {
        let manager = textures();
        let t = manager
            .try_borrow_mut()
            .map_err(|_| texture_error(TextureHostError::Borrowed("texture manager")))?
            .white();
        let texture = texture_ref(Some(t)).map_err(texture_error)?;
        Ok(c.push(texture_value_ref(texture.as_ref())))
    });
    b.static_fn("getErrorTexture", |c| {
        let manager = textures();
        let t = manager
            .try_borrow_mut()
            .map_err(|_| texture_error(TextureHostError::Borrowed("texture manager")))?
            .error_texture();
        let texture = texture_ref(Some(t)).map_err(texture_error)?;
        Ok(c.push(texture_value_ref(texture.as_ref())))
    });
    b.static_fn("onTexturePacksChanged", |_c| {
        textures()
            .try_borrow_mut()
            .map_err(|_| texture_error(TextureHostError::Borrowed("texture manager")))?
            .on_texture_packs_changed();
        Ok(0)
    });
    b.static_fn("processFilePath", |c| {
        let p = str_arg(c, 0).map(|s| s.replace('\\', "/"));
        Ok(c.push(p.map_or(Value::Nil, |s| Value::str(&s))))
    });

    b.method("getName", |c| {
        let n = get(c, |t| t.name.clone())?;
        Ok(c.push(Value::Str(n)))
    });
    b.method("getWidth", |c| {
        let n = get(c, |t| t.width as f64)?;
        Ok(c.push(n))
    });
    b.method("getHeight", |c| {
        let n = get(c, |t| t.height as f64)?;
        Ok(c.push(n))
    });
    b.method("getWidthOrig", |c| {
        let n = get(c, |t| f64::from(t.width_orig()))?;
        Ok(c.push(n))
    });
    b.method("getHeightOrig", |c| {
        let n = get(c, |t| f64::from(t.height_orig()))?;
        Ok(c.push(n))
    });
    b.method("getWidthHW", |c| {
        let n = get(c, |t| f64::from(t.width_hw))?;
        Ok(c.push(n))
    });
    b.method("getHeightHW", |c| {
        let n = get(c, |t| f64::from(t.height_hw))?;
        Ok(c.push(n))
    });
    b.method("getOffsetX", |c| {
        let n = get(c, |t| f64::from(t.offset_x))?;
        Ok(c.push(n))
    });
    b.method("getOffsetY", |c| {
        let n = get(c, |t| f64::from(t.offset_y))?;
        Ok(c.push(n))
    });
    b.method("getXStart", |c| {
        let n = get(c, |t| f64::from(t.x_start))?;
        Ok(c.push(n))
    });
    b.method("getYStart", |c| {
        let n = get(c, |t| f64::from(t.y_start))?;
        Ok(c.push(n))
    });
    b.method("getXEnd", |c| {
        let n = get(c, |t| f64::from(t.x_end))?;
        Ok(c.push(n))
    });
    b.method("getYEnd", |c| {
        let n = get(c, |t| f64::from(t.y_end))?;
        Ok(c.push(n))
    });
    b.method("isSolid", |c| {
        let n = get(c, |t| t.solid)?;
        Ok(c.push(n))
    });
    b.method("getUseAlphaChannel", |c| {
        let n = get(c, |t| !t.solid)?;
        Ok(c.push(n))
    });
    b.method("isMaskSet", |c| {
        let (x, y) = (int_arg(c, 1), int_arg(c, 2));
        let n = get(c, |t| t.is_mask_set(x, y))?;
        Ok(c.push(n))
    });
    b.method("isValid", |c| {
        let n = get(c, |t| t.ready)?;
        Ok(c.push(n))
    });
    b.method("setOffsetX", |c| {
        let v = int_arg(c, 1) as f32;
        with_this::<LuaTexture, _>(c, |t| {
            t.0.try_borrow_mut()
                .map(|mut texture| texture.offset_x = v)
                .map_err(|_| texture_error(TextureHostError::Borrowed("actual texture body")))
        })??;
        Ok(0)
    });
    b.method("setOffsetY", |c| {
        let v = int_arg(c, 1) as f32;
        with_this::<LuaTexture, _>(c, |t| {
            t.0.try_borrow_mut()
                .map(|mut texture| texture.offset_y = v)
                .map_err(|_| texture_error(TextureHostError::Borrowed("actual texture body")))
        })??;
        Ok(0)
    });
    b.method("toString", |c| {
        let s = get(c, |t| {
            format!(
                "Texture{{ name:\"{}\", w:{}, h:{} }}",
                t.name, t.width, t.height
            )
        })?;
        Ok(c.push(Value::str(&s)))
    });

    crate::expose::global(thread, "getTexture", move |c| {
        let name = str_arg(c, 0).unwrap_or_else(|| Rc::from(""));
        let texture = pz_render::get_shared_texture_ref(&name).map_err(texture_error)?;
        Ok(c.push(texture_value_ref(texture.as_ref())))
    });
    crate::expose::global(thread, "tryGetTexture", move |c| {
        let name = str_arg(c, 0).unwrap_or_else(|| Rc::from(""));
        let texture = pz_render::try_get_texture_ref(&name).map_err(texture_error)?;
        Ok(c.push(texture_value_ref(texture.as_ref())))
    });
}

#[cfg(test)]
#[path = "bind_render/capture_tests.rs"]
mod capture_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texture_globals_from_lua() {
        let dir = std::env::temp_dir().join(format!("pz-bind-render-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("media")).unwrap();
        let fs = Rc::new(RefCell::new(GameFs::new(&dir)));
        let mut mgr = LuaManager::init(fs).unwrap();
        let w = mgr
            .thread
            .globals()
            .get_str("Texture")
            .as_table()
            .unwrap()
            .get_str("getWhite");
        let a = mgr.thread.call(&w, &[]).unwrap();
        let b = mgr.thread.call(&w, &[]).unwrap();
        assert!(a[0].raw_equals(&b[0]), "same texture, same Lua object");
        let get = mgr.thread.globals().get_str("getTexture");
        let white = mgr
            .thread
            .call(&get, &[Value::str("media/ui/white.png")])
            .unwrap();
        assert!(white[0].raw_equals(&a[0]));
        let none = mgr
            .thread
            .call(&get, &[Value::str("media/ui/nothing.png")])
            .unwrap();
        assert!(none.first().is_none_or(Value::is_nil));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
