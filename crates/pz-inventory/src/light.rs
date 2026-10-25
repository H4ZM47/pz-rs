//! Light-emitting items (flashlights, lanterns, candles, weapon lights):
//! `InventoryItem.canEmitLight`, `isEmittingLight` and the light getters,
//! with `HandWeapon`'s `activeLight` weapon part taking over.

use crate::item::{InventoryItem, ItemKind};
use crate::template::ItemClass;

impl InventoryItem {
    /// Port of `HandWeapon.activeLight`: the first attached weapon part
    /// with `isTorchCone()`. `setWeaponPart` picks it when the part is
    /// attached and only an earlier light part would keep it from being
    /// chosen; loaded weapons attach one part per part type in list order,
    /// which gives the same part.
    pub fn active_light(&self) -> Option<&InventoryItem> {
        match &self.kind {
            ItemKind::Weapon(w) => w.attachments.iter().find(|p| p.is_torch_cone()),
            _ => None,
        }
    }

    /// Port of `getLightStrength()` (`HandWeapon`: the active light's).
    pub fn get_light_strength(&self) -> f32 {
        match self.active_light() {
            Some(l) => l.get_light_strength(),
            None => self.light_strength,
        }
    }

    /// Port of `isTorchCone()` (`HandWeapon`: also the active light's).
    pub fn is_torch_cone(&self) -> bool {
        self.active_light().is_some() || self.torch_cone
    }

    /// Port of `getTorchDot()` (the script's `torchDot`; `HandWeapon`: the
    /// active light's).
    pub fn get_torch_dot(&self) -> f32 {
        match self.active_light() {
            Some(l) => l.get_torch_dot(),
            None => self.template.torch_dot,
        }
    }

    /// Port of `getLightDistance()` (`HandWeapon`: the active light's).
    pub fn get_light_distance(&self) -> i32 {
        match self.active_light() {
            Some(l) => l.get_light_distance(),
            None => self.light_distance,
        }
    }

    /// Port of `canBeActivated()` (`HandWeapon`: the active light's).
    pub fn can_be_activated(&self) -> bool {
        match self.active_light() {
            Some(l) => l.can_be_activated(),
            None => self.can_be_activated,
        }
    }

    /// Port of `setActivated(activated)` (`HandWeapon`: its active light
    /// part too). The client's equip events are not modelled.
    pub fn set_activated(&mut self, activated: bool) {
        if let ItemKind::Weapon(w) = &mut self.kind {
            if let Some(l) = w.attachments.iter_mut().find(|p| p.is_torch_cone()) {
                l.activated = activated;
            }
        }
        self.activated = activated;
    }

    /// Port of `isActivated()`.
    pub fn is_activated(&self) -> bool {
        self.activated
    }

    /// Port of `canEmitLight()`: a positive light strength and, for a
    /// `Drainable` (`DrainableComboItem`), uses left. `HandWeapon`: or its
    /// active light can.
    pub fn can_emit_light(&self) -> bool {
        if self.active_light().is_some_and(|l| l.can_emit_light()) {
            return true;
        }
        if self.get_light_strength() <= 0.0 {
            return false;
        }
        self.template.class != ItemClass::Drainable || self.get_current_uses() > 0
    }

    /// Port of `isEmittingLight()`: can emit light and is switched on (or
    /// cannot be switched).
    pub fn is_emitting_light(&self) -> bool {
        self.can_emit_light() && (!self.can_be_activated() || self.is_activated())
    }
}
