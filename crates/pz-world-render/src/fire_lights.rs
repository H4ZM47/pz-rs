//! The light sources of burning objects in the cell's light list: an
//! `IsoFire`'s and an `IsoFireplace`'s `lightSource`, kept in step with the
//! visual state pz-iso-render computes ([`FireState::light`],
//! [`FireplaceState::light`]) through the [`WorldLighting`] lamppost calls,
//! in the original's call order.
//!
//! The owner keeps one [`LightLink`] per object, calls the matching
//! `sync_*` after the object's update and [`LightLink::remove`] from its
//! `removeFromWorld`. Within one update only the last light counts: a
//! light replaced twice before `checkLights` is never sent either way.

use pz_iso_render::fire::{FireLight, FireState};
use pz_iso_render::hearth::FireplaceState;

use crate::lighting::{CellLight, WorldLighting};

/// An object's `lightSource` in the cell's list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LightLink {
    /// The lamppost key, while the object has a light.
    pub key: Option<u64>,
    radius: i32,
}

impl LightLink {
    pub fn new() -> Self {
        Self::default()
    }

    /// After `IsoFire.update`: the first update's
    /// `currentCell.addLamppost(new IsoLightSource(…))`, the stage-5 colour
    /// writes (`lightSource.r/g/b = LIGHT_* * alpha`), and `setLightRadius`'s
    /// replacement (`removeLamppost(old)`, `getLamppostPositions().add(new)`,
    /// then the update flags again).
    pub fn sync_fire(
        &mut self,
        lighting: &mut WorldLighting,
        fire: &FireState,
        x: i32,
        y: i32,
        z: i32,
    ) {
        match (fire.light, self.key) {
            (Some(l), None) => self.add(lighting, l, x, y, z, true),
            (Some(l), Some(key)) if l.radius != self.radius => {
                lighting.remove_lamppost(key);
                self.add(lighting, l, x, y, z, false);
                lighting.mark_lights_dirty();
            }
            (Some(l), Some(key)) => Self::colour(lighting, key, l),
            (None, Some(_)) => self.remove(lighting),
            (None, None) => {}
        }
    }

    /// After `IsoFireplace.update` (`updateLightSource`): lit, a new radius
    /// kills the old light (`life = 0`) and adds a new one
    /// (`addLamppost`, then the update flags again); put out,
    /// `removeLamppost`.
    pub fn sync_fireplace(
        &mut self,
        lighting: &mut WorldLighting,
        fp: &FireplaceState,
        x: i32,
        y: i32,
        z: i32,
    ) {
        match (fp.light, self.key) {
            (Some(l), None) => {
                self.add(lighting, l, x, y, z, true);
                lighting.mark_lights_dirty();
            }
            (Some(l), Some(key)) if l.radius != self.radius => {
                lighting.kill_lamppost(key);
                self.add(lighting, l, x, y, z, true);
                lighting.mark_lights_dirty();
            }
            (Some(l), Some(key)) => Self::colour(lighting, key, l),
            (None, Some(_)) => self.remove(lighting),
            (None, None) => {}
        }
    }

    /// `removeFromWorld`: `removeLamppost(lightSource)`, `lightSource = null`.
    pub fn remove(&mut self, lighting: &mut WorldLighting) {
        if let Some(key) = self.key.take() {
            lighting.remove_lamppost(key);
        }
    }

    fn add(
        &mut self,
        lighting: &mut WorldLighting,
        l: FireLight,
        x: i32,
        y: i32,
        z: i32,
        flag: bool,
    ) {
        let c = CellLight::new(x, y, z, l.r, l.g, l.b, l.radius);
        self.key = Some(if flag {
            lighting.add_lamppost(c)
        } else {
            lighting.push_lamppost(c)
        });
        self.radius = l.radius;
    }

    fn colour(lighting: &mut WorldLighting, key: u64, l: FireLight) {
        if let Some(c) = lighting.lamppost_mut(key) {
            c.r = l.r;
            c.g = l.g;
            c.b = l.b;
        }
    }
}
