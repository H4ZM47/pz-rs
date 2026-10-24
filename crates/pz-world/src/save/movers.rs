//! Moving objects and animal tracks a square's object list can hold:
//! `IsoPushableObject` and `IsoWheelieBin` (saved through
//! `IsoMovingObject.save`, not `IsoObject.save`) and `IsoAnimalTrack` with
//! its `AnimalTracks`.

use super::io::{err, Reader, SaveResult, Writer};
use super::items::ItemRegistry;
use super::visual::MovingBase;
use crate::objects::ItemContainer;

/// `AnimalTracks` as `IsoAnimalTrack` saves it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AnimalTracks {
    pub animal_type: String,
    pub track_type: String,
    pub x: i32,
    pub y: i32,
    /// `dir.ordinal()`; `None` when the track has no direction.
    pub dir: Option<i32>,
    /// Calendar milliseconds when the track was made.
    pub added_time: i64,
    pub added_to_world: bool,
}

impl AnimalTracks {
    /// `AnimalTracks.load`.
    pub fn load(r: &mut Reader<'_>) -> SaveResult<Self> {
        let animal_type = r.string()?;
        let track_type = r.string()?;
        let x = r.i32()?;
        let y = r.i32()?;
        // IsoDirections.fromIndex.
        let dir = if r.bool()? { Some(r.i32()? & 7) } else { None };
        Ok(AnimalTracks {
            animal_type,
            track_type,
            x,
            y,
            dir,
            added_time: r.i64()?,
            added_to_world: r.bool()?,
        })
    }

    /// `AnimalTracks.save`.
    pub fn save(&self, w: &mut Writer) {
        w.string(&self.animal_type);
        w.string(&self.track_type);
        w.i32(self.x);
        w.i32(self.y);
        match self.dir {
            Some(d) => {
                w.i8(1);
                w.i32(d);
            }
            None => w.i8(0),
        }
        w.i64(self.added_time);
        w.bool(self.added_to_world);
    }
}

/// `IsoPushableObject` (`"Pushable"`) or its subclass `IsoWheelieBin`
/// (`"WheelieBin"`). The sprite a pushable saves is the object's `sprite`
/// and the container its `container`, as in the original.
#[derive(Clone, Debug, PartialEq)]
pub struct IsoPushableObject {
    pub wheelie_bin: bool,
    pub base: MovingBase,
}

impl IsoPushableObject {
    /// The class's factory name.
    pub fn object_name(&self) -> &'static str {
        if self.wheelie_bin {
            "WheelieBin"
        } else {
            "Pushable"
        }
    }

    /// The object's `offsetX`/`offsetY` after the `(IsoCell)` constructor:
    /// `IsoObject`'s field initialisers, or the wheelie bin's own.
    pub fn constructor_offsets(wheelie_bin: bool, tile_scale: i32) -> (f32, f32) {
        if wheelie_bin {
            (-26.0, -248.0)
        } else {
            let ts = tile_scale as f32;
            (32.0 * ts, 96.0 * ts)
        }
    }

    /// The container the `(IsoCell)` constructor gives the class (a wheelie
    /// bin has one before loading; a plain pushable has none).
    pub fn constructor_container(wheelie_bin: bool) -> Option<ItemContainer> {
        wheelie_bin.then(|| {
            let mut c = ItemContainer::new(Some("wheeliebin"));
            c.id = -1;
            c
        })
    }

    /// `IsoPushableObject.load` (after the factory bytes). Returns the
    /// object, the saved sprite id (`None` for a wheelie bin, which keeps
    /// its constructor sprite) and the container: a loaded one replaces the
    /// constructor's, which is otherwise kept.
    pub fn load(
        r: &mut Reader<'_>,
        wv: i32,
        wheelie_bin: bool,
        tile_scale: i32,
        items: &dyn ItemRegistry,
    ) -> SaveResult<(Self, Option<i32>, Option<ItemContainer>)> {
        let base = MovingBase::load(r, wv, Self::constructor_offsets(wheelie_bin, tile_scale))?;
        let sprite = if wheelie_bin { None } else { Some(r.i32()?) };
        let mut container = Self::constructor_container(wheelie_bin);
        if r.bool()? {
            let mut c = ItemContainer::new(None);
            c.id = -1;
            c.load(r, wv, items)?;
            container = Some(c);
        }
        Ok((IsoPushableObject { wheelie_bin, base }, sprite, container))
    }

    /// `IsoPushableObject.save`, including the serialise and class bytes.
    /// `sprite_id` is the object's `sprite.id` (`None` when it has no
    /// sprite: the original throws).
    pub fn save(
        &self,
        w: &mut Writer,
        class_id: i8,
        sprite_id: Option<i32>,
        container: Option<&ItemContainer>,
        items: &dyn ItemRegistry,
    ) -> SaveResult<()> {
        self.base.save(w, class_id);
        if !self.wheelie_bin {
            match sprite_id {
                Some(id) => w.i32(id),
                None => return err("NullPointerException: pushable sprite is null"),
            }
        }
        match container {
            Some(c) => {
                w.i8(1);
                c.save(w, items)?;
            }
            None => w.i8(0),
        }
        Ok(())
    }
}
