//! Resumable main-object activation before each square's erosion/observers.
//! The engine owns the cursor between calls, so no world, sprite, square or
//! object borrow needs to cross a host callback. Legacy loaders opt out.
use std::rc::Rc;

use super::{ChunkLoadHook, HookCtx};
use crate::chunk::{IsoCell, IsoChunk};
use crate::sprite::IsoSpriteManager;
use crate::{IsoObjectRef, SquareRef};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChunkActivationError {
    MissingChunk { wx: i32, wy: i32 },
    DisplacedSquare { x: i32, y: i32, z: i32 },
    BorrowedSquare { x: i32, y: i32, z: i32 },
    IncompleteCursor,
    AlreadyPublished,
}
impl std::fmt::Display for ChunkActivationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "chunk activation: {self:?}")
    }
}
impl std::error::Error for ChunkActivationError {}

/// All returned handles retain the actual selected allocations. The caller
/// releases enclosing world/sprite guards before invoking any host callback.
pub enum ChunkActivationStep {
    Object {
        square: SquareRef,
        object: IsoObjectRef,
        x: i32,
        y: i32,
        z: i32,
        index: usize,
    },
    /// Every reached main object was activated. Run optional erosion, then
    /// MapObjects/Lua observers for this same receiver before requesting more.
    SquareReady {
        square: SquareRef,
        x: i32,
        y: i32,
        z: i32,
    },
    Complete,
}

/// The native level/x/y/index order, with a strong selected-square pin. It
/// reads the main list anew on every step rather than snapshotting objects.
/// IsoChunk is still value-owned: arbitrary replacement of the entire chunk
/// is not made into a canonical retained chunk by this bounded coordinator.
pub struct ChunkActivationCursor {
    pub wx: i32,
    pub wy: i32,
    x: i32,
    y: i32,
    z: i32,
    current: Option<SquareRef>,
    index: usize,
    complete: bool,
    published: bool,
}
impl ChunkActivationCursor {
    pub fn new(chunk: &IsoChunk) -> Self {
        Self {
            wx: chunk.wx,
            wy: chunk.wy,
            x: 0,
            y: 0,
            z: chunk.min_level,
            current: None,
            index: 0,
            complete: false,
            published: false,
        }
    }
    pub fn is_complete(&self) -> bool {
        self.complete
    }

    fn advance_square(&mut self) {
        self.index = 0;
        self.y += 1;
        if self.y == 8 {
            self.y = 0;
            self.x += 1;
        }
        if self.x == 8 {
            self.x = 0;
            self.z += 1;
        }
    }

    /// Explicit host recovery, not the native addToWorld exception policy.
    /// Native chunk activation/erosion exceptions propagate; only the later
    /// LoadGridsquare Lua event has a local catch. The pin is returned for
    /// destruction only after the caller's guards end.
    pub fn skip_square(&mut self) -> Option<SquareRef> {
        let pin = self.current.take();
        if pin.is_some() {
            self.advance_square();
        }
        pin
    }

    pub fn next_step(
        &mut self,
        cell: &IsoCell,
    ) -> Result<ChunkActivationStep, ChunkActivationError> {
        if self.complete {
            return Ok(ChunkActivationStep::Complete);
        }
        loop {
            let chunk =
                cell.chunks
                    .get(&(self.wx, self.wy))
                    .ok_or(ChunkActivationError::MissingChunk {
                        wx: self.wx,
                        wy: self.wy,
                    })?;
            if let Some(square) = &self.current {
                check_square(cell, self.wx, self.wy, self.x, self.y, self.z, square)?;
                let object = {
                    let square =
                        square
                            .try_borrow()
                            .map_err(|_| ChunkActivationError::BorrowedSquare {
                                x: self.x,
                                y: self.y,
                                z: self.z,
                            })?;
                    square.objects.get(self.index).map(|slot| slot.ref_handle())
                };
                if let Some(object) = object {
                    let index = self.index;
                    self.index += 1;
                    return Ok(ChunkActivationStep::Object {
                        square: square.clone(),
                        object,
                        x: self.x,
                        y: self.y,
                        z: self.z,
                        index,
                    });
                }
                let square = self.current.take().expect("selected square retained");
                let (x, y, z) = (self.x, self.y, self.z);
                self.advance_square();
                return Ok(ChunkActivationStep::SquareReady { square, x, y, z });
            }
            if self.z > chunk.max_level {
                self.complete = true;
                return Ok(ChunkActivationStep::Complete);
            }
            if let Some(square) = chunk.get_grid_square(self.x, self.y, self.z) {
                let has_objects = !square
                    .try_borrow()
                    .map_err(|_| ChunkActivationError::BorrowedSquare {
                        x: self.x,
                        y: self.y,
                        z: self.z,
                    })?
                    .objects
                    .is_empty();
                if has_objects {
                    self.current = Some(square);
                    continue;
                }
            }
            self.advance_square();
        }
    }

