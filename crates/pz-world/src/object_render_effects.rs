//! Build 42.22 `zombie.iso.objects.ObjectRenderEffects` & `RenderEffectType`:
//! corner vertex-offset animations (`x1..y4`) for tree/vegetation wind sway,
//! `Hit_Tree_Shudder`, `Vegetation_Rustle`, and `Hit_Door`.

/// Build 42.22 `zombie.iso.objects.RenderEffectType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RenderEffectType {
    HitTreeShudder,
    VegetationRustle,
    HitDoor,
}

impl RenderEffectType {
    pub const ALL: [Self; 3] = [
        Self::HitTreeShudder,
        Self::VegetationRustle,
        Self::HitDoor,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::HitTreeShudder => "Hit_Tree_Shudder",
            Self::VegetationRustle => "Vegetation_Rustle",
            Self::HitDoor => "Hit_Door",
        }
    }

    pub fn ordinal(self) -> usize {
        match self {
            Self::HitTreeShudder => 0,
            Self::VegetationRustle => 1,
            Self::HitDoor => 2,
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.name() == s)
    }
}

/// Build 42.22 `zombie.iso.objects.ObjectRenderEffects`.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectRenderEffects {
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
    pub x3: f64,
    pub y3: f64,
    pub x4: f64,
    pub y4: f64,
    tx1: f64,
    ty1: f64,
    tx2: f64,
    ty2: f64,
    tx3: f64,
    ty3: f64,
    tx4: f64,
    ty4: f64,
    lx1: f64,
    ly1: f64,
    lx2: f64,
    ly2: f64,
    lx3: f64,
    ly3: f64,
    lx4: f64,
    ly4: f64,
    pub max_time: f32,
    pub cur_time: f32,
    pub total_time: f32,
    pub total_max_time: f32,
    pub effect_type: RenderEffectType,
    pub is_big: bool,
    pub is_wind: bool,
    pub wind_type: i32,
    pub finish: bool,
}

impl Default for ObjectRenderEffects {
    fn default() -> Self {
        Self {
            x1: 0.0,
            y1: 0.0,
            x2: 0.0,
            y2: 0.0,
            x3: 0.0,
            y3: 0.0,
            x4: 0.0,
            y4: 0.0,
            tx1: 0.0,
            ty1: 0.0,
            tx2: 0.0,
            ty2: 0.0,
            tx3: 0.0,
            ty3: 0.0,
            tx4: 0.0,
            ty4: 0.0,
            lx1: 0.0,
            ly1: 0.0,
            lx2: 0.0,
            ly2: 0.0,
            lx3: 0.0,
            ly3: 0.0,
            lx4: 0.0,
            ly4: 0.0,
            max_time: 0.0,
            cur_time: 0.0,
            total_time: 0.0,
            total_max_time: 60.0,
            effect_type: RenderEffectType::VegetationRustle,
            is_big: false,
            is_wind: false,
            wind_type: 1,
            finish: false,
        }
    }
}

impl ObjectRenderEffects {
    /// `ObjectRenderEffects.getNew(parent, type, isBig)`.
    pub fn new_effect(effect_type: RenderEffectType, is_big: bool) -> Self {
        let mut e = Self {
            effect_type,
            is_big,
            ..Default::default()
        };
        match effect_type {
            RenderEffectType::HitTreeShudder => {
                e.total_max_time = if is_big { 60.0 } else { 45.0 };
                e.max_time = 6.0;
                e.tx1 = if is_big { 0.025 } else { 0.015 };
                e.tx2 = e.tx1;
            }
            RenderEffectType::VegetationRustle => {
                e.total_max_time = 45.0;
                e.max_time = 8.0;
                e.tx1 = 0.012;
                e.tx2 = -0.012;
            }
            RenderEffectType::HitDoor => {
                e.total_max_time = 30.0;
                e.max_time = 5.0;
                e.tx1 = 0.018;
                e.tx2 = 0.018;
            }
        }
        e
    }

    /// `copyMainFromOther(other)`.
    pub fn copy_main_from_other(&mut self, other: &Self) {
        self.x1 = other.x1;
        self.y1 = other.y1;
        self.x2 = other.x2;
        self.y2 = other.y2;
        self.x3 = other.x3;
        self.y3 = other.y3;
        self.x4 = other.x4;
        self.y4 = other.y4;
    }

    /// `add(other)`.
    pub fn add(&mut self, other: &Self) {
        self.x1 += other.x1;
        self.y1 += other.y1;
        self.x2 += other.x2;
        self.y2 += other.y2;
        self.x3 += other.x3;
        self.y3 += other.y3;
        self.x4 += other.x4;
        self.y4 += other.y4;
    }

