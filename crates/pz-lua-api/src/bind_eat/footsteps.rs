use super::*;
use crate::bind_character_sim::ISO_PLAYER_CLASS;
use crate::bind_world_object::{borrowed, host_error};
use pz_entity::runtime_load::RuntimeHostError;
use pz_sound::emitter::ParentObject;
use pz_world::objects::{ObjectKind, ObjectSprite};

const MATERIALS: [&str; 13] = [
    "Upstairs",
    "BrokenGlass",
    "Concrete",
    "Grass",
    "Gravel",
    "Puddle",
    "Snow",
    "Wood",
    "Carpet",
    "Dirt",
    "Sand",
    "Ceramic",
    "Metal",
];

fn current_square(chr: &LuaGameCharacterRef) -> LuaResult<Option<pz_world::SquareRef>> {
    let Some((x, y, z)) = chr.borrow().path.current_square else {
        return Ok(None);
    };
    crate::bind_world::try_with_world_read(|w| {
        Ok(w.cell.as_ref().and_then(|c| c.get_grid_square(x, y, z)))
    })
    .map_err(host_error)
}

/// FMODManager counts occupied local player slots, including deaf listeners.
fn upstairs(chr: &LuaGameCharacterRef) -> LuaResult<bool> {
    let mut locals = (0..crate::bind_players::num_players()
        .clamp(0, crate::bind_players::MAX_PLAYERS as i32) as usize)
        .filter_map(crate::bind_players::player);
    let Some(listener) = locals.next() else {
        return Ok(false);
    };
    if locals.next().is_some() || Rc::ptr_eq(&listener, chr) {
        return Ok(false);
    }
    let listener = listener
        .try_borrow()
        .map_err(|_| borrowed("IsoPlayer listener"))?;
    let sm = crate::bind_registry::shared_script_manager();
    Ok(
        pz_world::java::fastfloor(listener.vars.z) < pz_world::java::fastfloor(chr.borrow().vars.z)
            && !listener
                .has_trait(&sm, "DEAF")
                .map_err(crate::bind_registry::java_error)?,
    )
}

fn materials(chr: &LuaGameCharacterRef) -> LuaResult<(usize, u8)> {
    let above = upstairs(chr)?;
    let Some(square) = current_square(chr)? else {
        return Ok((if above { 0 } else { 2 }, 0));
    };
    let manager = crate::bind_sprites::manager()?;
    let manager = manager
        .try_borrow()
        .map_err(|_| borrowed("IsoSpriteManager"))?;
    let square = square.try_borrow().map_err(|_| borrowed("IsoGridSquare"))?;
    let mut material = None;
    let mut overlay = 0;
    for slot in &square.objects {
        let body = slot.try_borrow().map_err(|_| borrowed("IsoObject"))?;
        if matches!(body.kind, ObjectKind::BrokenGlass) {
            overlay = 1;
        }
        if matches!(body.sprite, ObjectSprite::ArchiveUnmanaged(_)) {
            continue;
        }
        let Some(sprite) = body.sprite.try_read(&manager).map_err(host_error)? else {
            continue;
        };
        if !matches!(body.kind, ObjectKind::WorldInventory(_)) {
            if let Some(name) = sprite.properties.get(&manager.alias, "FootstepMaterial") {
                material = MATERIALS.iter().position(|&m| m == name);
            }
        }
        if overlay == 0 {
            overlay = match sprite.tileset_name.as_deref() {
                Some("d_trash_1" | "trash_01") => 4,
                Some("d_floorleaves_1") => 5,
                _ => 0,
            };
        }
    }
    let primary = if above {
        0
    } else if crate::bind_world::snow::square_is_snow(square.x, square.y, square.z)? {
        6
    } else {
        material.unwrap_or_else(|| if square.has_stairs(&manager) { 7 } else { 2 })
    };
    Ok((primary, overlay))
}

fn shoe_type(chr: &LuaGameCharacterRef) -> usize {
    let sm = crate::bind_registry::shared_script_manager();
    let actor = chr.borrow();
    for item in actor.items.worn() {
        let item = item.borrow();
        if !pz_characters::character::items::is_body_location(&sm, &item, "SHOES") {
            continue;
        }
        let name = item.get_type();
        return if name.contains("Boots") || name.contains("Wellies") {
            1
        } else if name.contains("FlipFlop") {
            2
        } else if name.contains("Slippers") {
            4
        } else if name.contains("Trainer") {
            5
        } else {
            3
        };
    }
    0
}

pub(super) fn play(chr: &LuaGameCharacterRef, name: &str) -> LuaResult<()> {
    if crate::bind_character_sim::current_sim_context().is_server() {
        return Ok(());
    }
    let (material, overlay) = materials(chr)?;
    let shoes = shoe_type(chr);
    let actor = chr.borrow();
    let position = [actor.vars.x, actor.vars.y, actor.vars.z];
    let params = [
        ("FootstepMaterial", material as f32),
        ("FootstepMaterial2", f32::from(overlay)),
        ("ShoeType", shoes as f32),
        (
            "EquippedBaggageContainer",
            f32::from(actor.player.sound_baggage_container),
        ),
        (
            "CharacterMovementSpeed",
            f32::from(actor.player.sound_movement_speed),
        ),
    ];
    drop(actor);
    with_audio(chr, |a| {
        let mut e = a.footsteps.borrow_mut();
        crate::bind_audio::sound_system()
            .borrow_mut()
            .with_world_emitters(|_, ctx| {
                e.set_pos(position[0], position[1], position[2]);
                let mut first = e.is_playing(a.steps[0]);
                if e.is_playing(a.steps[1]) {
                    if first {
                        a.steps.swap(0, 1);
                        for (key, val) in params {
                            e.set_parameter_value_by_name(a.steps[1], key, val, ctx);
                        }
                        if e.restart(a.steps[1], ctx) {
                            return;
                        }
                        e.stop_sound_local(a.steps[1], ctx);
                    } else {
                        a.steps[0] = a.steps[1];
                        first = true;
                    }
                    a.steps[1] = 0;
                }
                let h = e.play_sound_impl(name, ParentObject::None, ctx);
                a.steps[usize::from(first)] = h;
                for (key, val) in params {
                    e.set_parameter_value_by_name(h, key, val, ctx);
                }
                e.tick(ctx);
            });
    });
    Ok(())
}

pub(super) fn register(thread: &mut Thread) {
    let player = expose_class(thread, ISO_PLAYER_CLASS, Some(ISO_GAME_CHARACTER_CLASS));
    player.method("DoFootstepSound", |c| {
        let chr = chr_of(c)?;
        let v = c.arg(0);
        let volume = match c.arg(1) {
            Value::Str(gait) => chr.borrow_mut().player_footstep_gait(&gait),
            _ => num_arg(c, 1) as f32,
        };
        let indoor = current_square(&chr)?
            .map(|sq| {
                sq.try_borrow()
                    .map(|s| s.room_id != -1)
                    .map_err(|_| RuntimeHostError::Borrowed("IsoGridSquare"))
            })
            .transpose()
            .map_err(host_error)?
            .unwrap_or(false);
        let sm = crate::bind_registry::shared_script_manager();
        with_ctx(c, &v, |ctx| {
            chr.borrow()
                .player_footstep(volume, indoor, &sm, &shared_characters().borrow(), ctx)
        })?;
        Ok(0)
    });
}
