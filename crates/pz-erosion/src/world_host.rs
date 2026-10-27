//! [`ErosionHost`] over `pz-world`: a chunk's squares and objects and the
//! sprite manager.
//!
//! Square flags and edge queries are `pz_world::square`'s, so squares must
//! have had `recalc_properties` run, as the original's chunk load does
//! before `ErosionMain.LoadGridsquare`. Squares outside the chunk come
//! from [`WorldView`].
//!
//! This legacy host has infallible query methods. Sprite metadata reads require
//! an unborrowed actual body, as the square/object queries already do; a Native
//! sprite is read directly and never treated as a missing managed tile.

use pz_inventory::dictionary::WorldDictionary;
use pz_world::object_slot::{IsoObjectRef, ObjectSlot};
use pz_world::objects::{IsoObject, IsoSpriteInstance, IsoTree, ObjectKind, ObjectName};
use pz_world::square::{GetSquare, NoSquareHooks, SquareHooks};
use pz_world::{IsoChunk, IsoSpriteManager};

use crate::host::{Attached, ErosionHost, ObjId, Sq, SqFlag};
use crate::ice_queen::get_sprite;

#[cfg(test)]
mod native_sprite_tests;

/// What erosion reads beyond the chunk it processes: squares in other
/// chunks (neighbours across the chunk edge) and the object state
/// `pz_world::square` delegates through [`SquareHooks`].
#[derive(Clone, Copy)]
pub struct WorldView<'a> {
    /// Squares outside the chunk (usually the `IsoCell`); squares inside
    /// it are always read from the chunk itself.
    pub outside: &'a dyn GetSquare,
    pub hooks: &'a dyn SquareHooks,
    /// `WorldDictionary`, for object names saved as dictionary ids.
    pub dictionary: Option<&'a WorldDictionary>,
}

/// [`GetSquare`] that has no squares.
pub struct NoSquares;

impl GetSquare for NoSquares {
    fn get_grid_square(&self, _x: i32, _y: i32, _z: i32) -> Option<pz_world::SquareRef> {
        None
    }
}

impl WorldView<'_> {
    /// Only the chunk's own squares; no barricades, thumpables or
    /// dictionary.
    pub const CHUNK_ONLY: WorldView<'static> = WorldView {
        outside: &NoSquares,
        hooks: &NoSquareHooks,
        dictionary: None,
    };
}

/// The chunk's squares, then [`WorldView::outside`] for the rest.
struct HostSquares<'a> {
    chunk: &'a IsoChunk,
    outside: &'a dyn GetSquare,
}

impl GetSquare for HostSquares<'_> {
    fn get_grid_square(&self, x: i32, y: i32, z: i32) -> Option<pz_world::SquareRef> {
        let (lx, ly) = (x - self.chunk.wx * 8, y - self.chunk.wy * 8);
        if (0..8).contains(&lx) && (0..8).contains(&ly) {
            if (-32..=31).contains(&z) {
                self.chunk.get_grid_square(lx, ly, z)
            } else {
                None
            }
        } else {
            self.outside.get_grid_square(x, y, z)
        }
    }
}

/// A chunk being processed by the erosion system.
pub struct ChunkHost<'a> {
    pub chunk: &'a mut IsoChunk,
    pub sprites: &'a mut IsoSpriteManager,
    pub world: &'a WorldView<'a>,
    /// `IsoTree.LOGS_PER_SIZE` (`pz_world::objects::ObjectTables`), for
    /// trees erosion creates.
    pub logs_per_size: Option<&'a [i32]>,
}

impl ChunkHost<'_> {
    fn squares(&self) -> HostSquares<'_> {
        HostSquares {
            chunk: self.chunk,
            outside: self.world.outside,
        }
    }

    /// A square of the chunk or, through [`WorldView::outside`], next to it.
    fn any_square(&self, sq: Sq) -> Option<pz_world::SquareRef> {
        let (x, y, z) = self.local(sq);
        if (0..8).contains(&x) && (0..8).contains(&y) {
            self.square(sq).filter(|_| (-32..=31).contains(&z))
        } else {
            self.world.outside.get_grid_square(sq.0, sq.1, sq.2)
        }
    }

    fn local(&self, sq: Sq) -> (i32, i32, i32) {
        (sq.0 - self.chunk.wx * 8, sq.1 - self.chunk.wy * 8, sq.2)
    }

    pub fn square(&self, sq: Sq) -> Option<pz_world::SquareRef> {
        let (x, y, z) = self.local(sq);
        self.chunk.get_grid_square(x, y, z)
    }

    pub fn square_mut(&mut self, sq: Sq) -> Option<pz_world::SquareRef> {
        let (x, y, z) = self.local(sq);
        self.chunk.get_grid_square_mut(x, y, z)
    }

    fn obj(&self, sq: Sq, o: ObjId) -> Option<IsoObjectRef> {
        let square = self.square(sq)?;
        let object = square
            .borrow()
            .objects
            .iter()
            .find(|x| x.borrow().id == o)
            .map(ObjectSlot::ref_handle);
        object
    }

    fn sprite_metadata(&self, sq: Sq, o: ObjId) -> Option<pz_world::sprite::IsoSprite> {
        let owner = self.obj(sq, o)?;
        let object = owner.borrow();
        object
            .sprite
            .metadata_projection(self.sprites)
            .expect("legacy erosion requires unborrowed actual sprite metadata")
    }

    fn remove(&mut self, sq: Sq, o: ObjId, special_too: bool) {
        if let Some(owner) = self.square_mut(sq) {
            let mut s = owner.borrow_mut();
            s.retain_objects(|x| x.borrow().id != o);
            if special_too {
                s.special_objects.retain(|&x| x != o);
            }
        }
    }
}

