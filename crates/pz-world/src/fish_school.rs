//! `zombie.iso.FishSchoolManager`: spatial fish & trash abundance zones,
//! fishing depletion (`catchFish`), chum attraction (`addChum`), sound
//! disturbance (`addSoundNoise`), and time-based school regeneration.

use std::collections::HashMap;

/// Grid cell size (in world squares) for procedural school/depletion tracking.
const ZONE_GRID_SIZE: i32 = 30;
/// Default baseline fish count per 30x30 water region before modulation.
const BASE_FISH_MIN: f32 = 8.0;
const BASE_FISH_MAX: f32 = 34.0;
/// Fish regenerated per in-game hour per depleted zone.
const REGEN_PER_HOUR: f32 = 0.75;
/// Duration (in game hours) that chum remains active.
const CHUM_DURATION_HOURS: f64 = 2.0;
/// Duration (in game hours) that a loud sound scares fish away.
const NOISE_DURATION_HOURS: f64 = 0.5;
/// Radius (in world squares) affected by a chum drop.
const CHUM_RADIUS: f32 = 10.0;

#[derive(Clone, Debug)]
struct ChumSpot {
    x: f32,
    y: f32,
    force: f32,
    expire_hours: f64,
}

#[derive(Clone, Debug)]
struct NoiseSpot {
    x: f32,
    y: f32,
    radius: f32,
    expire_hours: f64,
}

/// Spatial fish/trash abundance state (`zombie.iso.FishSchoolManager`).
#[derive(Clone, Debug)]
pub struct FishSchoolManager {
    seed: i32,
    last_update_hours: f64,
    initialized: bool,
    /// Per-zone caught fish depletion, indexed by `(gx, gy)` where `gx = x.div_euclid(30)`.
    depletion: HashMap<(i32, i32), f32>,
    chum: Vec<ChumSpot>,
    noise: Vec<NoiseSpot>,
}

impl Default for FishSchoolManager {
    fn default() -> Self {
        Self {
            seed: 1,
            last_update_hours: 0.0,
            initialized: false,
            depletion: HashMap::new(),
            chum: Vec::new(),
            noise: Vec::new(),
        }
    }
}

fn hash_u32(mut v: u32) -> u32 {
    v ^= v >> 16;
    v = v.wrapping_mul(0x7feb_352d);
    v ^= v >> 15;
    v = v.wrapping_mul(0x846c_a68b);
    v ^ (v >> 16)
}

fn cell_hash01(seed: i32, gx: i32, gy: i32, layer: u32) -> f32 {
    let h = hash_u32(
        (seed as u32)
            .wrapping_mul(374_761_393)
            ^ (gx as u32).wrapping_mul(668_265_263)
            ^ (gy as u32).wrapping_mul(2_147_483_647)
            ^ layer.wrapping_mul(1_597_334_677),
    );
    (h & 0x00ff_ffff) as f32 / 16_777_215.0
}

fn zone_key(x: i32, y: i32) -> (i32, i32) {
    (x.div_euclid(ZONE_GRID_SIZE), y.div_euclid(ZONE_GRID_SIZE))
}

impl FishSchoolManager {
    /// `FishSchoolManager.init()`.
    pub fn init(&mut self, seed: i32, world_age_hours: f64) {
        self.reset();
        self.seed = if seed == 0 { 1 } else { seed };
        self.last_update_hours = world_age_hours;
        self.initialized = true;
    }

    /// Clears all zones, chum, and noise state on world unload.
    pub fn reset(&mut self) {
        self.depletion.clear();
        self.chum.clear();
        self.noise.clear();
        self.last_update_hours = 0.0;
        self.initialized = false;
    }

    /// `generateSeed()` / `updateSeed()`.
    pub fn generate_seed(&mut self, next_seed: i32) {
        self.seed = if next_seed == 0 { 1 } else { next_seed };
    }

    /// `updateSeed()`: deterministically rotates the school seed.
    pub fn update_seed(&mut self) {
        self.seed = hash_u32(self.seed as u32 ^ 0x9e37_79b9) as i32;
        if self.seed == 0 {
            self.seed = 1;
        }
    }

    pub fn seed(&self) -> i32 {
        self.seed
    }

    /// `update()`: regenerates depleted schools over elapsed game hours and
    /// expires old chum and sound disturbance spots.
    pub fn update(&mut self, world_age_hours: f64) {
        if !self.initialized {
            self.last_update_hours = world_age_hours;
            self.initialized = true;
            return;
        }
        let elapsed = (world_age_hours - self.last_update_hours).max(0.0) as f32;
        self.last_update_hours = world_age_hours;
        if elapsed > 0.0 {
            let regen = elapsed * REGEN_PER_HOUR;
            self.depletion.retain(|_, dep| {
                *dep = (*dep - regen).max(0.0);
                *dep > 0.01
            });
        }
        self.chum.retain(|c| c.expire_hours > world_age_hours);
        self.noise.retain(|n| n.expire_hours > world_age_hours);
    }

    fn base_fish_abundance(&self, x: i32, y: i32) -> f32 {
        let (gx, gy) = zone_key(x, y);
        let t = cell_hash01(self.seed, gx, gy, 1);
        BASE_FISH_MIN + t * (BASE_FISH_MAX - BASE_FISH_MIN)
    }

