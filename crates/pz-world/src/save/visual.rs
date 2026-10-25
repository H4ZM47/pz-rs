//! `HumanVisual`, `ItemVisual`, `AnimalVisual`, `SurvivorDesc` and
//! `IsoDeadBody` as saved in chunk files.

use super::io::{err, Reader, SaveResult, Writer};
use super::items::ItemRegistry;
use super::lua_table::{LuaKey, LuaTable, LuaValue};
use super::SaveTables;
use crate::objects::ItemContainer;

pub use pz_inventory::visual::human::{HumanStyles, HumanVisualOwner, NoStyles, SkinList};
pub use pz_inventory::visual::{load_rgb, non_blank, save_rgb, HumanVisual, ItemVisual, Rgb};

/// `SurvivorDesc` as saved.
#[derive(Clone, Debug, PartialEq)]
pub struct SurvivorDesc {
    pub id: i32,
    pub forename: String,
    pub surname: String,
    pub torso: String,
    pub female: bool,
    /// The profession's registry location.
    pub profession: String,
    pub extra: Vec<String>,
    /// XP boosts as `(perk id, level)`. Loading drops unknown perks
    /// (`PerkFactory.Perks.FromString` gives `MAX`); with no perk table all
    /// are kept.
    pub xp_boosts: Vec<(String, i32)>,
    pub voice_prefix: String,
    pub voice_pitch: f32,
    pub voice_type: i32,
}

impl SurvivorDesc {
    pub fn load(r: &mut Reader<'_>, wv: i32) -> SaveResult<Self> {
        let id = r.i32()?;
        let forename = r.string()?;
        let surname = r.string()?;
        let torso = r.string()?;
        let female = r.i32()? == 1;
        let profession = r.string()?;
        let mut extra = Vec::new();
        if r.i32()? == 1 {
            let n = r.i32()?;
            for _ in 0..n.max(0) {
                extra.push(r.string()?);
            }
        }
        let n = r.i32()?;
        let mut xp_boosts = Vec::new();
        for _ in 0..n.max(0) {
            let perk = r.string()?;
            let level = r.i32()?;
            xp_boosts.push((perk, level));
        }
        let (voice_prefix, voice_pitch, voice_type) = if wv >= 208 {
            (r.string()?, r.f32()?, r.i32()?)
        } else {
            // The original draws Rand.Next(3) for the voice type here.
            return err("SurvivorDesc from world version < 208 needs Rand");
        };
        Ok(SurvivorDesc {
            id,
            forename,
            surname,
            torso,
            female,
            profession,
            extra,
            xp_boosts,
            voice_prefix,
            voice_pitch,
            voice_type,
        })
    }

    pub fn save(&self, w: &mut Writer) -> SaveResult<()> {
        if self.xp_boosts.len() > 1 {
            // getXPBoostMap() is a HashMap keyed by Perk objects with
            // identity hash codes: its iteration order is not reproducible.
            return err("SurvivorDesc with more than one XP boost");
        }
        w.i32(self.id);
        w.string(&self.forename);
        w.string(&self.surname);
        w.string(&self.torso);
        w.i32(self.female as i32);
        w.string(&self.profession);
        if self.extra.is_empty() {
            w.i32(0);
        } else {
            w.i32(1);
            w.i32(self.extra.len() as i32);
            for e in &self.extra {
                w.string(e);
            }
        }
        w.i32(self.xp_boosts.len() as i32);
        for (p, l) in &self.xp_boosts {
            w.string(p);
            w.i32(*l);
        }
        w.string(&self.voice_prefix);
        w.f32(self.voice_pitch);
        w.i32(self.voice_type);
        Ok(())
    }
}

/// `AnimalAllele`.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimalAllele {
    pub name: String,
    pub current_value: f32,
    pub true_ratio_value: f32,
    pub dominant: bool,
    pub genetic_disorder: String,
}

impl AnimalAllele {
    fn load(r: &mut Reader<'_>) -> SaveResult<Self> {
        Ok(AnimalAllele {
            name: r.string()?,
            current_value: r.f32()?,
            true_ratio_value: r.f32()?,
            dominant: r.bool()?,
            genetic_disorder: r.string()?,
        })
    }

    fn save(&self, w: &mut Writer) {
        w.string(&self.name);
        w.f32(self.current_value);
        w.f32(self.true_ratio_value);
        w.bool(self.dominant);
        w.string(&self.genetic_disorder);
    }
}

/// `AnimalGene`.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimalGene {
    pub id: i32,
    pub name: String,
    pub allele1: AnimalAllele,
    pub allele2: AnimalAllele,
}

/// The animal fields of an animal corpse.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimalCorpse {
    pub animal_type: String,
    pub size: f32,
    pub genome: Vec<AnimalGene>,
    pub disorders: Vec<String>,
    pub custom_name: String,
    pub corpse_item: String,
    pub weight: f32,
    pub inv_icon: String,
    pub shadow: [f32; 3],
}

