//! `zombie.erosion.obj`: the sprite sets of erosion objects
//! (`ErosionObjSprites`, `ErosionObjOverlaySprites`) and the objects that
//! place and stage them (`ErosionObj`, `ErosionObjOverlay`).

use std::collections::HashMap;

use crate::host::{ErosionHost, ObjId, Sq};

pub const SECTION_BASE: usize = 0;
pub const SECTION_SNOW: usize = 1;
pub const SECTION_FLOWER: usize = 2;
pub const SECTION_CHILD: usize = 3;
pub const NUM_SECTIONS: usize = 4;

/// `ErosionObjSprites.Sprites`: one sprite, or a list handed out round
/// robin.
#[derive(Clone, Debug)]
struct Sprites {
    sprites: Vec<String>,
    index: i32,
}

impl Sprites {
    fn get_next(&mut self) -> String {
        self.index += 1;
        if self.index as usize >= self.sprites.len() {
            self.index = 0;
        }
        self.sprites[self.index as usize].clone()
    }
}

type Section = [Option<Sprites>; 6];

/// `ErosionObjSprites.Entry`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub stage: i32,
    pub section: i32,
    pub season: i32,
}

/// `ErosionObjSprites`.
#[derive(Clone, Debug)]
pub struct ErosionObjSprites {
    pub name: String,
    pub stages: i32,
    pub has_snow: bool,
    pub has_flower: bool,
    pub has_child_sprite: bool,
    rev_lookup: HashMap<String, Entry>,
    sprites: Vec<[Option<Section>; NUM_SECTIONS]>,
}

impl ErosionObjSprites {
    pub fn new(
        stages: i32,
        name: &str,
        has_snow: bool,
        has_flower: bool,
        has_child_sprite: bool,
    ) -> Self {
        let sprites = (0..stages)
            .map(|_| {
                let mut s: [Option<Section>; NUM_SECTIONS] = Default::default();
                s[SECTION_BASE] = Some(Default::default());
                if has_snow {
                    s[SECTION_SNOW] = Some(Default::default());
                }
                if has_flower {
                    s[SECTION_FLOWER] = Some(Default::default());
                }
                if has_child_sprite {
                    s[SECTION_CHILD] = Some(Default::default());
                }
                s
            })
            .collect();
        ErosionObjSprites {
            name: name.to_string(),
            stages,
            has_snow,
            has_flower,
            has_child_sprite,
            rev_lookup: HashMap::new(),
            sprites,
        }
    }

    fn get_sprite(&mut self, stage: i32, section: usize, season: i32) -> Option<String> {
        self.sprites.get_mut(stage as usize)?[section]
            .as_mut()?
            .get_mut(season as usize)?
            .as_mut()
            .map(Sprites::get_next)
    }

    pub fn get_base(&mut self, stage: i32, season: i32) -> Option<String> {
        self.get_sprite(stage, SECTION_BASE, season)
    }

    pub fn get_flower(&mut self, stage: i32) -> Option<String> {
        if self.has_flower {
            self.get_sprite(stage, SECTION_FLOWER, 0)
        } else {
            None
        }
    }

    pub fn get_child_sprite(&mut self, stage: i32, season: i32) -> Option<String> {
        if self.has_child_sprite {
            self.get_sprite(stage, SECTION_CHILD, season)
        } else {
            None
        }
    }

    pub fn get_entry(&self, sprite: &str) -> Option<Entry> {
        self.rev_lookup.get(sprite).copied()
    }

    fn set_sprites(&mut self, stage: i32, section: usize, sprites: Vec<String>, season: i32) {
        let Some(sec) = self
            .sprites
            .get_mut(stage as usize)
            .and_then(|s| s[section].as_mut())
        else {
            return;
        };
        sec[season as usize] = Some(Sprites {
            sprites: sprites.clone(),
            index: -1,
        });
        for s in sprites {
            self.rev_lookup.insert(
                s,
                Entry {
                    stage,
                    section: section as i32,
                    season,
                },
            );
        }
    }

    pub fn set_base(&mut self, stage: i32, sprite: &str, season: i32) {
        self.set_sprites(stage, SECTION_BASE, vec![sprite.to_string()], season);
    }

