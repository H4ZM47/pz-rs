//! `CellLoader.DoTileObjectCreation` and the `IsoGridSquare` lookups it
//! uses, plus `IsoGridSquare.FixStackableObjects`.
//!
//! A Java exception thrown while creating an object (a bad number in a
//! property, a null sprite) is returned as `Err` with the exception text;
//! `IsoCell.PlaceLot` catches it and blocks out the chunk.

use crate::chunk::IsoGridSquare;
use crate::java::{parse_float, parse_int, GameRand};
use crate::meta_grid::IsoMetaGrid;
use crate::object_slot::{IsoObjectRef, ObjectSlot};
use crate::objects::{
    DeviceData, IsoBarbecue, IsoCurtain, IsoDoor, IsoLightSource, IsoLightSwitch, IsoMannequin,
    IsoObject, IsoTree, IsoWindow, ItemContainer, ObjectId, ObjectKind, ObjectLoadHooks,
    ObjectLoadSettings, ObjectTables,
};
use crate::sprite::{IsoObjectType, IsoSprite, IsoSpriteManager, SpriteIdx};

/// A Java exception (its message).
pub type JavaResult<T> = Result<T, String>;

fn npe() -> String {
    "NullPointerException".to_string()
}

fn nfe(s: Option<&str>) -> String {
    match s {
        Some(s) => format!("NumberFormatException: For input string: \"{s}\""),
        None => "NumberFormatException: Cannot parse null string".to_string(),
    }
}

/// `Integer.parseInt(s)` with Java's failures.
fn int(s: Option<&str>) -> JavaResult<i32> {
    s.and_then(parse_int).ok_or_else(|| nfe(s))
}

/// `Float.parseFloat(s)` with Java's failures (`null` is an NPE).
fn float(s: Option<&str>) -> JavaResult<f32> {
    let s = s.ok_or_else(npe)?;
    parse_float(s).ok_or_else(|| nfe(Some(s)))
}

/// What object creation needs from the rest of the game.
pub struct ObjectEnv<'a> {
    pub sprites: &'a mut IsoSpriteManager,
    pub rand: &'a mut dyn GameRand,
    pub tables: &'a ObjectTables,
    /// Installed `BloodBodyPartType.MAX.index()` for native visual constructors.
    pub blood_body_parts: usize,
    pub settings: ObjectLoadSettings,
    pub hooks: &'a mut dyn ObjectLoadHooks,
}

