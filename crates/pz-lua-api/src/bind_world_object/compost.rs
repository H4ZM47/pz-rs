//! The shipped MOCompost callbacks replace authored tiles with these objects.
use super::*;
use crate::expose::{expose_class, int_arg, num_arg};
use pz_kahlua::NativeCall;
use pz_world::objects::{IsoCompost, ItemContainer};

fn construct(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    let cell = c.arg(0);
    if !cell.is_nil() && !cell.raw_equals(&crate::bind_world::current_cell()) {
        return Err(LuaError::runtime("expected IsoCell"));
    }
    let state = IsoCompost {
        compost: 0.,
        last_updated: -1.,
        health: 100,
        max_health: 100,
    };
    let mut body = IsoObject::new(0, ObjectKind::Compost(state), None);
    match c.nargs() {
        1 => {}
        3 => {
            let (_, square) = construction::get_square(&c.arg(1))?;
            let arg = c.arg(2);
            body.sprite = if let Some(name) = arg.as_str() {
                construction::named_sprite(name)?
            } else {
                crate::bind_sprites::object_sprite_argument(&arg)?
            };
            let manager = crate::bind_sprites::manager()?;
            let manager = manager
                .try_borrow()
                .map_err(|_| borrowed("IsoSpriteManager"))?;
            let mut container = ItemContainer::new(Some("composter"));
            container.explored = true;
            let capacity = body
                .sprite
                .try_read(&manager)
                .map_err(host_error)?
                .as_deref()
                .and_then(|s| manager.prop(s, "ContainerCapacity"))
                .and_then(|s| s.parse().ok())
                .unwrap_or(30);
            container.capacity = capacity;
            body.container = Some(container);
            body.replace_square(Some(&square), &mut pz_entity::RetiredComponents::new());
        }
        _ => return Err(LuaError::runtime("invalid IsoCompost constructor")),
    }
    Ok(c.push(object_value(pz_world::ObjectSlot::new(body).owner_slot())?))
}

pub(super) fn register(t: &mut Thread) {
    let class = expose_class(t, "zombie.iso.objects.IsoCompost", Some(ISO_OBJECT));
    class.static_fn("new", construct);
    for method in [
        "getCompost",
        "setCompost",
        "getHealth",
        "setHealth",
        "getMaxHealth",
        "setMaxHealth",
    ] {
        class.method(method, move |c| {
            let obj = receiver_object(c)?;
            let mut b = obj.try_borrow_mut().map_err(|_| borrowed("IsoCompost"))?;
            let ObjectKind::Compost(v) = &mut b.kind else {
                return Err(LuaError::runtime("expected IsoCompost"));
            };
            Ok(match method {
                "getCompost" => c.push(v.compost as f64),
                "setCompost" => {
                    v.compost = (num_arg(c, 1) as f32).clamp(0., 100.);
                    0
                }
                "getHealth" => c.push(v.health),
                "setHealth" => {
                    v.health = int_arg(c, 1);
                    0
                }
                "getMaxHealth" => c.push(v.max_health),
                _ => {
                    v.max_health = int_arg(c, 1);
                    0
                }
            })
        });
    }
    class.method("updateSprite", |c| {
        let obj = receiver_object(c)?;
        let manager = crate::bind_sprites::manager()?;
        let m = manager
            .try_borrow()
            .map_err(|_| borrowed("IsoSpriteManager"))?;
        let b = obj.try_borrow().map_err(|_| borrowed("IsoCompost"))?;
        let ObjectKind::Compost(v) = &b.kind else {
            return Err(LuaError::runtime("expected IsoCompost"));
        };
        let spr = b.sprite.try_read(&m).map_err(host_error)?;
        let name = spr.as_deref().and_then(|s| s.name.as_deref());
        let new = match (name, v.compost >= 10.) {
            (Some("camping_01_19"), true) => "camping_01_20",
            (Some("camping_01_20"), false) => "camping_01_19",
            (Some("carpentry_02_116"), true) => "carpentry_02_117",
            (Some("carpentry_02_117"), false) => "carpentry_02_116",
            _ => return Ok(0),
        };
        drop(spr);
        drop(b);
        drop(m);
        let this = c.arg(0);
        let setter = c
            .thread
            .table_get(&this, &Value::str("setSpriteFromName"))?;
        c.thread.call1(&setter, &[this, Value::str(new)])?;
        Ok(0)
    });
}