    pub fn set_base_list(&mut self, stage: i32, sprites: &[String], season: i32) {
        self.set_sprites(stage, SECTION_BASE, sprites.to_vec(), season);
    }

    pub fn set_flower(&mut self, stage: i32, sprite: &str) {
        self.set_sprites(stage, SECTION_FLOWER, vec![sprite.to_string()], 0);
    }

    pub fn set_child_sprite(&mut self, stage: i32, sprite: &str, season: i32) {
        self.set_sprites(stage, SECTION_CHILD, vec![sprite.to_string()], season);
    }
}

/// `ErosionObj`.
#[derive(Clone, Debug)]
pub struct ErosionObj {
    sprites: ErosionObjSprites,
    pub name: String,
    pub stages: i32,
    pub has_snow: bool,
    pub has_flower: bool,
    pub has_child_sprite: bool,
    pub bloom_start: f32,
    pub bloom_end: f32,
    pub no_season_base: bool,
    pub cycle_time: i32,
}

impl ErosionObj {
    pub fn new(
        sprites: ErosionObjSprites,
        cycle_time: i32,
        bloom_start: f32,
        bloom_end: f32,
        no_season_base: bool,
    ) -> Self {
        ErosionObj {
            name: sprites.name.clone(),
            stages: sprites.stages,
            has_snow: sprites.has_snow,
            has_flower: sprites.has_flower,
            has_child_sprite: sprites.has_child_sprite,
            sprites,
            bloom_start,
            bloom_end,
            no_season_base,
            cycle_time,
        }
    }

    fn base_season(&self, season: i32) -> i32 {
        if self.no_season_base {
            0
        } else {
            season
        }
    }

    /// `getObject(square, remove)`: the topmost object named like this one.
    pub fn get_object(&self, host: &mut dyn ErosionHost, sq: Sq, remove: bool) -> Option<ObjId> {
        let objs = host.objects(sq);
        for &o in objs.iter().rev() {
            if host.object_name(sq, o).as_deref() != Some(self.name.as_str()) {
                continue;
            }
            if remove {
                host.remove_object_raw(sq, o);
            }
            return Some(o);
        }
        None
    }

    /// `placeObject(square, stage, tree, season, bloom)`: `createObject`
    /// (which draws one base sprite), then `setStageObject` and adding the
    /// object to the square. A stage outside the sprite set (an
    /// `ArrayIndexOutOfBoundsException` upstream) places nothing.
    pub fn place_object(
        &mut self,
        host: &mut dyn ErosionHost,
        sq: Sq,
        stage: i32,
        tree: bool,
        season: i32,
        bloom: bool,
    ) -> bool {
        if stage < 0 || stage >= self.sprites.sprites.len() as i32 {
            return false;
        }
        let sprite = self
            .sprites
            .get_base(stage, self.base_season(season))
            .unwrap_or_default();
        if stage >= self.stages {
            return false;
        }
        let o = host.add_object(sq, &sprite, &self.name, tree);
        self.set_stage_object(host, sq, stage, o, season, bloom);
        true
    }

    /// `setStageObject(stage, obj, season, bloom)`.
    pub fn set_stage_object(
        &mut self,
        host: &mut dyn ErosionHost,
        sq: Sq,
        stage: i32,
        o: ObjId,
        season: i32,
        bloom: bool,
    ) -> bool {
        if stage < 0 || stage >= self.stages {
            return false;
        }
        let Some(sprite) = self.sprites.get_base(stage, self.base_season(season)) else {
            host.set_object_sprite(sq, o, "");
            if host.attached(sq, o).is_some() {
                host.attached_clear(sq, o);
            }
            return true;
        };
        host.set_object_sprite(sq, o, &sprite);
        if self.has_child_sprite || self.has_flower {
            host.attached_clear(sq, o);
            if self.has_child_sprite && season != 0 {
                if let Some(child) = self.sprites.get_child_sprite(stage, season) {
                    host.attached_add(sq, o, &child);
                }
            }
            if self.has_flower && bloom {
                if let Some(flower) = self.sprites.get_flower(stage) {
                    host.attached_add(sq, o, &flower);
                }
            }
        }
        true
    }