/// Square lookups (`IsoGridSquare` methods) that need sprite data.
impl IsoGridSquare {
    fn spr<'m>(m: &'m IsoSpriteManager, o: &IsoObject) -> Option<&'m IsoSprite> {
        o.sprite.map(|s| &m.sprites[s])
    }

    /// `getFloor()`: index of the first object whose sprite is a
    /// `solidfloor`.
    pub fn floor_index(&self, m: &IsoSpriteManager) -> Option<usize> {
        let f = m.tables.flag.solidfloor;
        self.objects
            .iter()
            .map(ObjectSlot::borrow)
            .position(|o| Self::spr(m, &o).is_some_and(|s| s.properties.has(f)))
    }

    /// `getWall(north)`.
    pub fn wall_index(&self, m: &IsoSpriteManager, north: bool) -> Option<usize> {
        let se = m.tables.flag.WallSE;
        self.objects.iter().map(ObjectSlot::borrow).position(|o| {
            Self::spr(m, &o)
                .is_some_and(|s| !s.properties.has(se) && (s.cut_n && north || s.cut_w && !north))
        })
    }

    /// `getWallSE()`.
    pub fn wall_se_index(&self, m: &IsoSpriteManager) -> Option<usize> {
        let se = m.tables.flag.WallSE;
        self.objects
            .iter()
            .map(ObjectSlot::borrow)
            .position(|o| Self::spr(m, &o).is_some_and(|s| s.properties.has(se)))
    }

    /// `getWindow(GridSquareEdgeFacingDirection)`: the first `IsoWindow` in
    /// `specialObjects` facing that way (`north` is `NORTH_SOUTH`).
    pub fn window_index(&self, north: bool) -> Option<usize> {
        self.special_objects.iter().find_map(|&id| {
            let i = self.object_index(id)?;
            match &self.objects[i].borrow().kind {
                ObjectKind::Window(w) if w.north == north => Some(i),
                _ => None,
            }
        })
    }

    /// `getGarageDoor(north)`.
    pub fn garage_door_index(
        &self,
        m: &IsoSpriteManager,
        north: bool,
    ) -> JavaResult<Option<usize>> {
        for (i, o) in self.objects.iter().map(ObjectSlot::borrow).enumerate() {
            if garage_door_index(m, &o)? == -1 {
                continue;
            }
            if let ObjectKind::Door(d) = &o.kind {
                if d.north == north {
                    return Ok(Some(i));
                }
            }
        }
        Ok(None)
    }

    /// `getCountertopAttachObject()`.
    pub fn countertop_attach_index(&self, m: &IsoSpriteManager) -> Option<usize> {
        self.objects
            .iter()
            .map(ObjectSlot::borrow)
            .rposition(|o| Self::spr(m, &o).is_some_and(|s| m.has_prop(s, "CountertopAttach")))
    }

    /// Position of the object with identity `id` in `objects`.
    pub fn object_index(&self, id: ObjectId) -> Option<usize> {
        self.objects
            .iter()
            .map(ObjectSlot::borrow)
            .position(|o| o.id == id)
    }

    /// A fresh object identity for this square.
    pub fn new_object_id(&mut self) -> ObjectId {
        self.next_object_id += 1;
        self.next_object_id
    }

    /// `placeWallAndDoorCheck(obj, index)`.
    pub fn place_wall_and_door_check(
        &self,
        m: &IsoSpriteManager,
        obj: &IsoObject,
        index: i32,
    ) -> i32 {
        let Some(s) = Self::spr(m, obj) else {
            return index;
        };
        let ty = m.tables.ty;
        let is_door = |t: IsoObjectType| t == ty.doorN || t == ty.doorW;
        let wall_like = |s: &IsoSprite| {
            s.cut_w
                || s.cut_n
                || s.tile_type == ty.doorFrN
                || s.tile_type == ty.doorFrW
                || s.treat_as_wall_order
        };
        let find_walls = is_door(s.tile_type);
        let find_doors = !find_walls && wall_like(s);
        if !(find_doors || find_walls) {
            return index;
        }
        let mut needle = -1;
        for (i, other) in self.objects.iter().map(ObjectSlot::borrow).enumerate() {
            let Some(o) = Self::spr(m, &other) else {
                continue;
            };
            if find_doors && is_door(o.tile_type) {
                needle = i as i32;
            }
            if find_walls && wall_like(o) {
                needle = i as i32;
            }
        }
        if find_walls && needle > index {
            return needle + 1;
        }
        if find_doors && needle >= 0 && (needle < index || index < 0) {
            return needle;
        }
        index
    }

    /// `FixStackableObjects()`: swaps in stack-replacement sprites and
    /// offsets stacked table tops.
    pub fn fix_stackable_objects(&mut self, m: &IsoSpriteManager) {
        let mut table: Option<usize> = None;
        for i in 0..self.objects.len() {
            if matches!(self.objects[i].borrow().kind, ObjectKind::WorldInventory(_)) {
                continue;
            }
            let Some(mut s) = self.objects[i].borrow().sprite else {
                continue;
            };
            let mut info = m.sprites[s].properties.surface_info(&m.alias);
            if info.stack_replace_tile_offset != 0 {
                let id = m.sprites[s].id.wrapping_add(info.stack_replace_tile_offset);
                self.objects[i].borrow_mut().sprite = m.idx_by_id(id);
                let Some(s2) = self.objects[i].borrow().sprite else {
                    continue;
                };
                s = s2;
                info = m.sprites[s].properties.surface_info(&m.alias);
            }
            if info.is_table && !m.has_prop(&m.sprites[s], "Countertop") || info.is_table_top {
                let offset = if info.is_surface_offset {
                    info.surface as f32
                } else {
                    0.0
                };
                let y = match table {
                    Some(t) => {
                        let t = self.objects[t].borrow();
                        t.render_y_offset + surface_offset(m, &t) - offset
                    }
                    None => 0.0 - offset,
                };
                self.objects[i].borrow_mut().render_y_offset = y;
                self.objects[i].borrow_mut().sx = 0.0;
            }
            if info.is_table {
                table = Some(i);
            }
        }
    }
}

/// `IsoObject.getSurfaceOffset()`.
fn surface_offset(m: &IsoSpriteManager, o: &IsoObject) -> f32 {
    match o.sprite {
        Some(s) => {
            let info = m.sprites[s].properties.surface_info(&m.alias);
            if info.is_table {
                info.surface as f32
            } else {
                0.0
            }
        }
        None => 0.0,
    }
}

/// `IsoDoor.getGarageDoorIndex(obj)`.
fn garage_door_index(m: &IsoSpriteManager, o: &IsoObject) -> JavaResult<i32> {
    let Some(s) = o.sprite else {
        return Ok(-1);
    };
    let Some(v) = m.prop(&m.sprites[s], "GarageDoor") else {
        return Ok(-1);
    };
    let index = int(Some(v))?;
    if !(1..=6).contains(&index) {
        return Ok(-1);
    }
    let ObjectKind::Door(d) = &o.kind else {
        return Ok(-1);
    };
    if d.open {
        return Ok(if index >= 4 { index - 3 } else { -1 });
    }
    Ok(index)
}

/// `CellLoader.AddObject`: entity hook, then insert at the
/// `placeWallAndDoorCheck` index. Returns the object's position.
fn add_object(
    sq: &crate::SquareRef,
    env: &mut ObjectEnv<'_>,
    obj: IsoObject,
    special: bool,
) -> usize {
    let obj = ObjectSlot::new(obj);
    sq.borrow_mut()
        .retire_object(ObjectSlot::retain(&obj.ref_handle()));
    create_iso_entity_from_cell_loading(env, &obj.ref_handle());
    let mut sq = sq.borrow_mut();
    let size = sq.objects.len() as i32;
    let index = sq.place_wall_and_door_check(env.sprites, &obj.borrow(), size);
    if index != size && index >= 0 && index <= size {
        sq.objects.insert(index as usize, (obj).into());
        index as usize
    } else {
        if special {
            sq.special_objects.push(obj.borrow().id);
        }
        sq.objects.push((obj).into());
        sq.objects.len() - 1
    }
}