impl ErosionHost for ChunkHost<'_> {
    fn square_exists(&self, sq: Sq) -> bool {
        self.any_square(sq).is_some()
    }

    fn has_flag(&self, sq: Sq, flag: SqFlag) -> bool {
        let f = self.sprites.tables.flag;
        let flag = match flag {
            SqFlag::Exterior => f.exterior,
            SqFlag::CutN => f.cutN,
            SqFlag::CutW => f.cutW,
            SqFlag::CanPathN => f.canPathN,
            SqFlag::CanPathW => f.canPathW,
        };
        self.any_square(sq).is_some_and(|s| s.borrow().has(flag))
    }

    fn has_wall_type(&self, sq: Sq) -> bool {
        let wall = self.sprites.tables.ty.wall;
        self.any_square(sq)
            .is_some_and(|s| s.borrow().has_type(wall))
    }

    fn objects(&self, sq: Sq) -> Vec<ObjId> {
        self.square(sq).map_or_else(Vec::new, |s| {
            s.borrow().objects.iter().map(|o| o.borrow().id).collect()
        })
    }

    fn floor(&self, sq: Sq) -> Option<ObjId> {
        let square = self.square(sq)?;
        let s = square.borrow();
        s.floor_index(self.sprites)
            .map(|i| s.objects[i].borrow().id)
    }

    fn wall(&self, sq: Sq, north: bool) -> Option<ObjId> {
        let square = self.square(sq)?;
        let s = square.borrow();
        s.wall_index(self.sprites, north)
            .map(|i| s.objects[i].borrow().id)
    }

    fn is_wall_to(&self, sq: Sq, other: Sq) -> bool {
        let g = self.squares();
        let other = self.any_square(other);
        let other = other.as_ref().map(|s| s.borrow());
        self.any_square(sq)
            .is_some_and(|s| s.borrow().is_wall_to(other.as_deref(), self.sprites, &g))
    }

    fn is_window_to(&self, sq: Sq, other: Sq) -> bool {
        let g = self.squares();
        let other = self.any_square(other);
        let other = other.as_ref().map(|s| s.borrow());
        self.any_square(sq)
            .is_some_and(|s| s.borrow().is_window_to(other.as_deref(), self.sprites, &g))
    }

    fn is_window_blocked_to(&self, sq: Sq, other: Sq) -> bool {
        let g = self.squares();
        let other = self.any_square(other);
        let other = other.as_ref().map(|s| s.borrow());
        self.any_square(sq).is_some_and(|s| {
            s.borrow()
                .is_window_blocked_to(other.as_deref(), &g, self.world.hooks)
        })
    }

    fn window_to(&self, sq: Sq, other: Sq) -> Option<ObjId> {
        // Erosion only asks about the north and west edges, whose windows
        // belong to `sq`.
        let g = self.squares();
        let square = self.square(sq)?;
        let s = square.borrow();
        let other = self.any_square(other);
        let other = other.as_ref().map(|s| s.borrow());
        let w = s.get_window_to(other.as_deref(), &g)?;
        s.objects
            .iter()
            .any(|o| o.ptr_eq_ref(&w))
            .then(|| w.borrow().id)
    }

    fn sprite_name(&self, sq: Sq, o: ObjId) -> Option<String> {
        self.sprite_metadata(sq, o)?.name
    }

    fn sprite_tile_sheet_index(&self, sq: Sq, o: ObjId) -> i32 {
        self.sprite_metadata(sq, o)
            .map_or(0, |s| s.tile_sheet_index)
    }

    fn sprite_custom_name(&self, sq: Sq, o: ObjId) -> Option<String> {
        let s = self.sprite_metadata(sq, o)?;
        self.sprites.prop(&s, "CustomName").map(str::to_string)
    }

    fn named_sprite_custom_name(&self, name: &str) -> Option<Option<String>> {
        let spr = self.sprites.by_name(name)?;
        Some(self.sprites.prop(spr, "CustomName").map(str::to_string))
    }

    fn object_name(&self, sq: Sq, o: ObjId) -> Option<String> {
        let owner = self.obj(sq, o)?;
        let object = owner.borrow();
        match object.name.as_ref()? {
            ObjectName::Str(s) => Some(s.clone()),
            ObjectName::Grass => Some("Grass".to_string()),
            ObjectName::Dict(id) => self
                .world
                .dictionary
                .and_then(|d| d.object_name(*id))
                .map(str::to_string),
        }
    }

    fn set_object_name(&mut self, sq: Sq, o: ObjId, name: Option<&str>) {
        if let Some(owner) = self.obj(sq, o) {
            let mut obj = owner.borrow_mut();
            obj.name = name.map(|n| ObjectName::Str(n.to_string()));
            obj.do_not_sync = true;
        }
    }

    fn set_object_sprite(&mut self, sq: Sq, o: ObjId, sprite: &str) {
        let s = get_sprite(self.sprites, sprite);
        if let Some(owner) = self.obj(sq, o) {
            let mut obj = owner.borrow_mut();
            let mut retired = pz_entity::RetiredComponents::new();
            if let Some(sprite) = obj.sprite.native_slot() {
                retired.push_erased(sprite.clone());
            }
            obj.sprite = Some(s).into();
            obj.do_not_sync = true;
            drop(obj);
            pz_inventory::component_host::retire_components(retired);
        }
    }

    fn attached(&self, sq: Sq, o: ObjId) -> Option<Vec<Attached>> {
        let owner = self.obj(sq, o)?;
        let obj = owner.borrow();
        let list = obj.attached_anim_sprite.as_ref()?;
        Some(
            list.iter()
                .map(|i| {
                    let s = &self.sprites.sprites[i.sprite];
                    Attached {
                        name: s.name.clone(),
                        id: s.id,
                    }
                })
                .collect(),
        )
    }

    fn attached_remove(&mut self, sq: Sq, o: ObjId, index: usize) {
        if let Some(owner) = self.obj(sq, o) {
            let mut obj = owner.borrow_mut();
            let Some(list) = obj.attached_anim_sprite.as_mut() else {
                return;
            };
            if index < list.len() {
                list.remove(index);
            }
        }
    }

    fn attached_clear(&mut self, sq: Sq, o: ObjId) {
        if let Some(owner) = self.obj(sq, o) {
            let mut obj = owner.borrow_mut();
            obj.attached_anim_sprite
                .get_or_insert_with(Vec::new)
                .clear();
        }
    }

    fn attached_add(&mut self, sq: Sq, o: ObjId, sprite: &str) -> i32 {
        let s = get_sprite(self.sprites, sprite);
        let id = self.sprites.sprites[s].id;
        if let Some(owner) = self.obj(sq, o) {
            let mut obj = owner.borrow_mut();
            obj.attached_anim_sprite
                .get_or_insert_with(Vec::new)
                .push(IsoSpriteInstance::new(s));
        }
        id
    }

    fn remove_tile_object(&mut self, sq: Sq, o: ObjId) {
        self.remove(sq, o, true);
    }

    fn remove_tile_object_erosion_no_recalc(&mut self, sq: Sq, o: ObjId) {
        self.remove(sq, o, true);
    }

    fn remove_object_raw(&mut self, sq: Sq, o: ObjId) {
        self.remove(sq, o, false);
    }

    fn add_object(&mut self, sq: Sq, sprite: &str, name: &str, tree: bool) -> ObjId {
        let named = self.sprites.named_map.get(sprite).copied();
        let tree_type = self.sprites.tables.ty.tree;
        let kind = if tree {
            // `initTree`: the size comes from the sprite's `tree` property;
            // `setType(tree)` changes the shared sprite's tile type.
            let size = match named {
                Some(s) => {
                    self.sprites.sprites[s].tile_type = tree_type;
                    match self.sprites.prop(&self.sprites.sprites[s], "tree") {
                        Some(v) => crate::java::parse_int(v).unwrap_or(4).clamp(1, 8),
                        None => 4,
                    }
                }
                None => 4,
            };
            let log_yield = self
                .logs_per_size
                .and_then(|t| t.get(size as usize - 1).copied())
                .unwrap_or(0);
            ObjectKind::Tree(IsoTree {
                size,
                log_yield,
                damage: ((log_yield - 1) * 80).max(40),
            })
        } else {
            ObjectKind::Object
        };
        let sprites: &IsoSpriteManager = self.sprites;
        let Some(square) = ({
            let (x, y, z) = (sq.0 - self.chunk.wx * 8, sq.1 - self.chunk.wy * 8, sq.2);
            self.chunk.get_grid_square_mut(x, y, z)
        }) else {
            return 0;
        };
        let id = square.borrow_mut().new_object_id();
        let mut obj = IsoObject::new(id, kind, named);
        if !tree {
            obj.tile = Some(sprite.to_string());
        }
        obj.name = Some(ObjectName::Str(name.to_string()));
        obj.do_not_sync = true;
        if tree {
            let index = square
                .borrow_mut()
                .place_wall_and_door_check(sprites, &obj, -1);
            if index >= 0 && index as usize <= square.borrow().objects.len() {
                square
                    .borrow_mut()
                    .objects
                    .insert(index as usize, ObjectSlot::new(obj));
            } else {
                square.borrow_mut().objects.push(ObjectSlot::new(obj));
            }
        } else {
            square.borrow_mut().objects.push(ObjectSlot::new(obj));
        }
        id
    }
}
