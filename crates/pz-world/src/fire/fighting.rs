//! `zombie.fireFighting.FireFighting`: which square a player puts out
//! (`isSquareToExtinguish`, `getSquareToExtinguish`) and which item they
//! put it out with (`getExtinguisher`, `isExtinguisher`,
//! `getExtinguisherUses`, `getWaterUsesInteger`). The item side reads
//! items through [`ExtinguisherItem`], which the inventory code
//! implements.

use crate::chunk::IsoCell;
use crate::objects::ObjectKind;
use crate::sprite::IsoFlagType;

/// The item fields `FireFighting` reads.
pub trait ExtinguisherItem {
    /// `getType()`.
    fn item_type(&self) -> &str;
    /// `getCurrentUses()`.
    fn current_uses(&self) -> i32;
    /// `isWaterSource()`.
    fn is_water_source(&self) -> bool;
    /// `IsDrainable()`.
    fn is_drainable(&self) -> bool;
    /// With a `FluidContainer` component: `None` for no primary fluid,
    /// else whether it is `Water` / `TaintedWater` and `getAmount()` (in
    /// litres). `None` outside means no component.
    fn fluid(&self) -> Option<Option<(bool, f32)>>;
}

fn is_bag(t: &str) -> bool {
    matches!(t, "Sandbag" | "Gravelbag" | "Dirtbag")
}

/// `getWaterUsesInteger(item)`: 100 ml of water per use.
pub fn water_uses_integer(item: Option<&dyn ExtinguisherItem>) -> i32 {
    let Some(item) = item else { return 0 };
    if let Some(fluid) = item.fluid() {
        let Some((water, litres)) = fluid else {
            return 0;
        };
        if water {
            let millilitres = litres * 1000.0;
            return (millilitres / 100.0).floor() as i32;
        }
    }
    if item.is_drainable() && item.is_water_source() {
        return item.current_uses();
    }
    0
}

/// `getExtinguisherUses(item)`: what one use takes.
pub fn extinguisher_uses(item: Option<&dyn ExtinguisherItem>) -> i32 {
    let Some(item) = item else { return 10000 };
    let t = item.item_type();
    if t == "Extinguisher" || is_bag(t) {
        1
    } else if item.is_water_source() {
        10
    } else {
        10000
    }
}

/// `isExtinguisher(item)`: it can put out one fire now.
pub fn is_extinguisher(item: Option<&dyn ExtinguisherItem>) -> bool {
    let Some(it) = item else { return false };
    let t = it.item_type();
    if t == "Extinguisher" || is_bag(t) {
        return it.current_uses() >= extinguisher_uses(item);
    }
    if it.is_water_source() {
        return water_uses_integer(item) >= extinguisher_uses(item);
    }
    false
}

/// `getExtinguisher(player)`: the primary hand item, the secondary, then
/// from the inventory (in order) the first extinguisher, else the first
/// sand, gravel or dirt bag, else the first water source. Returns the
/// index into `[primary, secondary, inventory...]`.
pub fn get_extinguisher(
    primary: Option<&dyn ExtinguisherItem>,
    secondary: Option<&dyn ExtinguisherItem>,
    inventory: &[&dyn ExtinguisherItem],
) -> Option<usize> {
    if is_extinguisher(primary) {
        return Some(0);
    }
    if is_extinguisher(secondary) {
        return Some(1);
    }
    let (mut extinguisher, mut bag, mut water) = (None, None, None);
    for (i, &it) in inventory.iter().enumerate() {
        if !is_extinguisher(Some(it)) {
            continue;
        }
        if it.is_water_source() && water.is_none() {
            water = Some(i + 2);
        }
        if it.item_type() == "Extinguisher" && extinguisher.is_none() {
            extinguisher = Some(i + 2);
        }
        if is_bag(it.item_type()) && bag.is_none() {
            bag = Some(i + 2);
        }
    }
    extinguisher.or(bag).or(water)
}

/// `isSquareToExtinguish(square)`: a burning square with a fire that is
/// not permanent, or a character on fire on it (`character_on_fire`).
pub fn is_square_to_extinguish(
    cell: &IsoCell,
    burning: IsoFlagType,
    at: (i32, i32, i32),
    character_on_fire: &dyn Fn((i32, i32, i32)) -> bool,
) -> bool {
    let Some(sq) = cell.get_grid_square(at.0, at.1, at.2) else {
        return false;
    };
    if sq.has(burning)
        && sq
            .objects
            .iter()
            .any(|o| matches!(&o.kind, ObjectKind::Fire(f) if !f.perm))
    {
        return true;
    }
    character_on_fire(at)
}

/// `getSquareToExtinguish(square)`: the square itself, else the first of
/// its eight neighbours (rows north to south, west to east).
pub fn get_square_to_extinguish(
    cell: &IsoCell,
    burning: IsoFlagType,
    at: (i32, i32, i32),
    character_on_fire: &dyn Fn((i32, i32, i32)) -> bool,
) -> Option<(i32, i32, i32)> {
    if is_square_to_extinguish(cell, burning, at, character_on_fire) {
        return Some(at);
    }
    for dy in -1..=1 {
        for dx in -1..=1 {
            if dx == 0 && dy == 0 {
                continue;
            }
            let p = (at.0 + dx, at.1 + dy, at.2);
            if is_square_to_extinguish(cell, burning, p, character_on_fire) {
                return Some(p);
            }
        }
    }
    None
}
