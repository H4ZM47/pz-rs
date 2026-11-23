//! The object may share a tile-definition sprite or own a single-texture sprite.
use super::*;
use pz_world::sprite::runtime::{create_runtime_sprite, load_single_texture, ObjectSprite};

pub(super) fn register(class: &crate::expose::ClassBuilder) {
    class.method("setSprite", |c| {
        let this = c.arg(0);
        let arg = c.arg(1);
        let mut retired = pz_entity::RetiredComponents::new();
        let result = (|| {
            let (sprite, name) = match &arg {
                Value::Str(name) => {
                    let manager = crate::bind_sprites::manager()?;
                    let tables = manager.try_borrow().map_err(|_| borrowed("IsoSpriteManager"))?.tables.clone();
                    let sprite = create_runtime_sprite(&manager, &tables, &mut retired).map_err(host_error)?;
                    load_single_texture(&sprite, name, &mut retired).map_err(host_error)?;
                    (ObjectSprite::Native(sprite), Some(name.to_string()))
                }
                _ => (crate::bind_sprites::object_sprite_argument(&arg)?, None),
            };
            let receiver = object_from_value(&this).ok_or_else(|| borrowed("IsoObject"))?;
            let square = object_square(receiver)?;
            let old = {
                let mut body = receiver.object.try_borrow_mut().map_err(|_| borrowed("IsoObject"))?;
                if let Some(name) = name {
                    body.tile = Some(name.clone());
                    body.sprite_name = Some(pz_world::objects::SpriteName::Str(name));
                }
                std::mem::replace(&mut body.sprite, sprite)
            };
            retired.push_payload(Box::new(old));
            if let Some(pos) = crate::bind_world::square_coords(&square) {
                let manager = crate::bind_sprites::manager()?;
                let sprites = manager.try_borrow().map_err(|_| borrowed("IsoSpriteManager"))?;
                crate::bind_world::try_with_world_read(|w| {
                    if let (Some(cell), Some(meta)) = (&w.cell, &w.meta_grid) {
                        w.geometry.notify(cell, meta, &sprites, pos);
                    }
                    Ok(())
                }).map_err(host_error)?;
            }
            Ok(0)
        })();
        pz_inventory::component_host::retire_components(retired);
        result
    });
}