/// `GameEntityFactory.CreateIsoEntityFromCellLoading`: the property test is
/// ported; entity creation is the hook's.
fn create_iso_entity_from_cell_loading(env: &mut ObjectEnv<'_>, obj: &IsoObjectRef) {
    let Some(s) = obj.borrow().sprite else {
        return;
    };
    let m = &*env.sprites;
    let spr = &m.sprites[s];
    let wanted = spr.properties.has(m.tables.flag.EntityScript)
        || m.has_prop(spr, "IsMoveAble") && m.has_prop(spr, "CustomItem");
    if wanted && env.hooks.create_iso_entity(obj.clone(), env.rand) {
        obj.borrow_mut().has_components = true;
    }
}

/// `CellLoader.treatWindowFrameAsOverlay`.
fn treat_window_frame_as_overlay(
    m: &IsoSpriteManager,
    spr: SpriteIdx,
    sq: &mut IsoGridSquare,
) -> bool {
    let f = m.tables.flag;
    let sp = &m.sprites[spr].properties;
    for i in (0..sq.objects.len()).rev() {
        let o = sq.objects[i].borrow();
        let Some(os) = o.sprite.map(|s| &m.sprites[s]) else {
            continue;
        };
        let west = os.properties.has(f.cutW) && sp.has(f.WindowW);
        let north = os.properties.has(f.cutN) && sp.has(f.WindowN);
        if os.tile_type != m.tables.ty.wall || !west && !north {
            continue;
        }
        drop(o);
        sq.objects[i].borrow_mut().attach_anim_sprite(spr);
        return true;
    }
    false
}

/// `PropertyContainer.get(CONTAINER).equals(value)` behind a `container`
/// flag test.
fn container_is(m: &IsoSpriteManager, s: SpriteIdx, value: &str) -> JavaResult<bool> {
    let spr = &m.sprites[s];
    if !spr.properties.has(m.tables.flag.container) {
        return Ok(false);
    }
    Ok(m.prop(spr, "container").ok_or_else(npe)? == value)
}

