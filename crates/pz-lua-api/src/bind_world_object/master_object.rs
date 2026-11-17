//! `IsoObject.getMasterObject()`, which the world context menu reads for
//! each clicked object (`ISContextEntity.createMenu`).

use super::{borrowed, object_from_value, runtime_error, unsupported, ISO_OBJECT};
use pz_kahlua::{LuaError, Thread};

pub(super) fn register(thread: &mut Thread) {
    let object = crate::expose::expose_class(thread, ISO_OBJECT, None);
    // `getSpriteConfig()`, then `SpriteConfig.getMultiSquareMaster()`; an
    // object without a sprite config is its own master.
    object.method("getMasterObject", |c| {
        let this = c.arg(0);
        let world = object_from_value(&this).ok_or_else(|| LuaError::runtime("expected IsoObject receiver"))?;
        let config = world
            .object
            .try_borrow()
            .map_err(|_| borrowed("IsoObject body"))?
            .components
            .get_ref(pz_entity::ComponentType::SpriteConfig)
            .map_err(runtime_error)?;
        if config.is_some() {
            return Err(unsupported("SpriteConfig.getMultiSquareMaster"));
        }
        Ok(c.push(this))
    });
}

#[cfg(test)]
mod tests {
    use pz_entity::{Component, Components};
    use pz_inventory::item_components::ItemComponents;
    use pz_kahlua::{stdlib, TableRef, Thread};
    use pz_world::objects::ObjectKind;
    use pz_world::{IsoObject, ObjectSlot};

    #[test]
    fn an_object_without_a_sprite_config_is_its_own_master() {
        let plain = ObjectSlot::new(IsoObject::new(1, ObjectKind::Object, None)).ref_handle();
        let mut configured = IsoObject::new(2, ObjectKind::Object, None);
        configured.components = ItemComponents::from_values_unowned(Components {
            list: vec![Component::SpriteConfig(Default::default())],
        })
        .unwrap();
        let configured = ObjectSlot::new(configured).ref_handle();
        let mut t = Thread::new(TableRef::new());
        stdlib::install(&mut t, None).unwrap();
        super::super::register_classes(&mut t);
        for (name, o) in [("plain", &plain), ("configured", &configured)] {
            super::super::ensure_class(o, super::ISO_OBJECT).unwrap();
            t.globals().set_str(name, super::super::object_value(o).unwrap());
        }
        let src = "local ok, err = pcall(configured.getMasterObject, configured)
            return rawequal(plain:getMasterObject(), plain), ok, tostring(err)";
        let f = stdlib::load_string(src, Some("t"), t.globals().clone()).unwrap();
        let r = t.pcall(&f, &[]).unwrap_or_else(|(e, _)| panic!("{}", pz_kahlua::exec::java_value_of(&e)));
        assert!(r[0].truthy() && !r[1].truthy(), "{r:?}");
        assert!(pz_kahlua::exec::java_value_of(&r[2]).contains("SpriteConfig.getMultiSquareMaster"), "{r:?}");
        drop((plain, configured));
        pz_inventory::component_host::drain_retired_components();
    }
}
