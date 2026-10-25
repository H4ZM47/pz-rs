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
use pz_kahlua::value::LuaObject;
use pz_kahlua::{NativeCall, Value};
use pz_render::{Texture, TextureManager};

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
    static OBJECTS: RefCell<HashMap<TextureKey, Value>> = RefCell::new(HashMap::new());
}

/// The texture tables shared by Lua and the renderer.
pub fn textures() -> Rc<RefCell<TextureManager>> {
    TEXTURES.with(Rc::clone)
}

/// The Lua object for `t`, created once per texture.
pub fn texture_value(t: Option<Texture>) -> Value {
    let Some(t) = t else { return Value::Nil };
    let key = (t.id, t.uv.map(f32::to_bits), t.name.clone());
    OBJECTS.with(|o| o.borrow_mut().entry(key).or_insert_with(|| Value::Object(Rc::new(LuaTexture::new(t)))).clone())
}

fn shared(fs: &RefCell<GameFs>, name: &str) -> Value {
    let fs = fs.borrow();
    let tm = textures();
    let t = tm.borrow_mut().shared_texture(&fs, name);
    texture_value(t)
}

fn get<R>(c: &NativeCall<'_>, f: impl FnOnce(&Texture) -> R) -> pz_kahlua::LuaResult<R> {
    with_this::<LuaTexture, _>(c, |t| f(&t.0.borrow()))
}

pub fn register(mgr: &mut LuaManager) {
    let fs = mgr.fs.clone();
    let f = fs.clone();
    pz_render::install_shared_texture_loader(move |name| textures().borrow_mut().shared_texture(&f.borrow(), name));
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
            let t = tm
                .borrow_mut()
                .shared_texture_flags(&f.borrow(), &name, int_arg(c, 1));
            texture_value(t)
        } else {
            shared(&f, &name)
        };
        Ok(c.push(v))
    });
    let f = fs.clone();
    b.static_fn("trygetTexture", move |c| {
        let name = str_arg(c, 0).unwrap_or_else(|| Rc::from(""));
        let t = textures().borrow_mut().try_get_texture(&f.borrow(), &name);
        Ok(c.push(texture_value(t)))
    });
    b.static_fn("getWhite", |c| {
        let t = textures().borrow_mut().white();
        Ok(c.push(texture_value(Some(t))))
    });
    b.static_fn("getErrorTexture", |c| {
        let t = textures().borrow_mut().error_texture();
        Ok(c.push(texture_value(Some(t))))
    });
    b.static_fn("onTexturePacksChanged", |_c| {
        textures().borrow_mut().on_texture_packs_changed();
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
        with_this::<LuaTexture, _>(c, |t| t.0.borrow_mut().offset_x = v)?;
        Ok(0)
    });
    b.method("setOffsetY", |c| {
        let v = int_arg(c, 1) as f32;
        with_this::<LuaTexture, _>(c, |t| t.0.borrow_mut().offset_y = v)?;
        Ok(0)
    });
    b.method("toString", |c| {
        let s = with_this::<LuaTexture, _>(c, |t| t.to_lua_string())?;
        Ok(c.push(Value::str(&s)))
    });

    let f = fs.clone();
    crate::expose::global(thread, "getTexture", move |c| {
        let name = str_arg(c, 0).unwrap_or_else(|| Rc::from(""));
        Ok(c.push(shared(&f, &name)))
    });
    let f = fs;
    crate::expose::global(thread, "tryGetTexture", move |c| {
        let name = str_arg(c, 0).unwrap_or_else(|| Rc::from(""));
        let t = textures().borrow_mut().try_get_texture(&f.borrow(), &name);
        Ok(c.push(texture_value(t)))
    });
}

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
