//! [`ErosionHost`] over `pz-world`: a chunk's squares and objects and the
//! sprite manager.
//!
//! Square flags are computed from the objects the way
//! `IsoGridSquare.RecalcProperties` derives the ones erosion reads
//! (`exterior`, `cutN/W`, `canPathN/W`). Edge queries that need the
//! collision model (`isWallTo`, `isWindowTo`, `isWindowBlockedTo`,
//! `getWindowTo`) and squares outside the chunk come from [`EdgeQueries`].

use pz_world::objects::{IsoObject, IsoSpriteInstance, IsoTree, ObjectKind, ObjectName};
use pz_world::{IsoChunk, IsoGridSquare, IsoSpriteManager};

use crate::host::{Attached, ErosionHost, ObjId, Sq, SqFlag};
use crate::ice_queen::get_sprite;

/// `IsoGridSquare` edge queries the port of the world does not model yet.
/// The defaults answer as if no square had walls or windows.
pub trait EdgeQueries {
    /// `square.isWallTo(other)`.
    fn is_wall_to(
        &self,
        _chunk: &IsoChunk,
        _sprites: &IsoSpriteManager,
        _sq: Sq,
        _other: Sq,
    ) -> bool {
        false
    }
    /// `square.isWindowTo(other)`.
    fn is_window_to(
        &self,
        _chunk: &IsoChunk,
        _sprites: &IsoSpriteManager,
        _sq: Sq,
        _other: Sq,
    ) -> bool {
        false
    }
    /// `square.isWindowBlockedTo(other)`.
    fn is_window_blocked_to(
        &self,
        _chunk: &IsoChunk,
        _sprites: &IsoSpriteManager,
        _sq: Sq,
        _other: Sq,
    ) -> bool {
        false
    }
    /// `square.getWindowTo(other)` (an object of `sq`).
    fn window_to(
        &self,
        _chunk: &IsoChunk,
        _sprites: &IsoSpriteManager,
        _sq: Sq,
        _other: Sq,
    ) -> Option<ObjId> {
        None
    }
}

/// [`EdgeQueries`] with every default.
pub struct NoEdges;
impl EdgeQueries for NoEdges {}

/// A chunk being processed by the erosion system.
pub struct ChunkHost<'a> {
    pub chunk: &'a mut IsoChunk,
    pub sprites: &'a mut IsoSpriteManager,
    pub edges: &'a dyn EdgeQueries,
    /// `IsoTree.LOGS_PER_SIZE` (`pz_world::objects::ObjectTables`), for
    /// trees erosion creates.
    pub logs_per_size: Option<&'a [i32]>,
}

/// The square-level flags of `RecalcProperties` that erosion reads.
pub fn square_has_flag(sq: &IsoGridSquare, m: &IsoSpriteManager, flag: SqFlag) -> bool {
    let f = m.tables.flag;
    let blueprint = m.tables.flag_from_string("blueprint");
    let props = || {
        sq.objects
            .iter()
            .filter_map(|o| o.sprite.map(|s| &m.sprites[s].properties))
            .filter(move |p| !blueprint.is_some_and(|b| p.has(b)))
    };
    let any = |fl| props().any(|p| p.has(fl));
    match flag {
        SqFlag::Exterior => sq.room_id == -1 && !sq.have_roof,
        SqFlag::CutN => any(f.cutN),
        SqFlag::CutW => any(f.cutW),
        SqFlag::CanPathN | SqFlag::CanPathW => {
            let (can, hop, collide) = if flag == SqFlag::CanPathN {
                (f.canPathN, f.HoppableN, f.collideN)
            } else {
                (f.canPathW, f.HoppableW, f.collideW)
            };
            let non_hoppable_collide = props().any(|p| p.has(collide) && !p.has(hop));
            any(can) && !(non_hoppable_collide && any(hop))
        }
    }
}

impl ChunkHost<'_> {
    fn local(&self, sq: Sq) -> (i32, i32, i32) {
        (sq.0 - self.chunk.wx * 8, sq.1 - self.chunk.wy * 8, sq.2)
    }

    pub fn square(&self, sq: Sq) -> Option<&IsoGridSquare> {
        let (x, y, z) = self.local(sq);
        self.chunk.get_grid_square(x, y, z)
    }

    pub fn square_mut(&mut self, sq: Sq) -> Option<&mut IsoGridSquare> {
        let (x, y, z) = self.local(sq);
        self.chunk.get_grid_square_mut(x, y, z)
    }

    fn obj(&self, sq: Sq, o: ObjId) -> Option<&IsoObject> {
        let s = self.square(sq)?;
        s.objects.iter().find(|x| x.id == o)
    }

    fn obj_mut(&mut self, sq: Sq, o: ObjId) -> Option<&mut IsoObject> {
        self.square_mut(sq)?.objects.iter_mut().find(|x| x.id == o)
    }

    fn remove(&mut self, sq: Sq, o: ObjId, special_too: bool) {
        if let Some(s) = self.square_mut(sq) {
            s.objects.retain(|x| x.id != o);
            if special_too {
                s.special_objects.retain(|&x| x != o);
            }
        }
    }
}