/// `IsoDeadBody.baseVisual`.
#[derive(Clone, Debug, PartialEq)]
pub enum CorpseVisual {
    Human(Box<HumanVisual>),
    Animal {
        skin_texture_name: String,
        rot_stage: i8,
    },
}

/// One bone of `diedBoneTransforms`: position, rotation, scale.
#[derive(Clone, Debug, PartialEq)]
pub struct BoneTransform {
    pub bone: i32,
    pub values: [f32; 10],
}

/// `IsoMovingObject` fields saved for a static moving object.
#[derive(Clone, Debug, PartialEq)]
pub struct MovingBase {
    /// `offsetX`/`offsetY`: what saving writes. Loading reads and drops the
    /// saved values; these are the object's own (`32 * Core.tileScale`).
    pub offset_x: f32,
    pub offset_y: f32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// `getForwardIsoDirection().ordinal()`.
    pub dir: i32,
    pub mod_data: Option<LuaTable>,
}

/// `Math.max(a, b)` for floats (NaN wins, `-0.0 < 0.0`).
pub(crate) fn java_max_f32(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && a.is_sign_negative() {
        return b;
    }
    if a >= b {
        a
    } else {
        b
    }
}

/// `Math.min(a, b)` for floats.
pub(crate) fn java_min_f32(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && b.is_sign_negative() {
        return b;
    }
    if a <= b {
        a
    } else {
        b
    }
}

impl MovingBase {
    /// `IsoMovingObject.load` (after the factory bytes). `offsets` are the
    /// object's own `offsetX`/`offsetY`; the saved ones are read and
    /// dropped. `IsoWorld.saveoffsetx`/`saveoffsety` are taken as 0.
    pub fn load(r: &mut Reader<'_>, wv: i32, offsets: (f32, f32)) -> SaveResult<Self> {
        let _offset_x = r.f32()?;
        let _offset_y = r.f32()?;
        let x = r.f32()?;
        let y = r.f32()?;
        // setZ: clamped to [-32, 31].
        let z = java_min_f32(31.0, java_max_f32(-32.0, r.f32()?));
        // IsoDirections.fromIndex.
        let dir = r.i32()? & 7;
        let mod_data = if r.bool()? {
            Some(LuaTable::load(r, wv)?)
        } else {
            None
        };
        Ok(MovingBase {
            offset_x: offsets.0,
            offset_y: offsets.1,
            x,
            y,
            z,
            dir,
            mod_data,
        })
    }

    /// `IsoMovingObject.save`, including the serialise and class bytes.
    pub fn save(&self, w: &mut Writer, class_id: i8) {
        self.save_at(w, class_id, self.x, self.y);
    }

    /// [`MovingBase::save`] writing `(x, y)` as the position.
    pub fn save_at(&self, w: &mut Writer, class_id: i8, x: f32, y: f32) {
        w.i8(1);
        w.i8(class_id);
        w.f32(self.offset_x);
        w.f32(self.offset_y);
        w.f32(x);
        w.f32(y);
        w.f32(self.z);
        w.i32(self.dir);
        match self.mod_data.as_ref().filter(|t| !t.is_empty()) {
            Some(t) => {
                w.i8(1);
                t.save(w);
            }
            None => w.i8(0),
        }
    }
}

/// `IsoDeadBody`.
#[derive(Clone, Debug, PartialEq)]
pub struct IsoDeadBody {
    pub base: MovingBase,
    pub female: bool,
    pub was_zombie: bool,
    pub animal: Option<AnimalCorpse>,
    /// `ObjectID` (`ObjectIDShort`): id and type index.
    pub object_id: (i16, i8),
    pub persistent_outfit_id: i32,
    pub desc: Option<SurvivorDesc>,
    pub visual: CorpseVisual,
    pub container: Option<ItemContainer>,
    pub worn_items: Vec<(String, i16)>,
    pub attached_items: Vec<(String, i16)>,
    pub death_time: f32,
    pub reanimate_time: f32,
    pub fall_on_front: bool,
    pub killed_by_fall: bool,
    pub was_skeleton: bool,
    pub angle: f32,
    pub zombie_rot_stage_at_death: i32,
    pub animal_rot_stage_at_death: i32,
    pub rotten_texture: String,
    pub skel_inv_icon: String,
    pub crawling: bool,
    pub fake_dead: bool,
    pub ragdoll: Option<Vec<BoneTransform>>,
}