    /// `getFishAbundance(x, y)`: live fish abundance at `(x, y)` (roughly
    /// `0.0..=40.0` matching `Bobber.lua` thresholds `< 10`, `<= 25`, `> 25`),
    /// incorporating depletion, active chum, and noise disturbances.
    pub fn get_fish_abundance(&self, x: i32, y: i32) -> f32 {
        let key = zone_key(x, y);
        let base = self.base_fish_abundance(x, y);
        let dep = self.depletion.get(&key).copied().unwrap_or(0.0);

        let (fx, fy) = (x as f32, y as f32);
        let mut chum_bonus = 0.0f32;
        for c in &self.chum {
            let dist = (fx - c.x).hypot(fy - c.y);
            if dist <= CHUM_RADIUS {
                let falloff = 1.0 - (dist / CHUM_RADIUS);
                chum_bonus += (c.force / 10.0) * falloff;
            }
        }

        let mut noise_penalty = 0.0f32;
        for n in &self.noise {
            let dist = (fx - n.x).hypot(fy - n.y);
            if dist <= n.radius && n.radius > 0.0 {
                let falloff = 1.0 - (dist / n.radius);
                noise_penalty += 25.0 * falloff;
            }
        }

        (base - dep + chum_bonus - noise_penalty).max(0.0)
    }

    /// `getTrashAbundance(x, y)`: trash probability factor (`0.05..=0.45`,
    /// consumed by `Fish.lua:getFishByLure`).
    pub fn get_trash_abundance(&self, x: i32, y: i32) -> f32 {
        let (gx, gy) = zone_key(x, y);
        let t = cell_hash01(self.seed, gx, gy, 2);
        0.05 + t * 0.40
    }

    /// `catchFish(x, y)`: reduces local fish abundance after a catch.
    pub fn catch_fish(&mut self, x: i32, y: i32) {
        let key = zone_key(x, y);
        let max_dep = self.base_fish_abundance(x, y);
        let entry = self.depletion.entry(key).or_insert(0.0);
        *entry = (*entry + 1.0).min(max_dep);
    }

    /// `addChum(x, y, force)`: boosts local fish abundance temporarily.
    pub fn add_chum(&mut self, x: i32, y: i32, force: f32) {
        if force <= 0.0 || !force.is_finite() {
            return;
        }
        self.chum.push(ChumSpot {
            x: x as f32,
            y: y as f32,
            force: force.clamp(0.0, 200.0),
            expire_hours: self.last_update_hours + CHUM_DURATION_HOURS,
        });
    }

    /// `addSoundNoise(x, y, radius)`: temporarily scares fish within `radius`.
    pub fn add_sound_noise(&mut self, x: i32, y: i32, radius: i32) {
        if radius <= 0 {
            return;
        }
        self.noise.push(NoiseSpot {
            x: x as f32,
            y: y as f32,
            radius: radius as f32,
            expire_hours: self.last_update_hours + NOISE_DURATION_HOURS,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catch_fish_depletes_and_update_regenerates() {
        let mut mgr = FishSchoolManager::default();
        mgr.init(42, 10.0);
        let initial = mgr.get_fish_abundance(100, 200);
        assert!((BASE_FISH_MIN..=BASE_FISH_MAX).contains(&initial));

        mgr.catch_fish(100, 200);
        mgr.catch_fish(100, 200);
        assert!((mgr.get_fish_abundance(100, 200) - (initial - 2.0)).abs() < 1e-4);

        mgr.update(14.0); // 4 hours * 0.75 = 3.0 regen -> full recovery
        assert!((mgr.get_fish_abundance(100, 200) - initial).abs() < 1e-4);
    }

    #[test]
    fn chum_boosts_and_expires_after_duration() {
        let mut mgr = FishSchoolManager::default();
        mgr.init(7, 5.0);
        let before = mgr.get_fish_abundance(50, 50);
        mgr.add_chum(50, 50, 100.0);
        assert!(mgr.get_fish_abundance(50, 50) > before + 5.0);

        mgr.update(5.0 + CHUM_DURATION_HOURS + 0.1);
        assert!((mgr.get_fish_abundance(50, 50) - before).abs() < 1e-4);
    }

    #[test]
    fn sound_noise_scares_nearby_fish_until_expiry() {
        let mut mgr = FishSchoolManager::default();
        mgr.init(9, 1.0);
        let before = mgr.get_fish_abundance(12, 12);
        mgr.add_sound_noise(12, 12, 10);
        assert!(mgr.get_fish_abundance(12, 12) < before);

        mgr.update(1.0 + NOISE_DURATION_HOURS + 0.1);
        assert!((mgr.get_fish_abundance(12, 12) - before).abs() < 1e-4);
    }

    #[test]
    fn trash_abundance_and_seed_rotation() {
        let mut mgr = FishSchoolManager::default();
        mgr.init(11, 0.0);
        let trash = mgr.get_trash_abundance(300, 400);
        assert!((0.05..=0.45).contains(&trash));
        let prev_seed = mgr.seed();
        mgr.update_seed();
        assert_ne!(mgr.seed(), prev_seed);
    }
}
