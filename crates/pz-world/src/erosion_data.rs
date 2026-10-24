//! `ErosionData.Chunk`/`Square` and the per-category `ErosionCategory.Data`
//! records as chunk files store them: plain data and the byte format only.
//! The erosion system itself (stages, sprites, regions) lives in
//! `pz-erosion`, which drives these fields through
//! [`crate::save::chunk_file::ChunkLoadHook`].
//!
//! A category record's tail depends on its class, so reading needs the
//! byte layout per `(regionId, categoryId)` pair as `ErosionRegions.init`
//! registers them: region 0 NatureTrees, NatureBush, NaturePlants,
//! NatureGeneric; region 1 StreetCracks; region 2 WallVines, WallCracks;
//! region 3 Flowerbed.

use crate::save::io::{err, has, unpack_unit, Reader, SaveResult, Writer};

/// `ErosionData.Chunk`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ErosionChunkData {
    pub init: bool,
    pub e_tick_stamp: i32,
    pub epoch: i32,
    pub moisture: f32,
    pub minerals: f32,
    pub soil: i8,
}

impl ErosionChunkData {
    pub fn load(r: &mut Reader<'_>) -> SaveResult<Self> {
        let mut c = ErosionChunkData {
            init: r.bool()?,
            ..Default::default()
        };
        if c.init {
            c.e_tick_stamp = r.i32()?;
            c.epoch = r.i32()?;
            c.moisture = r.f32()?;
            c.minerals = r.f32()?;
            c.soil = r.i8()?;
        }
        Ok(c)
    }

    pub fn save(&self, w: &mut Writer) {
        if self.init {
            w.i8(1);
            w.i32(self.e_tick_stamp);
            w.i32(self.epoch);
            w.f32(self.moisture);
            w.f32(self.minerals);
            w.i8(self.soil);
        } else {
            w.i8(0);
        }
    }
}

/// `{gameObj, spawnTime, curId[, alpha]}` of a wall category's top part.
#[derive(Clone, Debug, PartialEq)]
pub struct ErosionTop {
    pub game_obj: i8,
    pub spawn_time: i16,
    pub cur_id: i32,
    pub alpha: f32,
}

/// The category-specific tail of an `ErosionCategory.Data`.
#[derive(Clone, Debug, PartialEq)]
pub enum ErosionExtra {
    /// NatureTrees, NatureBush.
    Grown {
        game_obj: i8,
        max_stage: i8,
        spawn_time: i16,
    },
    NatureGeneric {
        game_obj: i8,
        max_stage: i8,
        spawn_time: i16,
        not_grass: bool,
    },
    NaturePlants {
        game_obj: i8,
        spawn_time: i16,
    },
    StreetCracks {
        game_obj: i8,
        max_stage: i8,
        spawn_time: i16,
        cur_id: i32,
        has_grass: bool,
    },
    WallVines {
        game_obj: i8,
        max_stage: i8,
        spawn_time: i16,
        cur_id: i32,
        top: Option<ErosionTop>,
    },
    WallCracks {
        game_obj: i8,
        spawn_time: i16,
        cur_id: i32,
        alpha: f32,
        top: Option<ErosionTop>,
    },
    Flowerbed {
        game_obj: i8,
    },
}

/// `ErosionCategory.Data` as saved.
#[derive(Clone, Debug, PartialEq)]
pub struct ErosionCategoryData {
    pub region_id: i8,
    pub category_id: i8,
    pub do_nothing: bool,
    pub has_spawned: bool,
    pub stage: i32,
    pub disp_season: i8,
    pub disp_bloom: bool,
    pub extra: ErosionExtra,
}

#[derive(Clone, Copy)]
enum Layout {
    Grown,
    NatureGeneric,
    NaturePlants,
    StreetCracks,
    WallVines,
    WallCracks,
    Flowerbed,
}

fn layout(region: i8, category: i8) -> Option<Layout> {
    Some(match (region, category) {
        (0, 0) | (0, 1) => Layout::Grown,
        (0, 2) => Layout::NaturePlants,
        (0, 3) => Layout::NatureGeneric,
        (1, 0) => Layout::StreetCracks,
        (2, 0) => Layout::WallVines,
        (2, 1) => Layout::WallCracks,
        (3, 0) => Layout::Flowerbed,
        _ => return None,
    })
}

