//! Build 42.22 `zombie.iso.fboRenderChunk.FBORenderChunk` and
//! `IsoObject.invalidateRenderChunkLevel` wired into the renderer's chunk
//! invalidation caches.

use crate::bind_world_object::{object_from_value, ISO_OBJECT};
use crate::expose::{expose_class, int_arg, num_arg};
use pz_iso_render::chunk::{
    dirty, FLOOR_HEIGHT, FLOOR_WIDTH, JUMBO_L_HEIGHT, JUMBO_L_WIDTH, JUMBO_XL_WIDTH,
    JUMBO_XXL_HEIGHT, JUMBO_XXL_WIDTH,
};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{Thread, Value};
use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

pub const FBO_RENDER_CHUNK_CLASS: &str = "zombie.iso.fboRenderChunk.FBORenderChunk";

/// `FBORenderChunk.PIXELS_PER_LEVEL` (32 * 3 * 2).
pub const PIXELS_PER_LEVEL: i32 = 192;
/// `FBORenderChunk.JUMBO_XL_HEIGHT` (FLOOR_HEIGHT * 12).
pub const JUMBO_XL_HEIGHT: i32 = FLOOR_HEIGHT * 12;
/// `FBORenderChunk.TEXTURE_HEIGHT`.
pub const TEXTURE_HEIGHT: i32 = 1024;
/// `FBORenderChunk.LEVELS_PER_TEXTURE`.
pub const LEVELS_PER_TEXTURE: i32 = 2;

#[derive(Debug, Default)]
struct FboRenderChunkState {
    min_level: i32,
    w: i32,
    h: i32,
    high_res: bool,
    submitted: bool,
    is_init: bool,
}

struct LuaFboRenderChunk(RefCell<FboRenderChunkState>);

