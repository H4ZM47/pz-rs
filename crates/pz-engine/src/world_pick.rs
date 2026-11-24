//! The world's end of the mouse: what `UIManager.update` asks of the world
//! (`IsoObjectPicker.Instance.ContextPick`, the player's and the clock's
//! state, `IsoObject.getObjectIndex`) and what `IsoObjectPicker.Instance`'s
//! `Pick*` give Lua, over the world renderer's records.
//!
//! [`install`] puts one [`EngineWorldMouse`] into `UIManager`
//! ([`pz_ui::WorldMouse`]) and into the Lua class
//! ([`pz_lua_api::bind_object_picker::ObjectPicker`]).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use pz_entity::runtime::RuntimeBody;
use pz_entity::runtime_load::{HostResult, RuntimeHostError};
use pz_entity::{Component, ComponentType};
use pz_kahlua::{LuaResult, Thread, Value};
use pz_lua_api::bind_object_picker::{self, ObjectPicker, Pick};
use pz_lua_api::hud::host::{with_host, HudPicking, NoHud};
use pz_lua_api::hud::picking::{self, ClickObject, PickObject, PickPlayer, PickWorld};
use pz_lua_api::hud::speed_controls::SpeedControls;
use pz_ui::tooltip::TooltipObject;
use pz_ui::{with_ui, PickedObject, WorldMouse};
use pz_world::objects::ObjectKind;
use pz_world::{IsoGridSquare, IsoObject, IsoSpriteManager};
use pz_world_render::{SharedCameraFrame, WorldPick, WorldView};

use crate::world_hud::{EnginePick, PickHooks};
use crate::world_render::WorldRender;

/// `IsoDoor.isAdjacentToSquare(other)` / door `IsoThumpable.isAdjacentToSquare(other)`:
/// the square holding the door or its opposite edge square on the same level.
fn door_is_adjacent_to_square(
    door_sq: (i32, i32, i32),
    obj: &IsoObject,
    other_sq: (i32, i32, i32),
) -> bool {
    let north = match &obj.kind {
        ObjectKind::Door(d) => d.north,
        ObjectKind::Thumpable(t) if t.is_door => t.north,
        _ => return false,
    };
    if door_sq.2 != other_sq.2 {
        return false;
    }
    if other_sq.0 == door_sq.0 && other_sq.1 == door_sq.1 {
        return true;
    }
    if north {
        other_sq.0 == door_sq.0 && other_sq.1 == door_sq.1 - 1
    } else {
        other_sq.0 == door_sq.0 - 1 && other_sq.1 == door_sq.1
    }
}

/// `ClickObject.isInteractiveEntity`: whether the object has an enabled
/// `UiConfig` entity component.
fn object_is_interactive_entity(obj: &IsoObject) -> bool {
    match obj.components.get_ref(ComponentType::UiConfig) {
        Ok(Some(c)) => c
            .try_borrow()
            .ok()
            .is_some_and(|c| matches!(c.body(), RuntimeBody::Data(Component::UiConfig(u)) if u.ui_enabled)),
        _ => false,
    }
}

/// Player 0 as the picker reads it (`IsoPlayer.players[0]`).
struct Player0(PickPlayer);

impl PickHooks for Player0 {
    fn player0(&self) -> Option<PickPlayer> {
        Some(self.0)
    }

    fn square_value(&self, sq: &IsoGridSquare) -> Value {
        pz_lua_api::bind_world::square_value(sq.x, sq.y, sq.z)
    }

    fn adjacent_to_player(&self, sq: &IsoGridSquare, index: usize) -> bool {
        if !self.0.has_square {
            return false;
        }
        let Some(slot) = sq.objects.get(index) else {
            return false;
        };
        let handle = slot.ref_handle();
        let Ok(obj) = handle.try_borrow() else {
            return false;
        };
        let px = pz_render::iso::fastfloor(self.0.x);
        let py = pz_render::iso::fastfloor(self.0.y);
        door_is_adjacent_to_square((sq.x, sq.y, sq.z), &obj, (px, py, self.0.zi))
    }