impl ErosionCategoryData {
    /// `ErosionCategory.loadCategoryData`.
    pub fn load(r: &mut Reader<'_>) -> SaveResult<Self> {
        let region_id = r.i8()?;
        let category_id = r.i8()?;
        let Some(lay) = layout(region_id, category_id) else {
            // ErosionRegions.getCategory returns null: NullPointerException.
            return err(format!("no erosion category {region_id}/{category_id}"));
        };
        let disp_season = r.i8()?;
        let flags = r.i8()? as u8;
        let mut stage = 0;
        if flags & 8 != 0 {
            stage = 1;
        } else if flags & 0x10 != 0 {
            stage = 2;
        } else if flags & 0x20 != 0 {
            stage = 3;
        } else if flags & 0x40 != 0 {
            stage = 4;
        } else if flags & 0x80 != 0 {
            stage = r.i8()? as i32;
        }
        let extra = match lay {
            Layout::Grown => ErosionExtra::Grown {
                game_obj: r.i8()?,
                max_stage: r.i8()?,
                spawn_time: r.i16()?,
            },
            Layout::NatureGeneric => ErosionExtra::NatureGeneric {
                game_obj: r.i8()?,
                max_stage: r.i8()?,
                spawn_time: r.i16()?,
                not_grass: r.bool()?,
            },
            Layout::NaturePlants => ErosionExtra::NaturePlants {
                game_obj: r.i8()?,
                spawn_time: r.i16()?,
            },
            Layout::StreetCracks => ErosionExtra::StreetCracks {
                game_obj: r.i8()?,
                max_stage: r.i8()?,
                spawn_time: r.i16()?,
                cur_id: r.i32()?,
                has_grass: r.bool()?,
            },
            Layout::WallVines => {
                let game_obj = r.i8()?;
                let max_stage = r.i8()?;
                let spawn_time = r.i16()?;
                let cur_id = r.i32()?;
                let top = if r.bool()? {
                    Some(ErosionTop {
                        game_obj: r.i8()?,
                        spawn_time: r.i16()?,
                        cur_id: r.i32()?,
                        alpha: 0.0,
                    })
                } else {
                    None
                };
                ErosionExtra::WallVines {
                    game_obj,
                    max_stage,
                    spawn_time,
                    cur_id,
                    top,
                }
            }
            Layout::WallCracks => {
                let game_obj = r.i8()?;
                let spawn_time = r.i16()?;
                let cur_id = r.i32()?;
                let alpha = r.f32()?;
                let top = if r.bool()? {
                    Some(ErosionTop {
                        game_obj: r.i8()?,
                        spawn_time: r.i16()?,
                        cur_id: r.i32()?,
                        alpha: r.f32()?,
                    })
                } else {
                    None
                };
                ErosionExtra::WallCracks {
                    game_obj,
                    spawn_time,
                    cur_id,
                    alpha,
                    top,
                }
            }
            Layout::Flowerbed => ErosionExtra::Flowerbed { game_obj: r.i8()? },
        };
        Ok(ErosionCategoryData {
            region_id,
            category_id,
            do_nothing: flags & 1 != 0,
            has_spawned: flags & 2 != 0,
            stage,
            disp_season,
            disp_bloom: flags & 4 != 0,
            extra,
        })
    }

    pub fn save(&self, w: &mut Writer) {
        let mut flags: u8 = 0;
        if self.do_nothing {
            flags |= 1;
        }
        if self.has_spawned {
            flags |= 2;
        }
        if self.disp_bloom {
            flags |= 4;
        }
        match self.stage {
            1 => flags |= 8,
            2 => flags |= 0x10,
            3 => flags |= 0x20,
            4 => flags |= 0x40,
            s if s > 4 => flags |= 0x80,
            _ => {}
        }
        w.i8(self.region_id);
        w.i8(self.category_id);
        w.i8(self.disp_season);
        w.i8(flags as i8);
        if self.stage > 4 {
            w.i8(self.stage as i8);
        }
        match &self.extra {
            ErosionExtra::Grown {
                game_obj,
                max_stage,
                spawn_time,
            } => {
                w.i8(*game_obj);
                w.i8(*max_stage);
                w.i16(*spawn_time);
            }
            ErosionExtra::NatureGeneric {
                game_obj,
                max_stage,
                spawn_time,
                not_grass,
            } => {
                w.i8(*game_obj);
                w.i8(*max_stage);
                w.i16(*spawn_time);
                w.bool(*not_grass);
            }
            ErosionExtra::NaturePlants {
                game_obj,
                spawn_time,
            } => {
                w.i8(*game_obj);
                w.i16(*spawn_time);
            }
            ErosionExtra::StreetCracks {
                game_obj,
                max_stage,
                spawn_time,
                cur_id,
                has_grass,
            } => {
                w.i8(*game_obj);
                w.i8(*max_stage);
                w.i16(*spawn_time);
                w.i32(*cur_id);
                w.bool(*has_grass);
            }
            ErosionExtra::WallVines {
                game_obj,
                max_stage,
                spawn_time,
                cur_id,
                top,
            } => {
                w.i8(*game_obj);
                w.i8(*max_stage);
                w.i16(*spawn_time);
                w.i32(*cur_id);
                match top {
                    Some(t) => {
                        w.i8(1);
                        w.i8(t.game_obj);
                        w.i16(t.spawn_time);
                        w.i32(t.cur_id);
                    }
                    None => w.i8(0),
                }
            }
            ErosionExtra::WallCracks {
                game_obj,
                spawn_time,
                cur_id,
                alpha,
                top,
            } => {
                w.i8(*game_obj);
                w.i16(*spawn_time);
                w.i32(*cur_id);
                w.f32(*alpha);
                match top {
                    Some(t) => {
                        w.i8(1);
                        w.i8(t.game_obj);
                        w.i16(t.spawn_time);
                        w.i32(t.cur_id);
                        w.f32(t.alpha);
                    }
                    None => w.i8(0),
                }
            }
            ErosionExtra::Flowerbed { game_obj } => w.i8(*game_obj),
        }
    }
}