impl LuaObject for LuaFboRenderChunk {
    fn class_name(&self) -> &str {
        FBO_RENDER_CHUNK_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

thread_local! {
    /// Queued `(wx, wy, z, dirty_flags)` chunk-level invalidations from Lua
    /// `IsoObject:invalidateRenderChunkLevel(dirtyFlags)`.
    static PENDING_CHUNK_INVALIDATIONS: RefCell<Vec<(i32, i32, i32, u64)>> =
        const { RefCell::new(Vec::new()) };
}

/// Queue a chunk-level FBO render invalidation for the renderer to apply on the
/// next frame.
pub fn queue_render_chunk_invalidation(wx: i32, wy: i32, z: i32, flags: u64) {
    PENDING_CHUNK_INVALIDATIONS.with(|q| q.borrow_mut().push((wx, wy, z, flags)));
}

/// Drain all queued `(wx, wy, z, dirty_flags)` invalidations from Lua.
pub fn take_render_chunk_invalidations() -> Vec<(i32, i32, i32, u64)> {
    PENDING_CHUNK_INVALIDATIONS.with(|q| std::mem::take(&mut *q.borrow_mut()))
}

/// Reset per-session state when leaving a world.
pub(crate) fn reset_session_handles() {
    PENDING_CHUNK_INVALIDATIONS.with(|q| q.borrow_mut().clear());
}

fn with_fbo<R>(v: &Value, f: impl FnOnce(&FboRenderChunkState) -> R) -> Option<R> {
    if let Value::Object(o) = v {
        if let Some(rc) = o.as_any().downcast_ref::<LuaFboRenderChunk>() {
            let res = Some(f(&rc.0.borrow()));
            return res;
        }
    }
    None
}

pub fn register(thread: &mut Thread) {
    register_fbo_render_chunk(thread);
    register_iso_object_invalidation(thread);
}

fn register_fbo_render_chunk(thread: &mut Thread) {
    let fbo = expose_class(thread, FBO_RENDER_CHUNK_CLASS, Some("java.lang.Object"));

    fbo.field("PIXELS_PER_LEVEL", Value::Num(PIXELS_PER_LEVEL as f64));
    fbo.field("FLOOR_WIDTH", Value::Num(FLOOR_WIDTH as f64));
    fbo.field("FLOOR_HEIGHT", Value::Num(FLOOR_HEIGHT as f64));
    fbo.field("JUMBO_L_WIDTH", Value::Num(JUMBO_L_WIDTH as f64));
    fbo.field("JUMBO_L_HEIGHT", Value::Num(JUMBO_L_HEIGHT as f64));
    fbo.field("JUMBO_XL_WIDTH", Value::Num(JUMBO_XL_WIDTH as f64));
    fbo.field("JUMBO_XL_HEIGHT", Value::Num(JUMBO_XL_HEIGHT as f64));
    fbo.field("JUMBO_XXL_WIDTH", Value::Num(JUMBO_XXL_WIDTH as f64));
    fbo.field("JUMBO_XXL_HEIGHT", Value::Num(JUMBO_XXL_HEIGHT as f64));
    fbo.field("TEXTURE_HEIGHT", Value::Num(TEXTURE_HEIGHT as f64));
    fbo.field("LEVELS_PER_TEXTURE", Value::Num(LEVELS_PER_TEXTURE as f64));

    fbo.field("DIRTY_NONE", Value::Num(0.0));
    fbo.field("DIRTY_BLOOD", Value::Num(dirty::BLOOD as f64));
    fbo.field("DIRTY_CORPSE", Value::Num(dirty::CORPSE as f64));
    fbo.field("DIRTY_ITEM_ADD", Value::Num(dirty::ITEM_ADD as f64));
    fbo.field("DIRTY_ITEM_REMOVE", Value::Num(dirty::ITEM_REMOVE as f64));
    fbo.field("DIRTY_ITEM_MODIFY", Value::Num(dirty::ITEM_MODIFY as f64));
    fbo.field("DIRTY_LIGHTING", Value::Num(dirty::LIGHTING as f64));
    fbo.field("DIRTY_OBJECT_ADD", Value::Num(dirty::OBJECT_ADD as f64));
    fbo.field("DIRTY_OBJECT_REMOVE", Value::Num(dirty::OBJECT_REMOVE as f64));
    fbo.field("DIRTY_OBJECT_MODIFY", Value::Num(dirty::OBJECT_MODIFY as f64));
    fbo.field("DIRTY_CREATE", Value::Num(dirty::CREATE as f64));
    fbo.field("DIRTY_REDRAW", Value::Num(dirty::REDRAW as f64));
    fbo.field("DIRTY_CUTAWAYS", Value::Num(dirty::CUTAWAYS as f64));
    fbo.field("DIRTY_TREES", Value::Num(dirty::TREES as f64));
    fbo.field("DIRTY_OBSCURING", Value::Num(dirty::OBSCURING as f64));
    fbo.field("DIRTY_REDO_CUTAWAYS", Value::Num(dirty::REDO_CUTAWAYS as f64));

    fbo.ctor(|c| {
        Ok(c.push(Value::Object(Rc::new(LuaFboRenderChunk(RefCell::new(
            FboRenderChunkState {
                w: pz_iso_render::chunk::texture_width(0, 1),
                h: pz_iso_render::chunk::texture_height(0, 1),
                ..Default::default()
            },
        ))))))
    });

    fbo.method("getMinLevel", |c| {
        let v = with_fbo(&c.arg(0), |s| s.min_level).unwrap_or(0);
        Ok(c.push(v))
    });
    fbo.method("getTopLevel", |c| {
        let v = with_fbo(&c.arg(0), |s| s.min_level + LEVELS_PER_TEXTURE - 1).unwrap_or(1);
        Ok(c.push(v))
    });
    fbo.method("isTopLevel", |c| {
        let level = int_arg(c, 1);
        let top = with_fbo(&c.arg(0), |s| s.min_level + LEVELS_PER_TEXTURE - 1 == level)
            .unwrap_or(false);
        Ok(c.push(top))
    });
    fbo.method("getTextureWidth", |c| {
        let scale = num_arg(c, 1) as f32;
        let scale = if scale > 0.0 { scale } else { 1.0 };
        let w = with_fbo(&c.arg(0), |s| ((s.w as f32) * scale) as i32)
            .unwrap_or((pz_iso_render::chunk::texture_width(0, 1) as f32 * scale) as i32);
        Ok(c.push(w))
    });
    fbo.method("getTextureHeight", |c| {
        let scale = num_arg(c, 1) as f32;
        let scale = if scale > 0.0 { scale } else { 1.0 };
        let h = with_fbo(&c.arg(0), |s| ((s.h as f32) * scale) as i32)
            .unwrap_or((pz_iso_render::chunk::texture_height(0, 1) as f32 * scale) as i32);
        Ok(c.push(h))
    });
    fbo.method("isHighRes", |c| {
        let v = with_fbo(&c.arg(0), |s| s.high_res).unwrap_or(false);
        Ok(c.push(v))
    });
    fbo.method("isInit", |c| {
        let v = with_fbo(&c.arg(0), |s| s.is_init).unwrap_or(false);
        Ok(c.push(v))
    });
    fbo.method("isSubmitted", |c| {
        let v = with_fbo(&c.arg(0), |s| s.submitted).unwrap_or(false);
        Ok(c.push(v))
    });
}

fn register_iso_object_invalidation(thread: &mut Thread) {
    let object = expose_class(thread, ISO_OBJECT, None);
    object.method("invalidateRenderChunkLevel", |c| {
        let flags = num_arg(c, 1).max(0.0) as u64;
        if flags == 0 {
            return Ok(0);
        }
        let this = c.arg(0);
        let pos: Option<(i32, i32, i32)> = if let Some(obj) = object_from_value(&this) {
            obj.object.try_borrow().ok().and_then(|o| {
                o.square_slot().map(|sq| {
                    let sq = sq.borrow();
                    (sq.x, sq.y, sq.z)
                })
            })
        } else {
            None
        };
        let pos = match pos {
            Some(p) => Some(p),
            None => {
                let get_sq = c.thread.table_get(&this, &Value::str("getSquare"))?;
                if get_sq.is_nil() {
                    None
                } else {
                    let sq = c.thread.call1(&get_sq, &[this])?;
                    if sq.is_nil() {
                        None
                    } else {
                        let gx = c.thread.table_get(&sq, &Value::str("getX"))?;
                        let gy = c.thread.table_get(&sq, &Value::str("getY"))?;
                        let gz = c.thread.table_get(&sq, &Value::str("getZ"))?;
                        if gx.is_nil() || gy.is_nil() || gz.is_nil() {
                            None
                        } else {
                            let x = c.thread.call1(&gx, &[sq.clone()])?;
                            let y = c.thread.call1(&gy, &[sq.clone()])?;
                            let z = c.thread.call1(&gz, &[sq])?;
                            match (
                                pz_kahlua::exec::raw_tonumber(&x),
                                pz_kahlua::exec::raw_tonumber(&y),
                                pz_kahlua::exec::raw_tonumber(&z),
                            ) {
                                (Some(x), Some(y), Some(z)) => {
                                    Some((x.floor() as i32, y.floor() as i32, z.floor() as i32))
                                }
                                _ => None,
                            }
                        }
                    }
                }
            }
        };
        if let Some((x, y, z)) = pos {
            queue_render_chunk_invalidation(x.div_euclid(8), y.div_euclid(8), z, flags);
        }
        Ok(0)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::{stdlib, TableRef};
    use pz_world::objects::ObjectKind;
    use pz_world::{IsoGridSquare, IsoObject, ObjectSlot};

    #[test]
    fn fbo_render_chunk_and_object_invalidation_lua() {
        reset_session_handles();
        let mut t = Thread::new(TableRef::new());
        stdlib::install(&mut t, None).unwrap();
        crate::bind_world_object::register_classes(&mut t);
        register(&mut t);

        let sq = pz_world::SquareSlot::new(IsoGridSquare::new(17, 25, 1)).ref_handle();
        let mut obj = IsoObject::new(1, ObjectKind::Object, None);
        obj.replace_square(Some(&sq), &mut pz_entity::RetiredComponents::default());
        let obj_ref = ObjectSlot::new(obj).ref_handle();
        crate::bind_world_object::ensure_class(&obj_ref, ISO_OBJECT).unwrap();
        let obj_val = crate::bind_world_object::object_value(&obj_ref).unwrap();
        t.globals().set_str("testObj", obj_val);

        let src = r#"
            local function assert(v, msg)
                if not v then error(msg or "assertion failed") end
            end

            assert(FBORenderChunk.PIXELS_PER_LEVEL == 192)
            assert(FBORenderChunk.FLOOR_WIDTH == 128)
            assert(FBORenderChunk.FLOOR_HEIGHT == 64)
            assert(FBORenderChunk.JUMBO_L_WIDTH == 384)
            assert(FBORenderChunk.JUMBO_L_HEIGHT == 512)
            assert(FBORenderChunk.JUMBO_XL_WIDTH == 640)
            assert(FBORenderChunk.JUMBO_XL_HEIGHT == 768)
            assert(FBORenderChunk.JUMBO_XXL_WIDTH == 896)
            assert(FBORenderChunk.JUMBO_XXL_HEIGHT == 1024)
            assert(FBORenderChunk.TEXTURE_HEIGHT == 1024)
            assert(FBORenderChunk.LEVELS_PER_TEXTURE == 2)
            assert(FBORenderChunk.DIRTY_NONE == 0)
            assert(FBORenderChunk.DIRTY_BLOOD == 1)
            assert(FBORenderChunk.DIRTY_OBJECT_ADD == 64)
            assert(FBORenderChunk.DIRTY_REDRAW == 1024)
            assert(FBORenderChunk.DIRTY_REDO_CUTAWAYS == 16384)

            local rc = FBORenderChunk.new()
            assert(rc:getMinLevel() == 0)
            assert(rc:getTopLevel() == 1)
            assert(rc:isTopLevel(1) == true)
            assert(rc:isTopLevel(0) == false)

            testObj:invalidateRenderChunkLevel(FBORenderChunk.DIRTY_OBJECT_ADD)
            testObj:invalidateRenderChunkLevel(FBORenderChunk.DIRTY_REDRAW)
        "#;
        let f = stdlib::load_string(src, Some("fbo_render_chunk"), t.globals().clone()).unwrap();
        t.pcall(&f, &[]).unwrap();

        let invalidations = take_render_chunk_invalidations();
        assert_eq!(
            invalidations,
            vec![(2, 3, 1, dirty::OBJECT_ADD), (2, 3, 1, dirty::REDRAW)]
        );
        assert!(take_render_chunk_invalidations().is_empty());
    }
}