    fn interactive_entity(&self, sq: &IsoGridSquare, index: usize) -> bool {
        let Some(slot) = sq.objects.get(index) else {
            return false;
        };
        let handle = slot.ref_handle();
        let Ok(obj) = handle.try_borrow() else {
            return false;
        };
        object_is_interactive_entity(&obj)
    }

    fn combined_fbo(&self) -> bool {
        pz_core::debug::debug_options()
            .try_borrow()
            .is_ok_and(|o| o.get_boolean("FBORenderChunk.CombinedFbo"))
    }
    // TODO(port): `corpses` (`IsoDeadBody` is not exposed to Lua yet) and
    // multi-square `SpriteConfig.getMultiSquareMaster()` lookup.
}

/// `UIManager`'s and `IsoObjectPicker.Instance`'s world.
pub struct EngineWorldMouse {
    render: Rc<RefCell<Option<WorldRender>>>,
    sprites: Rc<RefCell<IsoSpriteManager>>,
    cameras: SharedCameraFrame,
    /// The square of the last picked object (`UIManager.picked`).
    picked_square: Cell<[i32; 3]>,
    /// `UIManager.picked.tile` as the shared tooltip reads it.
    picked_tile: RefCell<Option<TooltipObject>>,
}

impl EngineWorldMouse {
    pub fn new(
        render: Rc<RefCell<Option<WorldRender>>>,
        sprites: Rc<RefCell<IsoSpriteManager>>,
        cameras: SharedCameraFrame,
    ) -> Self {
        EngineWorldMouse {
            render,
            sprites,
            cameras,
            picked_square: Cell::new([0; 3]),
            picked_tile: RefCell::new(None),
        }
    }

    /// `f` over the picker's view of the world for player 0; `None` while
    /// there is no player, camera, cell or renderer yet.
    fn with_pick<R>(&self, f: impl FnOnce(&dyn PickWorld) -> HostResult<R>) -> HostResult<Option<R>> {
        let Some(chr) = pz_lua_api::bind_players::player(0) else {
            return Ok(None);
        };
        let Some(camera) = self.cameras.borrow().cameras[0] else {
            return Ok(None);
        };
        let render = self
            .render
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("pick world renderer"))?;
        let Some(render) = render.as_ref() else {
            return Ok(None);
        };
        let sprites = self
            .sprites
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("pick sprite manager"))?;
        pz_lua_api::bind_world::try_with_world_read(|w| {
            let (Some(cell), Some(meta)) = (w.cell.as_ref(), w.meta_grid.as_ref()) else {
                return Ok(None);
            };
            let (x, y, z, dir) = {
                let c = chr.borrow();
                (c.vars.x, c.vars.y, c.vars.z, c.vars.dir)
            };
            let zi = pz_render::iso::fastfloor(z);
            let square = cell.get_grid_square(pz_render::iso::fastfloor(x), pz_render::iso::fastfloor(y), zi);
            let room = match &square {
                Some(s) => {
                    let s = s
                        .try_borrow()
                        .map_err(|_| RuntimeHostError::Borrowed("pick player square"))?;
                    (s.room_id != -1 && s.room_def(meta).is_some()).then_some(s.room_id)
                }
                None => None,
            };
            let (dx, dy) = dir.delta();
            let player = PickPlayer {
                x,
                y,
                z,
                zi,
                forward: (dx as f32, dy as f32),
                has_square: square.is_some(),
                room,
            };
            let pick = EnginePick {
                world: WorldPick {
                    view: WorldView {
                        cell,
                        meta,
                        sprites: &sprites,
                    },
                    render: &render.world,
                    infos: &render.infos,
                    player: 0,
                    camera,
                },
                hooks: Player0(player),
            };
            f(&pick).map(Some)
        })
    }

    /// The Lua object of a picked object.
    fn value(o: &PickObject) -> LuaResult<Option<Value>> {
        bind_object_picker::world_object_value(o.square, o.object_index)
    }
}