impl IsoDeadBody {
    /// `isSkeleton()`.
    pub fn is_skeleton(&self) -> bool {
        if self.animal.is_some() {
            let key = LuaKey::Str("skeleton".into());
            return matches!(
                self.base.mod_data.as_ref().and_then(|t| t.rawget(&key)),
                Some(LuaValue::Str(s)) if s.eq_ignore_ascii_case("true")
            );
        }
        self.was_skeleton
    }

    /// `IsoMovingObject.load` + `IsoDeadBody.load` (after the factory
    /// bytes).
    pub fn load(
        r: &mut Reader<'_>,
        wv: i32,
        t: &SaveTables,
        offsets: (f32, f32),
        items: &dyn ItemRegistry,
    ) -> SaveResult<Self> {
        let base = MovingBase::load(r, wv, offsets)?;
        let female = r.bool()?;
        let was_zombie = r.bool()?;
        let animal = if r.bool()? {
            let animal_type = r.string()?;
            let size = r.f32()?;
            let mut genome = Vec::new();
            let mut disorders = Vec::new();
            if wv >= 246 {
                let n = r.i8()? as u8;
                for _ in 0..n {
                    genome.push(AnimalGene {
                        id: r.i32()?,
                        name: r.string()?,
                        allele1: AnimalAllele::load(r)?,
                        allele2: AnimalAllele::load(r)?,
                    });
                }
                let n = r.i8()? as u8;
                for _ in 0..n {
                    disorders.push(r.string()?);
                }
            }
            Some(AnimalCorpse {
                animal_type,
                size,
                genome,
                disorders,
                custom_name: r.string()?,
                corpse_item: r.string()?,
                weight: r.f32()?,
                inv_icon: r.string()?,
                shadow: [r.f32()?, r.f32()?, r.f32()?],
            })
        } else {
            None
        };
        let object_id = if wv >= 199 {
            (r.i16()?, r.i8()?)
        } else {
            return err("IsoDeadBody from world version < 199");
        };
        let _server = r.bool()?;
        let persistent_outfit_id = r.i32()?;
        let desc = if r.bool()? {
            Some(SurvivorDesc::load(r, wv)?)
        } else {
            None
        };
        let visual = match r.i8()? {
            0 => CorpseVisual::Human(Box::new(HumanVisual::load(r, t.blood_body_parts)?)),
            1 => CorpseVisual::Animal {
                skin_texture_name: r.string()?,
                rot_stage: r.i8()?,
            },
            _ => return err("invalid visualType for corpse"),
        };
        let mut container = None;
        let mut worn_items = Vec::new();
        let mut attached_items = Vec::new();
        if r.bool()? {
            let id = r.i32()?;
            let mut c = load_container(r, wv, items)?;
            c.id = id;
            container = Some(c);
            let n = r.i8()?;
            for _ in 0..n.max(0) {
                worn_items.push((r.string()?, r.i16()?));
            }
            let n = r.i8()?;
            for _ in 0..n.max(0) {
                attached_items.push((r.string()?, r.i16()?));
            }
        }
        let death_time = r.f32()?;
        let reanimate_time = r.f32()?;
        let flags = r.i8()?;
        let was_skeleton = r.bool()?;
        let angle = r.f32()?;
        let zombie_rot_stage_at_death = (r.i8()? as u8) as i32;
        let animal_rot_stage_at_death = if wv >= 222 { (r.i8()? as u8) as i32 } else { 0 };
        let (rotten_texture, skel_inv_icon) = if wv >= 225 {
            (r.string()?, r.string()?)
        } else {
            (String::new(), String::new())
        };
        let crawling = r.bool()?;
        let fake_dead = r.bool()?;
        let ragdoll = if r.bool()? {
            let n = r.i32()?;
            let mut bones: Vec<Option<BoneTransform>> = vec![None; n.max(0) as usize];
            for _ in 0..n.max(0) {
                let bone = r.i32()?;
                let mut values = [0f32; 10];
                for v in values.iter_mut() {
                    *v = r.f32()?;
                }
                if bone >= 0 && bone < n {
                    bones[bone as usize] = Some(BoneTransform { bone, values });
                }
            }
            if bones.iter().any(Option::is_none) {
                // Unset bones keep the allocated transform's identity.
                return err("ragdoll bones with gaps");
            }
            Some(bones.into_iter().flatten().collect())
        } else {
            None
        };
        Ok(IsoDeadBody {
            base,
            female,
            was_zombie,
            animal,
            object_id,
            persistent_outfit_id,
            desc,
            visual,
            container,
            worn_items,
            attached_items,
            death_time,
            reanimate_time,
            fall_on_front: flags & 1 != 0,
            killed_by_fall: flags & 2 != 0,
            was_skeleton,
            angle,
            zombie_rot_stage_at_death,
            animal_rot_stage_at_death,
            rotten_texture,
            skel_inv_icon,
            crawling,
            fake_dead,
            ragdoll,
        })
    }