impl ErosionHost for ChunkHost<'_> {
    fn square_exists(&self, sq: Sq) -> bool {
        self.square(sq).is_some()
    }

    fn has_flag(&self, sq: Sq, flag: SqFlag) -> bool {
        self.square(sq)
            .is_some_and(|s| square_has_flag(s, self.sprites, flag))
    }

    fn has_wall_type(&self, sq: Sq) -> bool {
        let wall = self.sprites.tables.ty.wall;
        self.square(sq).is_some_and(|s| {
            s.objects.iter().any(|o| {
                o.sprite
                    .is_some_and(|sp| self.sprites.sprites[sp].tile_type == wall)
            })
        })
    }

    fn objects(&self, sq: Sq) -> Vec<ObjId> {
        self.square(sq)
            .map_or_else(Vec::new, |s| s.objects.iter().map(|o| o.id).collect())
    }

    fn floor(&self, sq: Sq) -> Option<ObjId> {
        let s = self.square(sq)?;
        s.floor_index(self.sprites).map(|i| s.objects[i].id)
    }

    fn wall(&self, sq: Sq, north: bool) -> Option<ObjId> {
        let s = self.square(sq)?;
        s.wall_index(self.sprites, north).map(|i| s.objects[i].id)
    }

    fn is_wall_to(&self, sq: Sq, other: Sq) -> bool {
        self.edges.is_wall_to(self.chunk, self.sprites, sq, other)
    }

    fn is_window_to(&self, sq: Sq, other: Sq) -> bool {
        self.edges.is_window_to(self.chunk, self.sprites, sq, other)
    }

    fn is_window_blocked_to(&self, sq: Sq, other: Sq) -> bool {
        self.edges
            .is_window_blocked_to(self.chunk, self.sprites, sq, other)
    }

    fn window_to(&self, sq: Sq, other: Sq) -> Option<ObjId> {
        self.edges.window_to(self.chunk, self.sprites, sq, other)
    }

    fn sprite_name(&self, sq: Sq, o: ObjId) -> Option<String> {
        let s = self.obj(sq, o)?.sprite?;
        self.sprites.sprites[s].name.clone()
    }

    fn sprite_tile_sheet_index(&self, sq: Sq, o: ObjId) -> i32 {
        self.obj(sq, o)
            .and_then(|x| x.sprite)
            .map_or(0, |s| self.sprites.sprites[s].tile_sheet_index)
    }

    fn sprite_custom_name(&self, sq: Sq, o: ObjId) -> Option<String> {
        let s = self.obj(sq, o)?.sprite?;
        self.sprites
            .prop(&self.sprites.sprites[s], "CustomName")
            .map(str::to_string)
    }

    fn named_sprite_custom_name(&self, name: &str) -> Option<Option<String>> {
        let spr = self.sprites.by_name(name)?;
        Some(self.sprites.prop(spr, "CustomName").map(str::to_string))
    }

    fn object_name(&self, sq: Sq, o: ObjId) -> Option<String> {
        match self.obj(sq, o)?.name.as_ref()? {
            ObjectName::Str(s) => Some(s.clone()),
            ObjectName::Grass => Some("Grass".to_string()),
            // A `WorldDictionary` id: not resolved here.
            ObjectName::Dict(_) => None,
        }
    }

    fn set_object_name(&mut self, sq: Sq, o: ObjId, name: Option<&str>) {
        if let Some(obj) = self.obj_mut(sq, o) {
            obj.name = name.map(|n| ObjectName::Str(n.to_string()));
            obj.do_not_sync = true;
        }
    }

    fn set_object_sprite(&mut self, sq: Sq, o: ObjId, sprite: &str) {
        let s = get_sprite(self.sprites, sprite);
        if let Some(obj) = self.obj_mut(sq, o) {
            obj.sprite = Some(s);
            obj.do_not_sync = true;
        }
    }

    fn attached(&self, sq: Sq, o: ObjId) -> Option<Vec<Attached>> {
        let list = self.obj(sq, o)?.attached_anim_sprite.as_ref()?;
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
        if let Some(list) = self
            .obj_mut(sq, o)
            .and_then(|x| x.attached_anim_sprite.as_mut())
        {
            if index < list.len() {
                list.remove(index);
            }
        }
    }

    fn attached_clear(&mut self, sq: Sq, o: ObjId) {
        if let Some(obj) = self.obj_mut(sq, o) {
            obj.attached_anim_sprite
                .get_or_insert_with(Vec::new)
                .clear();
        }
    }

    fn attached_add(&mut self, sq: Sq, o: ObjId, sprite: &str) -> i32 {
        let s = get_sprite(self.sprites, sprite);
        let id = self.sprites.sprites[s].id;
        if let Some(obj) = self.obj_mut(sq, o) {
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
        let id = square.new_object_id();
        let mut obj = IsoObject::new(id, kind, named);
        if !tree {
            obj.tile = Some(sprite.to_string());
        }
        obj.name = Some(ObjectName::Str(name.to_string()));
        obj.do_not_sync = true;
        if tree {
            let index = square.place_wall_and_door_check(sprites, &obj, -1);
            if index >= 0 && index as usize <= square.objects.len() {
                square.objects.insert(index as usize, obj);
            } else {
                square.objects.push(obj);
            }
        } else {
            square.objects.push(obj);
        }
        id
    }
}