/// `CellLoader.DoTileObjectCreation(spr, type, sq, cell, x, y, z, name)`.
pub fn do_tile_object_creation(
    env: &mut ObjectEnv<'_>,
    meta: &IsoMetaGrid,
    sq: &crate::SquareRef,
    spr: SpriteIdx,
    ty: IsoObjectType,
    name: &str,
) -> JavaResult<()> {
    let mut spr = spr;
    let mut ty = ty;
    let mut glass_removed_window = false;
    let mut smashed_window = false;
    if let Some(&s) = env.sprites.glass_removed_window_sprites.get(&spr) {
        spr = s;
        ty = env.sprites.sprites[s].tile_type;
        glass_removed_window = true;
    } else if let Some(&s) = env.sprites.smashed_window_sprites.get(&spr) {
        spr = s;
        ty = env.sprites.sprites[s].tile_type;
        smashed_window = true;
    }
    let f = env.sprites.tables.flag;
    let t = env.sprites.tables.ty;

    {
        let m = &*env.sprites;
        let sp = &m.sprites[spr];
        let props = &sp.properties;
        if sp.solidfloor && props.has(f.diamondFloor) && !props.has(f.transparentFloor) {
            let floor_index = sq.borrow().floor_index(m);
            if let Some(fi) = floor_index {
                let floor_owner = sq.borrow().objects[fi].ref_handle();
                let floor = floor_owner.borrow();
                if IsoGridSquare::spr(m, &floor).is_some_and(|s| s.properties.has(f.diamondFloor)) {
                    if floor.has_components {
                        // Its components are removed and the floor leaves
                        // the square (`removeFromSquare`).
                        let id = floor.id;
                        drop(floor);
                        sq.borrow_mut().remove_object_slot(fi);
                        sq.borrow_mut().special_objects.retain(|&x| x != id);
                    } else {
                        drop(floor);
                        let mut floor = floor_owner.borrow_mut();
                        if let Some(a) = &mut floor.attached_anim_sprite {
                            a.clear();
                        }
                        floor.sprite = Some(spr);
                        return Ok(());
                    }
                }
            }
        }
    }
    let m = &*env.sprites;
    let props = &m.sprites[spr].properties;
    let is_stove = ["stove", "toaster", "coffeemaker"]
        .iter()
        .any(|v| props.property_equals(&m.alias, "container", v));
    let id = sq.borrow_mut().new_object_id();

    // The object to finish (`obj != null` after the class switch).
    let obj_index: Option<usize>;
    if ty == t.doorW || ty == t.doorN {
        let door = new_door(env, spr, ty == t.doorN)?;
        let m = &*env.sprites;
        let props = &m.sprites[spr].properties;
        let has_double = props.has_name(&m.alias, "DoubleDoor");
        let has_garage = props.has_name(&m.alias, "GarageDoor");
        let mut obj = IsoObject::new(id, ObjectKind::Door(door), Some(spr));
        obj.outline_on_mouseover = true;
        let i = add_object(sq, env, obj, true);
        if let ObjectKind::Door(d) = &mut sq.borrow().objects[i].borrow_mut().kind {
            if has_double {
                d.locked = false;
                d.locked_by_key = false;
            }
            if has_garage {
                d.locked = !d.open;
                d.locked_by_key = false;
            }
        }
        obj_index = Some(i);
    } else if ty == t.lightswitch {
        let room_id = sq.borrow().room_id;
        let sw = new_light_switch(env, meta, sq, spr, room_id)?;
        let obj = IsoObject::new(id, ObjectKind::LightSwitch(sw), Some(spr));
        let i = add_object(sq, env, obj, false);
        let m = &*env.sprites;
        let sp = &m.sprites[spr];
        if m.has_prop(sp, "lightR") {
            let r = float(m.prop(sp, "lightR"))? / 255.0;
            let g = float(m.prop(sp, "lightG"))? / 255.0;
            let b = float(m.prop(sp, "lightB"))? / 255.0;
            let mut radius = 10;
            if m.has_prop(sp, "LightRadius") && int(m.prop(sp, "LightRadius"))? > 0 {
                radius = int(m.prop(sp, "LightRadius"))?;
            }
            let light = IsoLightSource {
                x: sq.borrow().x,
                y: sq.borrow().y,
                z: sq.borrow().z,
                r,
                g,
                b,
                radius,
                active: true,
                hydro_powered: true,
            };
            if let ObjectKind::LightSwitch(sw) = &mut sq.borrow().objects[i].borrow_mut().kind {
                sw.lights.push(light);
            }
        } else if let ObjectKind::LightSwitch(sw) = &mut sq.borrow().objects[i].borrow_mut().kind {
            sw.light_room = true;
        }
        obj_index = Some(i);
    } else if ty == t.curtainN || ty == t.curtainS || ty == t.curtainE || ty == t.curtainW {
        let suffix = &name[name.rfind('_').map_or(0, |i| i + 1)..];
        let closed = int(Some(suffix))? % 8 <= 3;
        let north = ty == t.curtainN || ty == t.curtainS;
        let (open_sprite, closed_sprite) = if closed {
            (m.sprite_with_offset(spr, 4)?, Some(spr))
        } else {
            (Some(spr), m.sprite_with_offset(spr, -4)?)
        };
        let c = IsoCurtain {
            north,
            open: true,
            health: 1000,
            max_health: 1000,
            pushed_strength: 2500,
            pushed_max_strength: 2500,
            open_sprite,
            closed_sprite,
            ..Default::default()
        };
        let mut obj = IsoObject::new(id, ObjectKind::Curtain(c), open_sprite);
        obj.outline_on_mouseover = true;
        obj_index = Some(add_object(sq, env, obj, true));
    } else if props.has(f.windowW) || props.has(f.windowN) {
        let north = props.has(f.windowN);
        let mut w = new_window(env, spr, north)?;
        let mut sprite = w.closed_sprite;
        if glass_removed_window {
            // setSmashed(true) then setGlassRemoved(true).
            w.destroyed = true;
            w.glass_removed = true;
            sprite = w.glass_removed_sprite;
        } else if smashed_window {
            w.destroyed = true;
            w.glass_removed = false;
            sprite = w.smashed_sprite;
        }
        // `gridSquare.getWall(north).rerouteCollide = this`.
        let m = &*env.sprites;
        let wall_se = m.tables.flag.WallSE;
        let wall = sq
            .borrow()
            .objects
            .iter()
            .map(ObjectSlot::borrow)
            .position(|o| {
                o.sprite.is_some_and(|s| {
                    let sp = &m.sprites[s];
                    !sp.properties.has(wall_se) && (sp.cut_n && north || sp.cut_w && !north)
                })
            });
        if let Some(i) = wall {
            sq.borrow().objects[i].borrow_mut().reroute_collide = Some(id);
        }
        let obj = IsoObject::new(id, ObjectKind::Window(w), sprite);
        obj_index = Some(add_object(sq, env, obj, true));
    } else if props.has(f.WindowW) || props.has(f.WindowN) {
        let north = props.has(f.WindowN);
        if !treat_window_frame_as_overlay(m, spr, &mut sq.borrow_mut()) {
            let obj = IsoObject::new(id, ObjectKind::WindowFrame { north }, Some(spr));
            obj_index = Some(add_object(sq, env, obj, false));
        } else {
            obj_index = None;
        }
    } else if container_is(m, spr, "barbecue")? || container_is(m, spr, "barbecuepropane")? {
        let mut obj = IsoObject::new(id, ObjectKind::Object, Some(spr));
        let b = new_barbecue(env, spr, &mut obj)?;
        obj.kind = ObjectKind::Barbecue(b);
        obj_index = Some(add_object(sq, env, obj, false));
    } else if container_is(m, spr, "fireplace")?
        || !matches!(
            m.sprites[spr].name.as_deref().ok_or_else(npe)?,
            "camping_01_04" | "camping_01_05" | "camping_01_06"
        ) && container_is(m, spr, "campfire")?
        || m.prop(&m.sprites[spr], "container") == Some("brazier")
    {
        // Fireplaces, campfires (except three camping tiles) and braziers.
        obj_index = Some(add_fireplace(sq, env, id, spr)?);
    } else if m.prop(&m.sprites[spr], "IsoType") == Some("IsoCombinationWasherDryer") {
        let obj = IsoObject::new(
            id,
            ObjectKind::CombinationWasherDryer(Default::default()),
            Some(spr),
        );
        obj_index = Some(add_object(sq, env, obj, false));
    } else if container_is(m, spr, "clothingdryer")? {
        let obj = IsoObject::new(id, ObjectKind::ClothingDryer(Default::default()), Some(spr));
        obj_index = Some(add_object(sq, env, obj, false));
    } else if container_is(m, spr, "clothingwasher")? {
        let obj = IsoObject::new(
            id,
            ObjectKind::ClothingWasher(Default::default()),
            Some(spr),
        );
        obj_index = Some(add_object(sq, env, obj, false));
    } else if container_is(m, spr, "woodstove")? {
        obj_index = Some(add_fireplace(sq, env, id, spr)?);
    } else if props.has(f.container) && (is_stove || container_is(m, spr, "microwave")?) {
        let obj = IsoObject::new(id, ObjectKind::Stove(Default::default()), Some(spr));
        obj_index = Some(add_object(sq, env, obj, false));
    } else if ty == t.jukebox {
        let mut obj = IsoObject::new(id, ObjectKind::Jukebox, Some(spr));
        obj.outline_on_mouseover = true;
        obj_index = Some(add_object(sq, env, obj, false));
    } else if ty == t.radio {
        let dd = wave_signal_init(env, spr, true);
        let obj = IsoObject::new(id, ObjectKind::Radio(dd), Some(spr));
        obj_index = Some(add_object(sq, env, obj, false));
    } else if props.has_name(&m.alias, "signal") {
        let kind = match m.prop(&m.sprites[spr], "signal") {
            Some("radio") => Some(true),
            Some("tv") => Some(false),
            _ => None,
        };
        // Any other value adds a null object: an NPE in AddObject.
        let radio = kind.ok_or_else(npe)?;
        let dd = wave_signal_init(env, spr, radio);
        let kind = if radio {
            ObjectKind::Radio(dd)
        } else {
            ObjectKind::Television(dd)
        };
        let obj = IsoObject::new(id, kind, Some(spr));
        obj_index = Some(add_object(sq, env, obj, false));
    } else if props.has_name(&m.alias, "Countertop") {
        let countertop = sq.borrow().countertop_attach_index(m);
        if let Some(ci) = countertop {
            sq.borrow().objects[ci].borrow_mut().attach_anim_sprite(spr);
            obj_index = None;
        } else {
            let obj = IsoObject::new(id, ObjectKind::Object, Some(spr));
            obj_index = Some(add_object(sq, env, obj, false));
        }
    } else if props.has(f.WallOverlay) {
        let target = if props.has(f.attachedSE) {
            sq.borrow().wall_se_index(m)
        } else if props.has(f.attachedW) || props.has(f.attachedN) {
            let north = !props.has(f.attachedW);
            let (window_flag, mut found) = if north {
                (f.WindowN, sq.borrow().wall_index(m, true))
            } else {
                (f.WindowW, sq.borrow().wall_index(m, false))
            };
            if found.is_none() {
                // A window only counts when its sprite has the window
                // frame flag.
                found = sq.borrow().window_index(north).filter(|&w| {
                    IsoGridSquare::spr(m, &sq.borrow().objects[w].borrow())
                        .is_some_and(|s| s.properties.has(window_flag))
                });
            }
            if found.is_none() {
                found = sq.borrow().garage_door_index(m, north)?;
            }
            found
        } else {
            let mut found = None;
            for n in (0..sq.borrow().objects.len()).rev() {
                let s = IsoGridSquare::spr(m, &sq.borrow().objects[n].borrow()).ok_or_else(npe)?;
                if s.properties.has(f.cutW) || s.properties.has(f.cutN) {
                    found = Some(n);
                    break;
                }
            }
            found
        };
        match target {
            Some(i) => sq.borrow().objects[i].borrow_mut().attach_anim_sprite(spr),
            None => {
                let obj = IsoObject::new(id, ObjectKind::Object, Some(spr));
                add_object(sq, env, obj, false);
            }
        }
        return Ok(());
    } else if props.has(f.FloorOverlay) {
        if let Some(fi) = sq.borrow().floor_index(m) {
            sq.borrow().objects[fi].borrow_mut().attach_anim_sprite(spr);
        }
        obj_index = None;
    } else if m.prop(&m.sprites[spr], "CustomName") == Some("Mannequin") {
        let obj = IsoObject::new(
            id,
            ObjectKind::Mannequin(Box::new(IsoMannequin::new(env.blood_body_parts))),
            Some(spr),
        );
        obj_index = Some(add_object(sq, env, obj, false));
    } else if ty == t.tree {
        let sname = m.sprites[spr].name.as_deref();
        if sname.is_some_and(|n| n.starts_with("vegetation_trees")) {
            let natural = sq.borrow().floor_index(m).is_some_and(|fi| {
                IsoGridSquare::spr(m, &sq.borrow().objects[fi].borrow())
                    .and_then(|s| s.name.as_deref())
                    .is_some_and(|n| n.starts_with("blends_natural"))
            });
            if !natural {
                return Ok(());
            }
        }
        let tree = init_tree(env, spr)?;
        let obj = IsoObject::new(id, ObjectKind::Tree(tree), Some(spr));
        let old_tree = sq
            .borrow()
            .objects
            .iter()
            .map(ObjectSlot::borrow)
            .position(|o| matches!(o.kind, ObjectKind::Tree(_)));
        if let Some(i) = old_tree {
            sq.borrow_mut().remove_object_slot(i);
        }
        obj_index = Some(add_object(sq, env, obj, false));
    } else {
        // The client's missing-texture fallback always finds
        // `missing-tile.png`, so the object is created either way.
        let obj = IsoObject::new(id, ObjectKind::Object, Some(spr));
        obj_index = Some(add_object(sq, env, obj, false));
    }

    let Some(i) = obj_index else {
        return Ok(());
    };
    sq.borrow().objects[i].borrow_mut().tile = Some(name.to_string());
    create_containers_from_sprite_properties(
        env.sprites,
        &mut sq.borrow().objects[i].borrow_mut(),
    )?;
    let s = sq.borrow().objects[i].borrow().sprite.ok_or_else(npe)?;
    if env.sprites.sprites[s].properties.has(f.vegitation) {
        let object = sq.borrow().objects[i].ref_handle();
        let mut o = object.borrow_mut();
        o.tint_r = 0.7 + env.rand.next(30) as f32 / 100.0;
        o.tint_g = 0.7 + env.rand.next(30) as f32 / 100.0;
        o.tint_b = 0.7 + env.rand.next(30) as f32 / 100.0;
    }
    if should_not_spawn_activated_radios_or_tvs(meta, &sq.borrow()) {
        if let ObjectKind::Radio(d) | ObjectKind::Television(d) =
            &mut sq.borrow().objects[i].borrow_mut().kind
        {
            if d.is_turned_on {
                d.set_turned_on_raw(false);
            }
        }
    }
    Ok(())
}

