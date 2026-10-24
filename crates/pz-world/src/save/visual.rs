//! `HumanVisual`, `ItemVisual`, `AnimalVisual`, `SurvivorDesc` and
//! `IsoDeadBody` as saved in chunk files.

use super::io::{color_from_byte, color_to_byte, err, Reader, SaveResult, Writer};
use super::items::{load_items, save_items, ItemRegistry};
use super::lua_table::{LuaKey, LuaTable, LuaValue};
use super::SaveTables;
use crate::objects::ItemContainer;

/// An `ImmutableColor` built from bytes (components are `byte / 255f`).
pub type Rgb = [f32; 3];

fn load_rgb(r: &mut Reader<'_>) -> SaveResult<Rgb> {
    Ok([
        color_from_byte(r.i8()?),
        color_from_byte(r.i8()?),
        color_from_byte(r.i8()?),
    ])
}

fn save_rgb(w: &mut Writer, c: &Rgb) {
    for &v in c {
        w.i8(color_to_byte(v));
    }
}

/// `HumanVisual`.
#[derive(Clone, Debug, PartialEq)]
pub struct HumanVisual {
    pub hair_color: Option<Rgb>,
    pub beard_color: Option<Rgb>,
    /// `clear()` sets `ImmutableColor.white`, so a loaded visual always has
    /// one.
    pub skin_color: Option<Rgb>,
    pub body_hair: i8,
    pub skin_texture: i8,
    pub zombie_rot_stage: i8,
    pub skin_texture_name: Option<String>,
    pub beard_model: Option<String>,
    pub hair_model: Option<String>,
    /// `BloodBodyPartType.MAX.index()` entries each.
    pub blood: Vec<i8>,
    pub dirt: Vec<i8>,
    pub holes: Vec<i8>,
    pub body_visuals: Vec<ItemVisual>,
    pub non_attached_hair: Option<String>,
    pub natural_hair_color: Option<Rgb>,
    pub natural_beard_color: Option<Rgb>,
}

impl HumanVisual {
    /// A visual after `clear()`.
    pub fn new(parts: usize) -> Self {
        HumanVisual {
            hair_color: None,
            beard_color: None,
            skin_color: Some([1.0, 1.0, 1.0]),
            body_hair: -1,
            skin_texture: -1,
            zombie_rot_stage: -1,
            skin_texture_name: None,
            beard_model: None,
            hair_model: None,
            blood: vec![0; parts],
            dirt: vec![0; parts],
            holes: vec![0; parts],
            body_visuals: Vec::new(),
            non_attached_hair: None,
            natural_hair_color: None,
            natural_beard_color: None,
        }
    }

    pub fn load(r: &mut Reader<'_>, t: &SaveTables) -> SaveResult<Self> {
        let mut v = HumanVisual::new(t.blood_body_parts);
        let f1 = r.i8()? as u8;
        if f1 & 4 != 0 {
            v.hair_color = Some(load_rgb(r)?);
        }
        if f1 & 2 != 0 {
            v.beard_color = Some(load_rgb(r)?);
        }
        if f1 & 8 != 0 {
            v.skin_color = Some(load_rgb(r)?);
        }
        v.body_hair = r.i8()?;
        v.skin_texture = r.i8()?;
        v.zombie_rot_stage = r.i8()?;
        if f1 & 0x40 != 0 {
            v.skin_texture_name = Some(r.string()?);
        }
        if f1 & 0x10 != 0 {
            v.beard_model = Some(r.string()?);
        }
        if f1 & 0x20 != 0 {
            v.hair_model = Some(r.string()?);
        }
        for arr in [&mut v.blood, &mut v.dirt, &mut v.holes] {
            let n = r.i8()?;
            for i in 0..n.max(0) as usize {
                let a = r.i8()?;
                if i < arr.len() {
                    arr[i] = a;
                }
            }
        }
        let n = r.i8()?;
        for _ in 0..n.max(0) {
            v.body_visuals.push(ItemVisual::load(r, t)?);
        }
        v.non_attached_hair = non_blank(r.string()?);
        let f2 = r.i8()? as u8;
        if f2 & 4 != 0 {
            v.natural_hair_color = Some(load_rgb(r)?);
        }
        if f2 & 2 != 0 {
            v.natural_beard_color = Some(load_rgb(r)?);
        }
        Ok(v)
    }