fn host_error(error: RuntimeHostError) -> pz_kahlua::LuaError {
    pz_kahlua::LuaError::runtime(format!("object picker: {error:?}"))
}

impl WorldMouse for EngineWorldMouse {
    fn has_player0(&self) -> bool {
        pz_lua_api::bind_players::player(0).is_some()
    }

    fn context_pick(&self, _: &mut Thread, mx: i32, my: i32) -> LuaResult<Option<PickedObject>> {
        let picked: Option<ClickObject> = self
            .with_pick(|w| picking::context_pick(w, mx, my))
            .map_err(host_error)?
            .flatten();
        if let Some(c) = &picked {
            self.picked_square.set(c.tile.square);
        }
        *self.picked_tile.borrow_mut() = picked.as_ref().map(|c| {
            let [x, y, z] = c.tile.square;
            TooltipObject {
                id: c.tile.id,
                have_special_tooltip: c.tile.have_special_tooltip,
                square: pz_lua_api::bind_world::square_value(x, y, z),
            }
        });
        let Some(c) = picked else {
            return Ok(None);
        };
        Ok(Self::value(&c.tile)?.map(|tile| PickedObject {
            tile,
            lx: c.lx,
            ly: c.ly,
        }))
    }

    fn object_index(&self, tile: &Value) -> i32 {
        bind_object_picker::world_object_index(tile, self.picked_square.get())
    }

    fn has_cell(&self) -> bool {
        pz_lua_api::bind_world::try_with_world_read(|w| Ok(w.cell.is_some())).unwrap_or(false)
    }

    fn player_state(&self) -> Option<(bool, bool)> {
        let chr = pz_lua_api::bind_players::player_instance()?;
        let c = chr.borrow();
        Some((c.is_aiming(), c.is_asleep()))
    }

    fn is_game_paused(&self) -> bool {
        pz_lua_api::bind_climate::is_game_paused()
    }

    fn set_current_game_speed(&self, speed: i32) {
        if let Some(e) = with_ui(|m| m.speed_controls.clone()) {
            if let Some(sc) = e.ext::<SpeedControls>() {
                sc.set_current_game_speed(&e, speed);
            }
        }
    }

    // `IsoObject.onMouseRightClick` / `onMouseRightReleased` do nothing for
    // a tile object. TODO(port): `IsoMovingObject.onMouseRightClick`, which
    // starts dragging a nearby moving object (the picker yields no moving
    // objects yet).
    fn on_mouse_right_click(&self, _: &mut Thread, _: &Value, _: i32, _: i32) -> LuaResult<()> {
        Ok(())
    }

    fn on_mouse_right_released(&self, _: &mut Thread, _: &Value) -> LuaResult<()> {
        Ok(())
    }

    // TODO(port): `IsoPlayer.setDragObject`; nothing is dragged yet.
    fn clear_drag_object(&self) {}
}

impl ObjectPicker for EngineWorldMouse {
    fn pick(&self, what: Pick, x: i32, y: i32) -> LuaResult<Option<Value>> {
        let found = match what {
            Pick::Door { transparent } => self.with_pick(|w| picking::pick_door(w, x, y, transparent)),
            Pick::Window => self.with_pick(|w| picking::pick_window(w, x, y)),
            Pick::WindowFrame => self.with_pick(|w| picking::pick_window_frame(w, x, y)),
            Pick::Thumpable => self.with_pick(|w| picking::pick_thumpable(w, x, y)),
            Pick::Tree => self.with_pick(|w| picking::pick_tree(w, x, y)),
            // TODO(port): `PickHoppable` (`IsoObject.getHoppableDirection`)
            // and `PickCorpse` (`IsoDeadBody` is not exposed to Lua).
            Pick::Hoppable | Pick::Corpse => return Ok(None),
        }
        .map_err(host_error)?
        .flatten();
        match found {
            Some(o) => Self::value(&o),
            None => Ok(None),
        }
    }
}