/// `IsoGridSquare.shouldNotSpawnActivatedRadiosOrTvs()`.
fn should_not_spawn_activated_radios_or_tvs(meta: &IsoMetaGrid, sq: &IsoGridSquare) -> bool {
    sq.room_def(meta)
        .is_some_and(|r| r.name.to_lowercase().contains("radiofactory"))
}

/// `new IsoFireplace(cell, sq, spr)` added with `AddObject`.
fn add_fireplace(
    sq: &crate::SquareRef,
    env: &mut ObjectEnv<'_>,
    id: ObjectId,
    spr: SpriteIdx,
) -> JavaResult<usize> {
    let mut obj = IsoObject::new(id, ObjectKind::Fireplace(Default::default()), Some(spr));
    ctor_container(env.sprites, spr, &mut obj, "fireplace")?;
    Ok(add_object(sq, env, obj, false))
}

/// The container `IsoBarbecue` / `IsoFireplace` constructors create when
/// the sprite has a `ContainerCapacity`.
fn ctor_container(
    m: &IsoSpriteManager,
    spr: SpriteIdx,
    obj: &mut IsoObject,
    default: &str,
) -> JavaResult<()> {
    let sp = &m.sprites[spr];
    let kind = if sp.properties.has(m.tables.flag.container) {
        m.prop(sp, "container")
    } else {
        Some(default)
    };
    if let Some(cap) = m.prop(sp, "ContainerCapacity") {
        let mut c = ItemContainer::new(kind);
        c.capacity = int(Some(cap))?;
        obj.container = Some(c);
    }
    Ok(())
}

