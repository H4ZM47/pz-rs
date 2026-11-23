//! The object may share a tile-definition sprite or own a single-texture sprite.
use super::*;
use pz_world::sprite::runtime::{create_runtime_sprite, load_single_texture, ObjectSprite};

pub(super) fn register(class: &crate::expose::ClassBuilder) {
    class.method("transmitModData", |c| {
        let obj = receiver_object(c)?;
        if obj.try_borrow().map_err(|_| borrowed("IsoObject.square"))?.square_slot().is_none() {
            return Ok(0);
        }
        let core = crate::bind_core::shared_core();
        let core = core.borrow();
        if core.is_client || core.is_server {
            return Err(unsupported("networked map object mod data"));
        }
        drop(core);
        pz_world::square_mutation::try_flag_object_for_hot_save(&obj).map_err(host_error)?;
        Ok(0)
    });
    for method in [
        "transmitUpdatedSpriteToClients",
        "transmitCompleteItemToClients",
    ] {
        class.method(method, move |c| {
            receiver_object(c)?;
            // Both IsoObject methods enter the UDP send branch only on a server.
            if crate::bind_core::shared_core().borrow().is_server {
                return Err(unsupported(format!("{method}: server object replication")));
            }
            Ok(0)
        });
    }
    class.method("setSpriteFromName", |c| {
        let name = crate::expose::str_arg(c, 1)
            .ok_or_else(|| LuaError::runtime("expected sprite name"))?;
        set_named(c, &name)
    });
    class.method("setSprite", |c| {
        let this = c.arg(0);
        let arg = c.arg(1);
        if let Value::Str(name) = &arg {
            if matches!(
                receiver_object(c)?
                    .try_borrow()
                    .map_err(|_| borrowed("IsoObject"))?
                    .kind,
                ObjectKind::Thumpable(_)
            ) {
                return set_named(c, name);
            }
        }
        let mut retired = pz_entity::RetiredComponents::new();
        let result = (|| {
            let (sprite, name) = match &arg {
                Value::Str(name) => {
                    let manager = crate::bind_sprites::manager()?;
                    let tables = manager
                        .try_borrow()
                        .map_err(|_| borrowed("IsoSpriteManager"))?
                        .tables
                        .clone();
                    let sprite = create_runtime_sprite(&manager, &tables, &mut retired)
                        .map_err(host_error)?;
                    load_single_texture(&sprite, name, &mut retired).map_err(host_error)?;
                    (ObjectSprite::Native(sprite), Some(name.to_string()))
                }
                _ => (crate::bind_sprites::object_sprite_argument(&arg)?, None),
            };
            let receiver = object_from_value(&this).ok_or_else(|| borrowed("IsoObject"))?;
            let square = object_square(receiver)?;
            let old = {
                let mut body = receiver
                    .object
                    .try_borrow_mut()
                    .map_err(|_| borrowed("IsoObject"))?;
                if let Some(name) = name {
                    body.tile = Some(name.clone());
                    body.sprite_name = Some(pz_world::objects::SpriteName::Str(name));
                }
                std::mem::replace(&mut body.sprite, sprite)
            };
            retired.push_payload(Box::new(old));
            if let Some(pos) = crate::bind_world::square_coords(&square) {
                let manager = crate::bind_sprites::manager()?;
                let sprites = manager
                    .try_borrow()
                    .map_err(|_| borrowed("IsoSpriteManager"))?;
                crate::bind_world::try_with_world_read(|w| {
                    if let (Some(cell), Some(meta)) = (&w.cell, &w.meta_grid) {
                        w.geometry.notify(cell, meta, &sprites, pos);
                    }
                    Ok(())
                })
                .map_err(host_error)?;
            }
            Ok(0)
        })();
        pz_inventory::component_host::retire_components(retired);
        result
    });
}

fn set_named(c: &mut pz_kahlua::NativeCall<'_>, name: &str) -> LuaResult<usize> {
    let object = receiver_object(c)?;
    let manager = crate::bind_sprites::manager()?;
    let idx = {
        let mut manager = manager
            .try_borrow_mut()
            .map_err(|_| borrowed("IsoSpriteManager"))?;
        manager
            .named_map
            .get(name)
            .copied()
            .unwrap_or_else(|| manager.add_sprite_named(name))
    };
    let this = c.arg(0);
    let set = c.thread.table_get(&this, &Value::str("setSprite"))?;
    c.thread.call1(
        &set,
        &[this, crate::bind_sprites::sprite_value(Some(idx), false)],
    )?;
    let mut body = object.try_borrow_mut().map_err(|_| borrowed("IsoObject"))?;
    let closed = body.sprite.tile_projection();
    if let ObjectKind::Thumpable(t) = &mut body.kind {
        t.closed_sprite = closed;
    }
    Ok(0)
}