/// `ErosionData.Square`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ErosionSquareData {
    pub init: bool,
    pub do_nothing: bool,
    pub noise_main_byte: i8,
    pub soil: i8,
    pub magic_num_byte: i8,
    pub regions: Vec<ErosionCategoryData>,
}

impl ErosionSquareData {
    /// `noiseMain` as derived on load.
    pub fn noise_main(&self) -> f32 {
        unpack_unit(self.noise_main_byte)
    }

    pub fn load(r: &mut Reader<'_>) -> SaveResult<Self> {
        let flags = r.i8()? as i64 as u64;
        let mut s = ErosionSquareData {
            init: has(flags, 1),
            ..Default::default()
        };
        if s.init {
            s.do_nothing = has(flags, 2);
            s.noise_main_byte = r.i8()?;
            s.soil = r.i8()?;
            s.magic_num_byte = r.i8()?;
            let count = if has(flags, 4) {
                1
            } else if has(flags, 8) {
                2
            } else if has(flags, 16) {
                3
            } else if has(flags, 32) {
                4
            } else if has(flags, 64) {
                r.i8()? as i32
            } else {
                0
            };
            for _ in 0..count.max(0) {
                s.regions.push(ErosionCategoryData::load(r)?);
            }
        }
        Ok(s)
    }

    pub fn save(&self, w: &mut Writer) {
        if !self.init {
            w.i8(0);
            return;
        }
        let at = w.pos();
        w.i8(0);
        let mut flags: u8 = 1;
        if self.do_nothing {
            flags |= 2;
        }
        w.i8(self.noise_main_byte);
        w.i8(self.soil);
        w.i8(self.magic_num_byte);
        match self.regions.len() {
            0 => {}
            1 => flags |= 4,
            2 => flags |= 8,
            3 => flags |= 16,
            4 => flags |= 32,
            n => {
                flags |= 64;
                w.i8(n as i8);
            }
        }
        for c in &self.regions {
            c.save(w);
        }
        w.patch_i8(at, flags as i8);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square_round_trip() {
        let sq = ErosionSquareData {
            init: true,
            do_nothing: true,
            noise_main_byte: -5,
            soil: 3,
            magic_num_byte: 7,
            regions: vec![
                ErosionCategoryData {
                    region_id: 2,
                    category_id: 1,
                    do_nothing: false,
                    has_spawned: true,
                    stage: 9,
                    disp_season: -1,
                    disp_bloom: true,
                    extra: ErosionExtra::WallCracks {
                        game_obj: 1,
                        spawn_time: 300,
                        cur_id: 99,
                        alpha: 0.5,
                        top: Some(ErosionTop {
                            game_obj: 2,
                            spawn_time: 3,
                            cur_id: 4,
                            alpha: 0.25,
                        }),
                    },
                };
                5
            ],
        };
        let mut w = Writer::new();
        sq.save(&mut w);
        let mut r = Reader::new(&w.buf);
        assert_eq!(ErosionSquareData::load(&mut r).unwrap(), sq);
        assert_eq!(r.remaining(), 0);
        let c = ErosionChunkData {
            init: true,
            soil: -2,
            ..Default::default()
        };
        let mut w = Writer::new();
        c.save(&mut w);
        assert_eq!(ErosionChunkData::load(&mut Reader::new(&w.buf)).unwrap(), c);
    }
}
