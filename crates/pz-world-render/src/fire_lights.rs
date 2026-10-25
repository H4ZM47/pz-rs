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

/// One [`LightLink`] per fire in the world's `FireStack`, for the engine's
/// `pz_world::fire::FireHooks`: [`FireLightLinks::updated`] from
/// `fire_updated` (after each `IsoFire.update`) and
/// [`FireLightLinks::removed`] from `fire_removed` (`removeFromWorld`).
#[derive(Clone, Debug, Default)]
pub struct FireLightLinks {
    pub links: std::collections::HashMap<pz_world::fire::FireRef, LightLink>,
}

impl FireLightLinks {
    pub fn new() -> Self {
        Self::default()
    }

    /// `FireHooks::fire_updated`: [`LightLink::sync_fire`].
    pub fn updated(
        &mut self,
        lighting: &mut WorldLighting,
        r: pz_world::fire::FireRef,
        fire: &FireState,
    ) {
        self.links
            .entry(r)
            .or_default()
            .sync_fire(lighting, fire, r.x, r.y, r.z);
    }

    /// `FireHooks::fire_removed`: [`LightLink::remove`].
    pub fn removed(&mut self, lighting: &mut WorldLighting, r: pz_world::fire::FireRef) {
        if let Some(mut l) = self.links.remove(&r) {
            l.remove(lighting);
        }
    }
}

/// What `IsoFireplace.update`'s visual half reads of the world object
/// (`obj`, an `ObjectKind::Fireplace`).
pub fn fireplace_input<'a>(
    obj: &'a pz_world::IsoObject,
    fp: &pz_world::objects::IsoFireplace,
    sprites: &'a pz_world::IsoSpriteManager,
) -> pz_iso_render::hearth::FireplaceInput<'a> {
    let sp = obj.sprite.map(|s| &sprites.sprites[s]);
    let f = &sprites.tables.flag;
    let has = |fl| sp.is_some_and(|s| s.properties.has(fl));
    pz_iso_render::hearth::FireplaceInput {
        lit: fp.lit,
        fuel_amount: fp.fuel_amount(),
        sprite_name: sp.map_or("", |s| s.name()),
        metal_drum: sp
            .is_some_and(|s| s.properties.get(&sprites.alias, "CustomName") == Some("Metal Drum")),
        container_type: obj.container.as_ref().and_then(|c| c.kind.as_deref()),
        collide_w: has(f.collideW),
        collide_n: has(f.collideN),
    }
}

/// A fireplace's or barbecue's visual state and light, kept with the world
/// object: [`HearthLink::update_fireplace`] / [`HearthLink::update_barbecue`]
/// run the whole of `IsoFireplace.update` / `IsoBarbecue.update` (the
/// world's fuel step, then the sprites and light); [`HearthLink::remove`]
/// is `removeFromWorld`.
#[derive(Clone, Debug, Default)]
pub struct HearthLink {
    pub fireplace: FireplaceState,
    pub barbecue: pz_iso_render::hearth::BarbecueState,
    pub light: LightLink,
}

impl HearthLink {
    pub fn new() -> Self {
        Self::default()
    }

    /// `IsoFireplace.update()` of `obj` at `(x, y, z)`: `dt` is
    /// `getMultipliedSecondsSinceLastUpdate() * 60`, `frame_size` a
    /// texture's original size.
    #[allow(clippy::too_many_arguments)]
    pub fn update_fireplace(
        &mut self,
        lighting: &mut WorldLighting,
        obj: &mut pz_world::IsoObject,
        sprites: &pz_world::IsoSpriteManager,
        at: (i32, i32, i32),
        world_age_hours: f64,
        dt: f32,
        rand: &dyn pz_iso_render::fire::RandSource,
        frame_size: &dyn Fn(&str) -> Option<(i32, i32)>,
    ) {
        let pz_world::ObjectKind::Fireplace(fp) = &mut obj.kind else {
            return;
        };
        fp.update_fuel(world_age_hours);
        let fp = fp.clone();
        let input = fireplace_input(obj, &fp, sprites);
        self.fireplace.update(&input, dt, rand, frame_size);
        self.light
            .sync_fireplace(lighting, &self.fireplace, at.0, at.1, at.2);
    }

    /// `IsoBarbecue.update()` of `obj`: the fuel step, the propane sprite
    /// (`updateSprite`), then the smoke.
    pub fn update_barbecue(
        &mut self,
        obj: &mut pz_world::IsoObject,
        sprites: &pz_world::IsoSpriteManager,
        world_age_hours: f64,
        dt: f32,
    ) {
        let propane = obj.sprite.is_some_and(|s| {
            sprites.sprites[s]
                .properties
                .has_name(&sprites.alias, "PropaneTank")
        });
        let pz_world::ObjectKind::Barbecue(b) = &mut obj.kind else {
            return;
        };
        b.update_fuel(world_age_hours);
        let lit = b.lit;
        obj.sprite = b.sprite(obj.sprite, propane);
        self.barbecue.update(lit, dt);
    }

    /// `removeFromWorld()`: the light goes.
    pub fn remove(&mut self, lighting: &mut WorldLighting) {
        self.light.remove(lighting);
    }
}
