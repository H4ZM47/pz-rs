//! SpriteConfigManager.ObjectInfo retains the script used to parse its faces.
use super::*;
use pz_world::sprite::SpriteConfigInfo;
use std::rc::Weak;

const MANAGER: &str = "zombie.entity.components.spriteconfig.SpriteConfigManager";
const INFO: &str = "zombie.entity.components.spriteconfig.SpriteConfigManager$ObjectInfo";

struct ObjectInfo {
    _info: Rc<SpriteConfigInfo>,
    parent: ScriptHandle,
    component_index: usize,
}

impl LuaObject for ObjectInfo {
    fn class_name(&self) -> &str {
        INFO
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub(super) fn register(thread: &mut Thread, sm: &Rc<ScriptManager>) {
    let manager = expose_class(thread, MANAGER, Some("java.lang.Object"));
    let handles = Rc::new(RefCell::new(std::collections::HashMap::<
        usize,
        Weak<ObjectInfo>,
    >::new()));
    for (method, by_sprite) in [("GetObjectInfo", false), ("getObjectInfoFromSprite", true)] {
        let handles = handles.clone();
        let sm = sm.clone();
        manager.static_fn(method, move |c| {
            let Some(name) = str_arg(c, 0) else {
                return Ok(c.push(Value::Nil));
            };
            let sprites = crate::bind_sprites::manager()?;
            let info = {
                let sprites = sprites.try_borrow().map_err(|_| busy())?;
                let infos = if by_sprite {
                    &sprites.sprite_config_for_tile
                } else {
                    &sprites.sprite_configs
                };
                infos.get(name.as_ref()).cloned()
            };
            let Some(info) = info else {
                return Ok(c.push(Value::Nil));
            };
            let key = Rc::as_ptr(&info) as usize;
            if let Some(handle) = handles.borrow().get(&key).and_then(Weak::upgrade) {
                return Ok(c.push(Value::Object(handle)));
            }
            let script = sm
                .get_script(ScriptType::Entity, &info.entity_name)
                .ok_or_else(gone)?;
            let parent = ScriptHandle::new(script, GAME_ENTITY_SCRIPT_CLASS);
            let component_index = parent
                .read(|s: &GameEntityScript| {
                    s.component_scripts()
                        .iter()
                        .position(|c| c.component_type() == ComponentType::SpriteConfig)
                })?
                .ok_or_else(gone)?;
            let handle = Rc::new(ObjectInfo {
                _info: info,
                parent,
                component_index,
            });
            handles.borrow_mut().insert(key, Rc::downgrade(&handle));
            Ok(c.push(Value::Object(handle)))
        });
    }
    let info = expose_class(thread, INFO, Some("java.lang.Object"));
    info.method("getScript", |c| {
        let receiver = c.arg(0);
        let Value::Object(receiver) = &receiver else {
            return Err(npe());
        };
        let info = receiver
            .as_any()
            .downcast_ref::<ObjectInfo>()
            .ok_or_else(npe)?;
        let script = component_value(&info.parent, info.component_index)?;
        Ok(c.push(script))
    });
}