    pub fn save(&self, w: &mut Writer) {
        let mut f1: u8 = 0;
        if self.hair_color.is_some() {
            f1 |= 4;
        }
        if self.beard_color.is_some() {
            f1 |= 2;
        }
        if self.skin_color.is_some() {
            f1 |= 8;
        }
        if self.beard_model.is_some() {
            f1 |= 0x10;
        }
        if self.hair_model.is_some() {
            f1 |= 0x20;
        }
        if self.skin_texture_name.is_some() {
            f1 |= 0x40;
        }
        w.i8(f1 as i8);
        for c in [&self.hair_color, &self.beard_color, &self.skin_color]
            .into_iter()
            .flatten()
        {
            save_rgb(w, c);
        }
        w.i8(self.body_hair);
        w.i8(self.skin_texture);
        w.i8(self.zombie_rot_stage);
        for s in [&self.skin_texture_name, &self.beard_model, &self.hair_model]
            .into_iter()
            .flatten()
        {
            w.string(s);
        }
        for arr in [&self.blood, &self.dirt, &self.holes] {
            w.i8(arr.len() as i8);
            for &a in arr {
                w.i8(a);
            }
        }
        w.i8(self.body_visuals.len() as i8);
        for v in &self.body_visuals {
            v.save(w);
        }
        w.opt_string(self.non_attached_hair.as_deref());
        let mut f2: u8 = 0;
        if self.natural_hair_color.is_some() {
            f2 |= 4;
        }
        if self.natural_beard_color.is_some() {
            f2 |= 2;
        }
        w.i8(f2 as i8);
        for c in [&self.natural_hair_color, &self.natural_beard_color]
            .into_iter()
            .flatten()
        {
            save_rgb(w, c);
        }
    }
}

/// `StringUtils.isNullOrWhitespace` folded to `None`.
fn non_blank(s: String) -> Option<String> {
    if s.trim().is_empty() {
        None
    } else {
        Some(s)
    }
}

/// `ItemVisual`.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemVisual {
    pub full_type: String,
    pub alternate_model_name: String,
    pub clothing_item_name: String,
    pub tint: Option<Rgb>,
    pub base_texture: i8,
    pub texture_choice: i8,
    pub hue: f32,
    pub decal: Option<String>,
    /// `blood`, `dirt`, `holes`, `basicPatches`, `denimPatches`,
    /// `leatherPatches`: `None` until a non-empty array is read.
    pub arrays: [Option<Vec<i8>>; 6],
}

impl ItemVisual {
    pub fn load(r: &mut Reader<'_>, t: &SaveTables) -> SaveResult<Self> {
        let f1 = r.i8()? as u8;
        let mut v = ItemVisual {
            full_type: r.string()?,
            alternate_model_name: r.string()?,
            clothing_item_name: r.string()?,
            tint: None,
            base_texture: -1,
            texture_choice: -1,
            hue: f32::INFINITY,
            decal: None,
            arrays: Default::default(),
        };
        if f1 & 1 != 0 {
            v.tint = Some(load_rgb(r)?);
        }
        if f1 & 2 != 0 {
            v.base_texture = r.i8()?;
        }
        if f1 & 4 != 0 {
            v.texture_choice = r.i8()?;
        }
        if f1 & 8 != 0 {
            v.hue = r.f32()?;
        }
        if f1 & 0x10 != 0 {
            v.decal = Some(r.string()?);
        }
        for arr in v.arrays.iter_mut() {
            let n = r.i8()?;
            if n > 0 && arr.is_none() {
                *arr = Some(vec![0; t.blood_body_parts]);
            }
            for i in 0..n.max(0) as usize {
                let a = r.i8()?;
                if let Some(arr) = arr {
                    if i < arr.len() {
                        arr[i] = a;
                    }
                }
            }
        }
        Ok(v)
    }