    pub(crate) fn check_unpublished_complete(&self) -> Result<(), ChunkActivationError> {
        if self.published {
            Err(ChunkActivationError::AlreadyPublished)
        } else if !self.complete {
            Err(ChunkActivationError::IncompleteCursor)
        } else {
            Ok(())
        }
    }
    pub(crate) fn mark_published(&mut self) {
        self.published = true;
    }
}

fn check_square(
    cell: &IsoCell,
    wx: i32,
    wy: i32,
    x: i32,
    y: i32,
    z: i32,
    square: &SquareRef,
) -> Result<(), ChunkActivationError> {
    let chunk = cell
        .chunks
        .get(&(wx, wy))
        .ok_or(ChunkActivationError::MissingChunk { wx, wy })?;
    if chunk
        .get_grid_square(x, y, z)
        .is_some_and(|actual| Rc::ptr_eq(&actual, square))
    {
        Ok(())
    } else {
        Err(ChunkActivationError::DisplacedSquare { x, y, z })
    }
}

/// Run the legacy pure-Rust erosion hook for the actual selected receiver.
/// This helper must not run Lua. The chunk is temporarily absent only during
/// this borrow-constrained erosion call; the engine runs observers afterwards.
pub fn finish_square_erosion(
    cell: &mut IsoCell,
    wx: i32,
    wy: i32,
    x: i32,
    y: i32,
    z: i32,
    square: &SquareRef,
    sprites: &mut IsoSpriteManager,
    hook: Option<&mut dyn ChunkLoadHook>,
) -> Result<(), ChunkActivationError> {
    check_square(cell, wx, wy, x, y, z, square)?;
    if let Some(hook) = hook {
        with_erosion_chunk(cell, wx, wy, sprites, |chunk, ctx| {
            hook.load_grid_square(chunk, x, y, z, ctx)
        })?;
    }
    Ok(())
}

/// `ErosionMain.ChunkLoaded` once all selected square observers are complete.
pub fn finish_chunk_erosion(
    cell: &mut IsoCell,
    wx: i32,
    wy: i32,
    sprites: &mut IsoSpriteManager,
    hook: Option<&mut dyn ChunkLoadHook>,
) -> Result<(), ChunkActivationError> {
    if let Some(hook) = hook {
        with_erosion_chunk(cell, wx, wy, sprites, |chunk, ctx| {
            hook.chunk_loaded(chunk, ctx)
        })?;
    } else if !cell.chunks.contains_key(&(wx, wy)) {
        return Err(ChunkActivationError::MissingChunk { wx, wy });
    }
    Ok(())
}

fn with_erosion_chunk(
    cell: &mut IsoCell,
    wx: i32,
    wy: i32,
    sprites: &mut IsoSpriteManager,
    callback: impl FnOnce(&mut IsoChunk, &mut HookCtx<'_>),
) -> Result<(), ChunkActivationError> {
    // Reinsert on unwind too. Hooks receive neighbours only, so cannot replace
    // this slot through the mutable cell while it is temporarily detached.
    struct Restore<'a> {
        cell: &'a mut IsoCell,
        chunk: Option<IsoChunk>,
        coords: (i32, i32),
    }
    impl Drop for Restore<'_> {
        fn drop(&mut self) {
            let chunk = self.chunk.take().expect("erosion chunk retained");
            let replaced = self.cell.chunks.insert(self.coords, chunk);
            assert!(
                replaced.is_none(),
                "erosion hook replaced its detached chunk"
            );
        }
    }
    let chunk = cell
        .chunks
        .remove(&(wx, wy))
        .ok_or(ChunkActivationError::MissingChunk { wx, wy })?;
    let mut restore = Restore {
        cell,
        chunk: Some(chunk),
        coords: (wx, wy),
    };
    let Restore { cell, chunk, .. } = &mut restore;
    let mut ctx = HookCtx {
        sprites,
        neighbours: &**cell,
    };
    callback(chunk.as_mut().expect("erosion chunk retained"), &mut ctx);
    Ok(())
}

#[cfg(test)]
mod tests;