/// `UIManager.getPicked().tile` for `UIManager.updateTooltip`: the
/// object the last context pick found.
struct PickedTile(Rc<EngineWorldMouse>);

impl HudPicking for PickedTile {
    fn picked_tile(&self) -> LuaResult<Option<TooltipObject>> {
        Ok(self.0.picked_tile.borrow().clone())
    }

    fn square_coords(&self, square: &Value) -> Option<(i32, i32, i32)> {
        pz_lua_api::bind_world::square_coords(square)
    }
}

/// Installs the world's end of the mouse for the game that is starting.
pub fn install(
    render: Rc<RefCell<Option<WorldRender>>>,
    sprites: Rc<RefCell<IsoSpriteManager>>,
    cameras: SharedCameraFrame,
) {
    let mouse = Rc::new(EngineWorldMouse::new(render, sprites, cameras));
    with_ui(|m| m.world_mouse = Some(mouse.clone()));
    with_host(|h| h.picking = Box::new(PickedTile(mouse.clone())));
    bind_object_picker::set_object_picker(Some(mouse));
}

/// Takes it away again when the game ends.
pub fn uninstall() {
    with_ui(|m| m.world_mouse = None);
    with_host(|h| h.picking = Box::new(NoHud));
    bind_object_picker::set_object_picker(None);
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_entity::components::UiConfig;
    use pz_entity::Components;
    use pz_inventory::item_components::ItemComponents;
    use pz_world::objects::{IsoDoor, IsoThumpable};

    #[test]
    fn door_adjacency_matches_door_and_opposite_square() {
        let north_door = IsoObject::new(
            1,
            ObjectKind::Door(IsoDoor {
                north: true,
                ..Default::default()
            }),
            None,
        );
        assert!(door_is_adjacent_to_square((10, 20, 0), &north_door, (10, 20, 0)));
        assert!(door_is_adjacent_to_square((10, 20, 0), &north_door, (10, 19, 0)));
        assert!(!door_is_adjacent_to_square((10, 20, 0), &north_door, (10, 21, 0)));
        assert!(!door_is_adjacent_to_square((10, 20, 0), &north_door, (9, 20, 0)));
        assert!(!door_is_adjacent_to_square((10, 20, 0), &north_door, (10, 20, 1)));

        let west_thump_door = IsoObject::new(
            2,
            ObjectKind::Thumpable(Box::new(IsoThumpable {
                north: false,
                is_door: true,
                ..Default::default()
            })),
            None,
        );
        assert!(door_is_adjacent_to_square((10, 20, 0), &west_thump_door, (10, 20, 0)));
        assert!(door_is_adjacent_to_square((10, 20, 0), &west_thump_door, (9, 20, 0)));
        assert!(!door_is_adjacent_to_square((10, 20, 0), &west_thump_door, (10, 19, 0)));
    }

    #[test]
    fn interactive_entity_reflects_ui_config_enabled() {
        let mut enabled = IsoObject::new(1, ObjectKind::Object, None);
        enabled.components = ItemComponents::from_values_unowned(Components {
            list: vec![Component::UiConfig(UiConfig {
                ui_enabled: true,
                ..Default::default()
            })],
        })
        .unwrap();
        assert!(object_is_interactive_entity(&enabled));

        let mut disabled = IsoObject::new(2, ObjectKind::Object, None);
        disabled.components = ItemComponents::from_values_unowned(Components {
            list: vec![Component::UiConfig(UiConfig {
                ui_enabled: false,
                ..Default::default()
            })],
        })
        .unwrap();
        assert!(!object_is_interactive_entity(&disabled));

        let plain = IsoObject::new(3, ObjectKind::Object, None);
        assert!(!object_is_interactive_entity(&plain));
        drop((enabled, disabled, plain));
        pz_inventory::component_host::drain_retired_components();
    }
}