    pub fn save(&self, w: &mut Writer) {
        let decal = self.decal.as_deref().filter(|d| !d.trim().is_empty());
        let mut f1: u8 = 0;
        if self.tint.is_some() {
            f1 |= 1;
        }
        if self.base_texture != -1 {
            f1 |= 2;
        }
        if self.texture_choice != -1 {
            f1 |= 4;
        }
        if self.hue != f32::INFINITY {
            f1 |= 8;
        }
        if decal.is_some() {
            f1 |= 0x10;
        }
        w.i8(f1 as i8);
        w.string(&self.full_type);
        w.string(&self.alternate_model_name);
        w.string(&self.clothing_item_name);
        if let Some(c) = &self.tint {
            save_rgb(w, c);
        }
        if self.base_texture != -1 {
            w.i8(self.base_texture);
        }
        if self.texture_choice != -1 {
            w.i8(self.texture_choice);
        }
        if self.hue != f32::INFINITY {
            w.f32(self.hue);
        }
        if let Some(d) = decal {
            w.string(d);
        }
        for arr in &self.arrays {
            match arr {
                Some(a) => {
                    w.i8(a.len() as i8);
                    for &x in a {
                        w.i8(x);
                    }
                }
                None => w.i8(0),
            }
        }
    }
}

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
        let _offset_x = r.f32()?;
        let _offset_y = r.f32()?;
        let x = r.f32()?;
        let y = r.f32()?;
        let z = r.f32()?;
        let dir = r.i32()? & 7;
        let mod_data = if r.bool()? {
            Some(LuaTable::load(r, wv)?)
        } else {
            None
        };
        let base = MovingBase {
            offset_x: offsets.0,
            offset_y: offsets.1,
            x,
            y,
            z,
            dir,
            mod_data,
        };
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
            0 => CorpseVisual::Human(Box::new(HumanVisual::load(r, t)?)),
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
            let mut c = load_container(r, items)?;
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
    pub fn save(&self, w: &mut Writer, class_id: i8) -> SaveResult<()> {
        let b = &self.base;
        w.i8(1);
        w.i8(class_id);
        w.f32(b.offset_x);
        w.f32(b.offset_y);
        w.f32(b.x);
        w.f32(b.y);
        w.f32(b.z);
        w.i32(b.dir);
        match b.mod_data.as_ref().filter(|t| !t.is_empty()) {
            Some(t) => {
                w.i8(1);
                t.save(w);
            }
            None => w.i8(0),
        }
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
                save_container(w, c);
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
pub fn load_container(r: &mut Reader<'_>, items: &dyn ItemRegistry) -> SaveResult<ItemContainer> {
    let kind = r.string()?;
    let mut c = ItemContainer::new(Some(&kind));
    c.explored = r.bool()?;
    c.items = load_items(r, items)?;
    c.has_been_looted = r.bool()?;
    c.capacity = r.i32()?;
    Ok(c)
}

/// `ItemContainer.save`.
pub fn save_container(w: &mut Writer, c: &ItemContainer) {
    w.opt_string(c.kind.as_deref());
    w.bool(c.explored);
    save_items(w, &c.items);
    w.bool(c.has_been_looted);
    w.i32(c.capacity);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_visual_gains_white_skin() {
        let t = SaveTables::for_tests();
        let mut v = HumanVisual::new(t.blood_body_parts);
        v.skin_color = None;
        v.hair_color = Some([color_from_byte(5), 0.5, 1.0]);
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
        let back = HumanVisual::load(&mut Reader::new(&w.buf), &t).unwrap();
        assert_eq!(back.skin_color, Some([1.0, 1.0, 1.0]));
        assert_eq!(back.blood.len(), t.blood_body_parts);
        assert_eq!(back.body_visuals[0].decal, None);
        let mut w2 = Writer::new();
        back.save(&mut w2);
        let again = HumanVisual::load(&mut Reader::new(&w2.buf), &t).unwrap();
        assert_eq!(again, back);
    }
}