/// `new IsoBarbecue(cell, sq, spr)`.
fn new_barbecue(
    env: &mut ObjectEnv<'_>,
    spr: SpriteIdx,
    obj: &mut IsoObject,
) -> JavaResult<IsoBarbecue> {
    ctor_container(env.sprites, spr, obj, "barbecue")?;
    let mut b = IsoBarbecue {
        has_propane_tank: false,
        fuel_amount_minutes: 0,
        normal_sprite: None,
        no_tank_sprite: None,
        ..Default::default()
    };
    let m = &*env.sprites;
    let tank = |s: Option<SpriteIdx>| s.is_some_and(|s| m.has_prop(&m.sprites[s], "propaneTank"));
    if tank(Some(spr)) && tank(m.sprite_with_offset(spr, 8)?) {
        b.has_propane_tank = true;
        b.fuel_amount_minutes = env.rand.next(1201);
        b.normal_sprite = Some(spr);
        b.no_tank_sprite = env.sprites.sprite_with_offset(spr, 8)?;
    } else if tank(Some(spr)) && tank(m.sprite_with_offset(spr, -8)?) {
        b.normal_sprite = m.sprite_with_offset(spr, -8)?;
        b.no_tank_sprite = Some(spr);
    }
    Ok(b)
}

/// `new IsoDoor(cell, sq, spr, north)`.
fn new_door(env: &mut ObjectEnv<'_>, spr: SpriteIdx, north: bool) -> JavaResult<IsoDoor> {
    let m = &*env.sprites;
    let sp = &m.sprites[spr];
    let open = sp.properties.has(m.tables.flag.open);
    let mut open_offset = 2;
    if m.has_prop(sp, "DoubleDoor") {
        open_offset = 4;
    }
    if m.has_prop(sp, "GarageDoor") {
        open_offset = 8;
    }
    let (closed_sprite, open_sprite) = if open {
        (m.sprite_with_offset(spr, -open_offset)?, Some(spr))
    } else {
        (Some(spr), m.sprite_with_offset(spr, open_offset)?)
    };
    // `type` is always `DoorType.WeakWooden`, the first constant.
    let (mut health, mut max_health) = (500, 500);
    if m.has_prop(sp, "forceLocked") {
        health = 2000;
        max_health = 2000;
    }
    if sp.name().contains("fences") {
        health = 100;
        max_health = 100;
    }
    let rand_lock = match env.settings.locked_houses {
        1 => -1,
        2 => 5,
        3 => 10,
        4 => 50,
        5 => 60,
        6 => 70,
        _ => 69,
    };
    let (mut locked, mut locked_by_key) = (false, false);
    if rand_lock > -1 {
        locked = env.rand.next(100) < rand_lock;
        if locked && env.rand.next(3) == 0 {
            locked_by_key = true;
        }
    }
    if m.has_prop(sp, "forceLocked") {
        locked = true;
        locked_by_key = true;
    }
    if open {
        locked = false;
        locked_by_key = false;
    }
    Ok(IsoDoor {
        north,
        open,
        locked,
        locked_by_key,
        health,
        max_health,
        pushed_strength: 2500,
        pushed_max_strength: 2500,
        open_sprite,
        closed_sprite,
        ..Default::default()
    })
}