    /// `update()` for a non-wind effect (`dt` in 60Hz ticks, default `1.0`).
    /// Returns `true` while still active, `false` once finished.
    pub fn update(&mut self, dt: f32) -> bool {
        if self.is_wind {
            return true;
        }
        self.cur_time += dt;
        self.total_time += dt;
        if self.total_time >= self.total_max_time {
            self.finish = true;
            self.x1 = 0.0;
            self.y1 = 0.0;
            self.x2 = 0.0;
            self.y2 = 0.0;
            self.x3 = 0.0;
            self.y3 = 0.0;
            self.x4 = 0.0;
            self.y4 = 0.0;
            return false;
        }
        let max_t = self.max_time.max(1.0);
        if self.cur_time >= max_t {
            self.cur_time -= max_t;
            self.lx1 = self.tx1;
            self.ly1 = self.ty1;
            self.lx2 = self.tx2;
            self.ly2 = self.ty2;
            self.lx3 = self.tx3;
            self.ly3 = self.ty3;
            self.lx4 = self.tx4;
            self.ly4 = self.ty4;
            let damp = 1.0 - (self.total_time / self.total_max_time).clamp(0.0, 1.0) as f64;
            self.tx1 = -self.tx1 * damp;
            self.tx2 = -self.tx2 * damp;
        }
        let f = (self.cur_time / max_t).clamp(0.0, 1.0) as f64;
        self.x1 = self.lx1 + (self.tx1 - self.lx1) * f;
        self.y1 = self.ly1 + (self.ty1 - self.ly1) * f;
        self.x2 = self.lx2 + (self.tx2 - self.lx2) * f;
        self.y2 = self.ly2 + (self.ty2 - self.ly2) * f;
        self.x3 = self.lx3 + (self.tx3 - self.lx3) * f;
        self.y3 = self.ly3 + (self.ty3 - self.ly3) * f;
        self.x4 = self.lx4 + (self.tx4 - self.lx4) * f;
        self.y4 = self.ly4 + (self.ty4 - self.ly4) * f;
        true
    }
}

/// Static wind-effect pools initialized by `ObjectRenderEffects.init()`
/// (`RANDOM_RUSTLE` / `WIND_EFFECTS` / `WIND_EFFECTS_TREES`, 15 entries each).
#[derive(Clone, Debug, Default)]
pub struct ObjectRenderEffectsPool {
    pub initialized: bool,
    pub wind_effects: Vec<ObjectRenderEffects>,
    pub wind_effects_trees: Vec<ObjectRenderEffects>,
    next_veg_idx: usize,
    next_tree_idx: usize,
    phase: f64,
}

impl ObjectRenderEffectsPool {
    pub const POOL_SIZE: usize = 15;

    /// `ObjectRenderEffects.init()`.
    pub fn init(&mut self) {
        self.wind_effects.clear();
        self.wind_effects_trees.clear();
        for i in 0..Self::POOL_SIZE {
            let mut veg = ObjectRenderEffects {
                is_wind: true,
                wind_type: (i % 3 + 1) as i32,
                ..Default::default()
            };
            veg.cur_time = (i as f32) * 3.0;
            self.wind_effects.push(veg);

            let mut tree = ObjectRenderEffects {
                is_wind: true,
                is_big: true,
                wind_type: (i % 3 + 1) as i32,
                ..Default::default()
            };
            tree.cur_time = (i as f32) * 4.0;
            self.wind_effects_trees.push(tree);
        }
        self.next_veg_idx = 0;
        self.next_tree_idx = 0;
        self.initialized = true;
    }

    /// `ObjectRenderEffects.getNextWindEffect(windType, isTree)`.
    pub fn next_wind_effect(&mut self, _wind_type: i32, is_tree: bool) -> Option<ObjectRenderEffects> {
        if !self.initialized {
            self.init();
        }
        if is_tree {
            let len = self.wind_effects_trees.len();
            if len == 0 {
                return None;
            }
            let idx = self.next_tree_idx % len;
            self.next_tree_idx = (idx + 1) % len;
            Some(self.wind_effects_trees[idx].clone())
        } else {
            let len = self.wind_effects.len();
            if len == 0 {
                return None;
            }
            let idx = self.next_veg_idx % len;
            self.next_veg_idx = (idx + 1) % len;
            Some(self.wind_effects[idx].clone())
        }
    }

    /// `ObjectRenderEffects.updateStatic()`.
    pub fn update_static(&mut self, wind_intensity: f32, dt: f32) {
        if !self.initialized {
            return;
        }
        let intensity = wind_intensity.clamp(0.0, 1.0) as f64;
        self.phase += (dt as f64) * (0.04 + intensity * 0.08);
        for (i, e) in self.wind_effects.iter_mut().enumerate() {
            let sway = (self.phase + (i as f64) * 0.45).sin() * 0.015 * intensity;
            e.x1 = sway;
            e.x2 = sway;
        }
        for (i, e) in self.wind_effects_trees.iter_mut().enumerate() {
            let sway = (self.phase * 0.7 + (i as f64) * 0.6).sin() * 0.03 * intensity;
            e.x1 = sway;
            e.x2 = sway;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_effects_pool_and_update_lifecycle() {
        let mut pool = ObjectRenderEffectsPool::default();
        pool.init();
        assert!(pool.initialized);
        assert_eq!(pool.wind_effects.len(), ObjectRenderEffectsPool::POOL_SIZE);
        assert_eq!(
            pool.wind_effects_trees.len(),
            ObjectRenderEffectsPool::POOL_SIZE
        );

        pool.update_static(0.8, 1.0);
        let w = pool.next_wind_effect(1, true).unwrap();
        assert!(w.is_wind && w.is_big);

        let mut hit = ObjectRenderEffects::new_effect(RenderEffectType::HitTreeShudder, true);
        assert!(hit.update(1.0));
        assert!(hit.x1.abs() > 0.0);
        assert!(!hit.update(100.0));
        assert!(hit.finish && hit.x1 == 0.0);
    }
}