    /// `setStage(square, stage, season, bloom)`.
    pub fn set_stage(
        &mut self,
        host: &mut dyn ErosionHost,
        sq: Sq,
        stage: i32,
        season: i32,
        bloom: bool,
    ) -> bool {
        match self.get_object(host, sq, false) {
            Some(o) => self.set_stage_object(host, sq, stage, o, season, bloom),
            None => false,
        }
    }

    /// `removeObject(square)`.
    pub fn remove_object(&self, host: &mut dyn ErosionHost, sq: Sq) -> Option<ObjId> {
        self.get_object(host, sq, true)
    }

    pub fn get_entry(&self, sprite: &str) -> Option<Entry> {
        self.sprites.get_entry(sprite)
    }
}

/// `ErosionObjOverlaySprites`.
#[derive(Clone, Debug)]
pub struct ErosionObjOverlaySprites {
    pub name: String,
    pub stages: i32,
    sprites: Vec<[Option<String>; 6]>,
}

impl ErosionObjOverlaySprites {
    pub fn new(stages: i32, name: &str) -> Self {
        ErosionObjOverlaySprites {
            name: name.to_string(),
            stages,
            sprites: vec![Default::default(); stages.max(0) as usize],
        }
    }

    /// The sprite name of `getSprite(stage, season)`.
    pub fn get_sprite(&self, stage: i32, season: i32) -> Option<&str> {
        self.sprites
            .get(stage as usize)?
            .get(season as usize)?
            .as_deref()
    }

    pub fn set_sprite(&mut self, stage: i32, sprite: &str, season: i32) {
        self.sprites[stage as usize][season as usize] = Some(sprite.to_string());
    }
}

/// `ErosionObjOverlay`.
#[derive(Clone, Debug)]
pub struct ErosionObjOverlay {
    sprites: ErosionObjOverlaySprites,
    pub name: String,
    pub stages: i32,
    pub apply_alpha: bool,
    pub cycle_time: i32,
}

impl ErosionObjOverlay {
    pub fn new(sprites: ErosionObjOverlaySprites, cycle_time: i32, apply_alpha: bool) -> Self {
        ErosionObjOverlay {
            name: sprites.name.clone(),
            stages: sprites.stages,
            sprites,
            apply_alpha,
            cycle_time,
        }
    }

    /// `setOverlay(obj, curId, stage, season, alpha)`: the new overlay's
    /// sprite id, or -1.
    #[allow(clippy::too_many_arguments)]
    pub fn set_overlay(
        &self,
        host: &mut dyn ErosionHost,
        sq: Sq,
        o: ObjId,
        cur_id: i32,
        stage: i32,
        season: i32,
        _alpha: f32,
    ) -> i32 {
        if stage < 0 || stage >= self.stages {
            return -1;
        }
        if cur_id >= 0 {
            self.remove_overlay(host, sq, o, cur_id);
        }
        // A missing sprite is a NullPointerException upstream; nothing is
        // attached then.
        let Some(sprite) = self.sprites.get_sprite(stage, season) else {
            return -1;
        };
        host.attached_add(sq, o, sprite)
    }

    /// `removeOverlay(obj, id)`.
    pub fn remove_overlay(&self, host: &mut dyn ErosionHost, sq: Sq, o: ObjId, id: i32) -> bool {
        let Some(list) = host.attached(sq, o) else {
            return false;
        };
        if list.is_empty() {
            return false;
        }
        // First pass: every instance whose parent sprite has the id.
        let mut j: i32 = 0;
        while (j as usize) < host.attached(sq, o).map_or(0, |l| l.len()) {
            if host.attached(sq, o).unwrap()[j as usize].id == id {
                host.attached_remove(sq, o, j as usize);
                j -= 1;
            }
            j += 1;
        }
        // Second pass (same test, so it only finds what the first missed).
        let list = host.attached(sq, o).unwrap_or_default();
        for i in (0..list.len()).rev() {
            if list[i].id == id {
                host.attached_remove(sq, o, i);
                return true;
            }
        }
        false
    }
}