/// `new IsoWindow(cell, sq, spr, north)`. Changes the sprites it touches,
/// like the original: the closed sprite gets its cut/window flags reset and
/// the open, smashed and glass-removed sprites get its properties.
/// `getWall(north).rerouteCollide` (collision state) is not ported.
fn new_window(env: &mut ObjectEnv<'_>, spr: SpriteIdx, north: bool) -> JavaResult<IsoWindow> {
    let m = &mut *env.sprites;
    let f = m.tables.flag;
    m.sprites[spr].properties.unset_flag(f.cutN);
    m.sprites[spr].properties.unset_flag(f.cutW);
    let sp = &m.sprites[spr];
    let open_offset = if m.has_prop(sp, "OpenTileOffset") {
        int(m.prop(sp, "OpenTileOffset"))?
    } else {
        0
    };
    let perma_locked = m.has_prop(sp, "WindowLocked");
    let smashed_offset = if m.has_prop(sp, "SmashedTileOffset") {
        int(m.prop(sp, "SmashedTileOffset"))?
    } else {
        0
    };
    let closed = spr;
    if north {
        m.sprites[closed].properties.set_flag(f.cutN);
        m.sprites[closed].properties.set_flag(f.windowN);
    } else {
        m.sprites[closed].properties.set_flag(f.cutW);
        m.sprites[closed].properties.set_flag(f.windowW);
    }
    let open_sprite = m.sprite_with_offset(spr, open_offset)?;
    let smashed_sprite = m.sprite_with_offset(spr, smashed_offset)?;
    let glass_removed_sprite = if m.has_prop(&m.sprites[closed], "GlassRemovedOffset") {
        let off = int(m.prop(&m.sprites[closed], "GlassRemovedOffset"))?;
        m.sprite_with_offset(closed, off)?
    } else {
        smashed_sprite
    };
    for other in [smashed_sprite, open_sprite, glass_removed_sprite] {
        if let Some(o) = other.filter(|&o| o != closed) {
            let props = m.sprites[closed].properties.clone();
            let tile_type = m.sprites[closed].tile_type;
            m.sprites[o].properties.add_properties(&props);
            m.sprites[o].tile_type = tile_type;
        }
    }
    let chance = env.tables.window_lock_chance(env.settings.locked_houses);
    let mut locked = false;
    if chance > env.tables.never_lock_chance() {
        locked = env.rand.next(100) < chance;
    }
    // `type` is always `WindowType.SinglePane`, the first constant.
    Ok(IsoWindow {
        north,
        locked,
        perma_locked,
        open: false,
        destroyed: false,
        glass_removed: false,
        health: 50,
        max_health: 50,
        open_sprite,
        closed_sprite: Some(closed),
        smashed_sprite,
        glass_removed_sprite,
    })
}

