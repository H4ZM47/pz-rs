//! What the erosion code needs from the world: square queries and object
//! edits, phrased as the original `IsoGridSquare` / `IsoObject` /
//! `IsoSpriteManager` calls. [`crate::world_host::ChunkHost`] implements it
//! over `pz-world`'s chunk and sprite manager.

/// A square by world coordinates.
pub type Sq = (i32, i32, i32);

/// An object of a square (`pz_world::objects::ObjectId`).
pub type ObjId = u32;

/// Square property flags erosion reads (`square.has(IsoFlagType.X)`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SqFlag {
    Exterior,
    CutN,
    CutW,
    CanPathN,
    CanPathW,
}

/// One `IsoSpriteInstance` of `attachedAnimSprite`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attached {
    /// `parentSprite.getName()`.
    pub name: Option<String>,
    /// `getID()` (`parentSprite.id`).
    pub id: i32,
}

pub trait ErosionHost {
    /// `IsoCell.getGridSquare(x, y, z) != null`.
    fn square_exists(&self, sq: Sq) -> bool;
    /// `square.has(flag)`.
    fn has_flag(&self, sq: Sq, flag: SqFlag) -> bool;
    /// `square.has(IsoObjectType.wall)`.
    fn has_wall_type(&self, sq: Sq) -> bool;
    /// `square.getObjects()`, in order.
    fn objects(&self, sq: Sq) -> Vec<ObjId>;
    /// `square.getFloor()`.
    fn floor(&self, sq: Sq) -> Option<ObjId>;
    /// `square.getWall(north)`.
    fn wall(&self, sq: Sq, north: bool) -> Option<ObjId>;
    /// `square.isWallTo(other)` (`other` may not exist).
    fn is_wall_to(&self, sq: Sq, other: Sq) -> bool;
    /// `square.isWindowTo(other)`.
    fn is_window_to(&self, sq: Sq, other: Sq) -> bool;
    /// `square.isWindowBlockedTo(other)`.
    fn is_window_blocked_to(&self, sq: Sq, other: Sq) -> bool;
    /// `square.getWindowTo(other)`.
    fn window_to(&self, sq: Sq, other: Sq) -> Option<ObjId>;

    /// `obj.getSprite().getName()` (`None` without a sprite or name).
    fn sprite_name(&self, sq: Sq, o: ObjId) -> Option<String>;
    /// `obj.getSprite().tileSheetIndex`.
    fn sprite_tile_sheet_index(&self, sq: Sq, o: ObjId) -> i32;
    /// `obj.getSprite().getProperties().get(CustomName)`.
    fn sprite_custom_name(&self, sq: Sq, o: ObjId) -> Option<String>;
    /// `IsoSpriteManager.namedMap.get(name)`'s `CustomName`: `None` when the
    /// sprite does not exist, `Some(None)` when it has no custom name.
    fn named_sprite_custom_name(&self, name: &str) -> Option<Option<String>>;

    /// `obj.getName()`.
    fn object_name(&self, sq: Sq, o: ObjId) -> Option<String>;
    /// `obj.setName(name)`.
    fn set_object_name(&mut self, sq: Sq, o: ObjId, name: Option<&str>);
    /// `obj.setSprite(IsoSpriteManager.getSprite(name))` (creating the
    /// sprite if the manager does not have it).
    fn set_object_sprite(&mut self, sq: Sq, o: ObjId, sprite: &str);
    /// `obj.attachedAnimSprite` (`None` when the list is null).
    fn attached(&self, sq: Sq, o: ObjId) -> Option<Vec<Attached>>;
    /// `attachedAnimSprite.remove(index)`.
    fn attached_remove(&mut self, sq: Sq, o: ObjId, index: usize);
    /// Creates the list if null, then clears it.
    fn attached_clear(&mut self, sq: Sq, o: ObjId);
    /// `attachedAnimSprite.add(getSprite(name).newInstance())`, creating the
    /// list if null; returns the instance's id.
    fn attached_add(&mut self, sq: Sq, o: ObjId, sprite: &str) -> i32;

    /// `square.RemoveTileObject(obj, false)`.
    fn remove_tile_object(&mut self, sq: Sq, o: ObjId);
    /// `square.RemoveTileObjectErosionNoRecalc(obj)`.
    fn remove_tile_object_erosion_no_recalc(&mut self, sq: Sq, o: ObjId);
    /// `square.getObjects().remove(obj)`.
    fn remove_object_raw(&mut self, sq: Sq, o: ObjId);
    /// `ErosionObj.createObject` + adding it: a tree
    /// (`IsoTree.getNew` + `initTree`, `square.AddTileObject`) or a plain
    /// `IsoObject.getNew(square, sprite, name, false)` appended to the
    /// objects. The object is named `name`.
    fn add_object(&mut self, sq: Sq, sprite: &str, name: &str, tree: bool) -> ObjId;
}