    /// `IsoDeadBody.save` (including the serialise and class bytes).
    pub fn save(&self, w: &mut Writer, class_id: i8, items: &dyn ItemRegistry) -> SaveResult<()> {
        self.base.save(w, class_id);
        w.bool(self.female);
        w.bool(self.was_zombie);
        match &self.animal {
            Some(a) => {
                w.i8(1);
                w.string(&a.animal_type);
                w.f32(a.size);
                w.i8(a.genome.len() as i8);
                for g in &a.genome {
                    w.i32(g.id);
                    w.string(&g.name);
                    g.allele1.save(w);
                    g.allele2.save(w);
                }
                w.i8(a.disorders.len() as i8);
                for d in &a.disorders {
                    w.string(d);
                }
                w.string(&a.custom_name);
                w.string(&a.corpse_item);
                w.f32(a.weight);
                w.string(&a.inv_icon);
                for &s in &a.shadow {
                    w.f32(s);
                }
            }
            None => w.i8(0),
        }
        w.i16(self.object_id.0);
        w.i8(self.object_id.1);
        // GameServer.server || GameClient.client
        w.i8(0);
        w.i32(self.persistent_outfit_id);
        match &self.desc {
            Some(d) => {
                w.i8(1);
                d.save(w)?;
            }
            None => w.i8(0),
        }
        match &self.visual {
            CorpseVisual::Human(v) => {
                w.i8(0);
                v.save(w);
            }
            CorpseVisual::Animal {
                skin_texture_name,
                rot_stage,
            } => {
                w.i8(1);
                w.string(skin_texture_name);
                w.i8(*rot_stage);
            }
        }
        match &self.container {
            Some(c) => {
                w.i8(1);
                w.i32(c.id);
                save_container(w, c, items)?;
                w.i8(self.worn_items.len() as i8);
                for (loc, i) in &self.worn_items {
                    w.string(loc);
                    w.i16(*i);
                }
                w.i8(self.attached_items.len() as i8);
                for (loc, i) in &self.attached_items {
                    w.string(loc);
                    w.i16(*i);
                }
            }
            None => w.i8(0),
        }
        w.f32(self.death_time);
        w.f32(self.reanimate_time);
        w.i8(self.fall_on_front as i8 | (self.killed_by_fall as i8) << 1);
        w.bool(self.is_skeleton());
        w.f32(self.angle);
        w.i8(self.zombie_rot_stage_at_death as i8);
        w.i8(self.animal_rot_stage_at_death as i8);
        w.string(&self.rotten_texture);
        w.string(&self.skel_inv_icon);
        w.bool(self.crawling);
        w.bool(self.fake_dead);
        match &self.ragdoll {
            Some(bones) => {
                w.i8(1);
                w.i32(bones.len() as i32);
                for (i, bone) in bones.iter().enumerate() {
                    w.i32(i as i32);
                    for &v in &bone.values {
                        w.f32(v);
                    }
                }
            }
            None => w.i8(0),
        }
        Ok(())
    }
}

/// `ItemContainer.load` (the type, flags, items and capacity).
pub fn load_container(
    r: &mut Reader<'_>,
    wv: i32,
    items: &dyn ItemRegistry,
) -> SaveResult<ItemContainer> {
    let mut c = ItemContainer::new(None);
    c.load(r, wv, items)?;
    Ok(c)
}

/// `ItemContainer.save`.
pub fn save_container(
    w: &mut Writer,
    c: &ItemContainer,
    items: &dyn ItemRegistry,
) -> SaveResult<()> {
    c.save(w, items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_visual_gains_white_skin() {
        let t = SaveTables::for_tests();
        let mut v = HumanVisual::new(t.blood_body_parts);
        v.skin_color = None;
        v.hair_color = Some([super::super::io::color_from_byte(5), 0.5, 1.0]);
        v.blood = vec![1, 2];
        v.body_visuals.push(ItemVisual {
            full_type: "Base.X".into(),
            alternate_model_name: String::new(),
            clothing_item_name: "X".into(),
            tint: None,
            base_texture: 2,
            texture_choice: -1,
            hue: 0.25,
            decal: Some(" ".into()),
            arrays: [Some(vec![3, 4]), None, None, None, None, None],
        });
        let mut w = Writer::new();
        v.save(&mut w);
        let back = HumanVisual::load(&mut Reader::new(&w.buf), t.blood_body_parts).unwrap();
        assert_eq!(back.skin_color, Some([1.0, 1.0, 1.0]));
        assert_eq!(back.blood.len(), t.blood_body_parts);
        assert_eq!(back.body_visuals[0].decal, None);
        let mut w2 = Writer::new();
        back.save(&mut w2);
        let again = HumanVisual::load(&mut Reader::new(&w2.buf), t.blood_body_parts).unwrap();
        assert_eq!(again, back);
    }
}
