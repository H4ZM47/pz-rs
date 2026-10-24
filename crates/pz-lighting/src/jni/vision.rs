//! The Java vision tests `updateChunk` uses to build the square flags:
//! `IsoGridSquare.testVisionAdjacent`, `DoDiagnalCheck`, `getOpenAir` and
//! the `TestVision` overrides of the special objects.

use super::input::{GridSquare, IsoDirection, LightingWorld, SpecialObject};
use crate::TestResult;

/// `IsoObject.VisionResult`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Vis {
    NoEffect,
    Blocked,
    Unblocked,
}

fn same(a: &GridSquare, b: &GridSquare) -> bool {
    a.x == b.x && a.y == b.y && a.z == b.z
}

/// `TestVision(from, to)` of an object that lives on `owner`.
fn test_vision(obj: &SpecialObject, owner: &GridSquare, from: &GridSquare, to: &GridSquare) -> Vis {
    use Vis::*;
    let from_owner = same(from, owner);
    // The shared door/window shape: blocked or not when looking out across
    // the object's edge.
    let edge = |north: bool| -> bool {
        if from_owner {
            (north && to.y < from.y) || (!north && to.x < from.x)
        } else {
            (north && to.y > from.y) || (!north && to.x > from.x)
        }
    };
    match obj {
        SpecialObject::Curtain(c) => {
            if to.z != from.z {
                return NoEffect;
            }
            use super::input::CurtainType::*;
            let near = (from_owner && matches!(c.kind, W | N)) || (!from_owner && matches!(c.kind, E | S));
            let hit = if near {
                (c.north && to.y < from.y) || (!c.north && to.x < from.x)
            } else {
                (c.north && to.y > from.y) || (!c.north && to.x > from.x)
            };
            if hit && !c.open {
                Blocked
            } else {
                NoEffect
            }
        }
        SpecialObject::Door(d) => {
            let mut trans = d.door_trans;
            if d.garage_door && d.open {
                trans = true;
            }
            if d.open {
                trans = true;
            } else if d.has_curtain && !d.curtain_open {
                trans = false;
            }
            let north = d.north != d.open;
            if to.z != from.z {
                return NoEffect;
            }
            if edge(north) {
                if trans {
                    Unblocked
                } else {
                    Blocked
                }
            } else {
                NoEffect
            }
        }
        SpecialObject::Thumpable(t) => {
            if t.can_pass_through {
                return NoEffect;
            }
            let north = t.north != t.open;
            if to.z != from.z {
                return NoEffect;
            }
            if edge(north) {
                if t.door_trans || t.is_window {
                    Unblocked
                } else {
                    Blocked
                }
            } else {
                NoEffect
            }
        }
        SpecialObject::Window(w) => {
            if to.z != from.z {
                return NoEffect;
            }
            if edge(w.north) {
                Unblocked
            } else {
                NoEffect
            }
        }
        SpecialObject::Barricade(b) => {
            if b.metal_health <= 0 && b.planks <= 2 {
                return NoEffect;
            }
            let (f, t) = if from_owner {
                (from, to)
            } else if same(to, owner) {
                (to, from)
            } else {
                return NoEffect;
            };
            let hit = match b.dir {
                Some(IsoDirection::N) => t.y < f.y,
                Some(IsoDirection::S) => t.y > f.y,
                Some(IsoDirection::W) => t.x < f.x,
                Some(IsoDirection::E) => t.x > f.x,
                _ => false,
            };
            if hit {
                Blocked
            } else {
                NoEffect
            }
        }
    }
}

/// `IsoGridSquare.getOpenAir()`.
pub fn open_air(world: &impl LightingWorld, sq: &GridSquare) -> bool {
    let mut s = sq;
    loop {
        if !s.exterior {
            return false;
        }
        match world.square(s.x, s.y, s.z + 1) {
            Some(u) => s = u,
            None => return s.z >= 0,
        }
    }
}

fn clear_like(t: TestResult) -> bool {
    matches!(
        t,
        TestResult::Clear | TestResult::ClearThroughWindow | TestResult::ClearThroughOpenDoor | TestResult::ClearThroughClosedDoor
    )
}