/// `new IsoLightSwitch(cell, sq, spr, roomId)`.
fn new_light_switch(
    env: &mut ObjectEnv<'_>,
    meta: &IsoMetaGrid,
    sq: &crate::SquareRef,
    spr: SpriteIdx,
    room_id: i64,
) -> JavaResult<IsoLightSwitch> {
    let m = &*env.sprites;
    let sp = &m.sprites[spr];
    let mut sw = IsoLightSwitch {
        activated: false,
        light_room: false,
        room_id,
        street_light: false,
        can_be_modified: false,
        primary_r: 1.0,
        primary_g: 1.0,
        primary_b: 1.0,
        lights: Vec::new(),
        ..Default::default()
    };
    if m.has_prop(sp, "lightR") {
        if m.has_prop(sp, "IsMoveAble") {
            sw.can_be_modified = true;
        }
        sw.primary_r = float(m.prop(sp, "lightR"))? / 255.0;
        sw.primary_g = float(m.prop(sp, "lightG"))? / 255.0;
        sw.primary_b = float(m.prop(sp, "lightB"))? / 255.0;
    } else {
        sw.light_room = true;
    }
    sw.street_light = m.has_prop(sp, "streetlight");
    let room = sq.borrow().room_def(meta);
    match room {
        Some(room) if sw.light_room => {
            if !env.hooks.square_powered(sq.clone(), room) {
                room.lights_active.set(false);
            }
            sw.activated = room.lights_active.get();
        }
        _ => sw.activated = true,
    }
    Ok(sw)
}

/// `IsoTree.getNew()` + `initTree()`. `setType(tree)` sets the sprite's
/// tile type.
fn init_tree(env: &mut ObjectEnv<'_>, spr: SpriteIdx) -> JavaResult<IsoTree> {
    let tree_ty = env.sprites.tables.ty.tree;
    env.sprites.sprites[spr].tile_type = tree_ty;
    let m = &*env.sprites;
    let size = match m.prop(&m.sprites[spr], "tree") {
        Some(v) => int(Some(v))?.clamp(1, 8),
        None => 4,
    };
    let log_yield = env.tables.logs_per_size[(size - 1) as usize];
    let damage = ((log_yield - 1) * 80).max(40);
    Ok(IsoTree {
        size,
        log_yield,
        damage,
    })
}

/// `IsoWaveSignal.init(false)`.
fn wave_signal_init(env: &mut ObjectEnv<'_>, spr: SpriteIdx, radio: bool) -> DeviceData {
    let m = &*env.sprites;
    let item = m.prop(&m.sprites[spr], "CustomItem").map(str::to_string);
    let mut dd = item.and_then(|item| env.hooks.device_data_from_item(&item, env.rand));
    if let Some(d) = &mut dd {
        generate_presets(d, &*env.hooks, env.rand);
        let v = env.rand.next_float_range(0.1, 1.0);
        d.set_device_volume(v);
        set_random_channel(d, env.rand);
        if env.rand.next(100) <= 35 && !env.settings.tutorial {
            d.set_turned_on_raw(true);
            if radio {
                d.set_initial_power(env.settings.minutes_stamp);
                if d.is_battery_powered && d.power_delta <= 0.0 {
                    d.set_turned_on_raw(false);
                }
            }
        }
    }
    dd.unwrap_or_default()
}

/// `ZomboidRadio.GetChannelList` through the object-load hooks.
struct HookChannels<'a>(&'a dyn ObjectLoadHooks);
impl pz_inventory::device::ChannelLists for HookChannels<'_> {
    fn channel_list(&self, category: &str) -> Option<Vec<(i32, String)>> {
        self.0.channel_list(category)
    }
}

/// `DeviceData.generatePresets()`.
fn generate_presets(d: &mut DeviceData, hooks: &dyn ObjectLoadHooks, rand: &mut dyn GameRand) {
    d.generate_presets(&HookChannels(hooks), &mut |b| rand.next(b));
}

/// `DeviceData.setRandomChannel()`.
fn set_random_channel(d: &mut DeviceData, rand: &mut dyn GameRand) {
    d.set_random_channel(&mut |b| rand.next(b));
}

/// `IsoObject.createContainersFromSpriteProperties()`.
pub(crate) fn create_containers_from_sprite_properties(
    m: &IsoSpriteManager,
    o: &mut IsoObject,
) -> JavaResult<()> {
    let Some(s) = o.sprite else {
        return Ok(());
    };
    if o.container.is_some() {
        return Ok(());
    }
    let sp = &m.sprites[s];
    let kind = m.prop(sp, "container");
    if matches!(
        kind,
        Some("fruitbusha" | "fruitbushb" | "fruitbushc" | "fruitbushd" | "fruitbushe" | "corn")
    ) {
        return Ok(());
    }
    if sp.properties.has(m.tables.flag.container) {
        let mut c = ItemContainer::new(kind);
        o.outline_on_mouseover = true;
        if m.has_prop(sp, "ContainerCapacity") {
            c.capacity = int(m.prop(sp, "ContainerCapacity"))?;
        }
        if m.has_prop(sp, "ContainerPosition") {
            c.container_position = m.prop(sp, "ContainerPosition").map(str::to_string);
        }
        o.container = Some(c);
    }
    if m.has_prop(sp, "Freezer") {
        let mut freezer = ItemContainer::new(Some("freezer"));
        freezer.capacity = if m.has_prop(sp, "FreezerCapacity") {
            int(m.prop(sp, "FreezerCapacity"))?
        } else {
            15
        };
        if m.has_prop(sp, "FreezerPosition") {
            freezer.freezer_position = m.prop(sp, "FreezerPosition").map(str::to_string);
        }
        if o.container.is_none() {
            o.container = Some(freezer);
        } else {
            o.secondary_containers.push(freezer);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