/// `IsoGridSquare.testVisionAdjacent(x, y, z, specialDiag, bIgnoreDoors)`.
pub fn test_vision_adjacent(
    world: &impl LightingWorld,
    sq: &GridSquare,
    x: i32,
    y: i32,
    z: i32,
    special_diag: bool,
    ignore_doors: bool,
) -> TestResult {
    if !(-1..=1).contains(&x) || !(-1..=1).contains(&y) || !(-1..=1).contains(&z) {
        return TestResult::Blocked;
    }
    if z == 1 && (x != 0 || y != 0) && sq.elevated_floor {
        if let Some(up) = world.square(sq.x, sq.y, sq.z + z) {
            return test_vision_adjacent(world, up, x, y, 0, special_diag, ignore_doors);
        }
    }
    if z == -1 && (x != 0 || y != 0) {
        if let Some(d) = world.square(sq.x + x, sq.y + y, sq.z + z) {
            if d.elevated_floor {
                return test_vision_adjacent(world, sq, x, y, 0, special_diag, ignore_doors);
            }
        }
    }
    if x != 0 && y != 0 && special_diag {
        let mut test = diagonal_check(world, sq, x, y, z, ignore_doors);
        if clear_like(test) {
            if let Some(sq2) = world.square(sq.x + x, sq.y + y, sq.z + z) {
                test = diagonal_check(world, sq2, -x, -y, -z, ignore_doors);
            }
        }
        return test;
    }
    let other = world.square(sq.x + x, sq.y + y, sq.z + z);
    let mut ret = TestResult::Clear;
    match other {
        Some(o) if o.z == sq.z => {
            for (owner, list) in [(sq, &sq.special_objects), (o, &o.special_objects)] {
                for obj in list {
                    let v = test_vision(obj, owner, sq, o);
                    match (v, obj) {
                        (Vis::NoEffect, _) => {}
                        (Vis::Unblocked, SpecialObject::Door(d)) => {
                            ret = if d.open { TestResult::ClearThroughOpenDoor } else { TestResult::ClearThroughClosedDoor };
                        }
                        (Vis::Unblocked, SpecialObject::Thumpable(t)) if t.is_door => ret = TestResult::ClearThroughOpenDoor,
                        (Vis::Unblocked, SpecialObject::Window(_)) => ret = TestResult::ClearThroughWindow,
                        (Vis::Blocked, SpecialObject::Door(_)) if !ignore_doors => return TestResult::Blocked,
                        (Vis::Blocked, SpecialObject::Thumpable(t)) if t.is_door && !ignore_doors => return TestResult::Blocked,
                        (Vis::Blocked, SpecialObject::Thumpable(t)) if t.is_window => return TestResult::Blocked,
                        (Vis::Blocked, SpecialObject::Curtain(_) | SpecialObject::Window(_) | SpecialObject::Barricade(_)) => {
                            return TestResult::Blocked
                        }
                        _ => {}
                    }
                }
            }
        }
        Some(o) if z > 0 && (sq.z != -1 || o.z != 0) && o.exterior && !sq.exterior => ret = TestResult::Blocked,
        _ => {}
    }
    let bit = (x + 1) + (y + 1) * 3 + (z + 1) * 9;
    if (sq.vision_matrix >> bit) & 1 != 0 {
        TestResult::Blocked
    } else {
        ret
    }
}

/// `IsoGridSquare.DoDiagnalCheck`.
fn diagonal_check(world: &impl LightingWorld, sq: &GridSquare, x: i32, y: i32, z: i32, ignore_doors: bool) -> TestResult {
    let r1 = test_vision_adjacent(world, sq, x, 0, z, false, ignore_doors);
    if r1 == TestResult::Blocked {
        return TestResult::Blocked;
    }
    let r2 = test_vision_adjacent(world, sq, 0, y, z, false, ignore_doors);
    if r2 == TestResult::Blocked {
        return TestResult::Blocked;
    }
    if r1 == TestResult::ClearThroughWindow || r2 == TestResult::ClearThroughWindow {
        return TestResult::ClearThroughWindow;
    }
    test_vision_adjacent(world, sq, x, y, z, false, ignore_doors)
}
