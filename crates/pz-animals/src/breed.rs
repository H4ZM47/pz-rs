//! `zombie.characters.animals.datas.AnimalBreed` and `AnimalGrowStage`.

use crate::java::{self, as_table, entries, lower, rawget_int, JHashMap, JResult};
use pz_kahlua::{TableRef, Value};

/// `AnimalBreed.ForcedGenes`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ForcedGenes {
    pub name: Option<String>,
    pub min_value: f32,
    pub max_value: f32,
}

/// `AnimalBreed.Sound`.
#[derive(Clone, Debug, PartialEq)]
pub struct BreedSound {
    pub id: Option<String>,
    pub sound_name: Option<String>,
    pub interval_min: i32,
    pub interval_max: i32,
    pub slot: Option<String>,
    pub priority: i32,
}

impl Default for BreedSound {
    fn default() -> Self {
        BreedSound {
            id: None,
            sound_name: None,
            interval_min: -1,
            interval_max: -1,
            slot: None,
            priority: 0,
        }
    }
}

impl BreedSound {
    /// `isIntervalValid()`.
    pub fn is_interval_valid(&self) -> bool {
        self.interval_min >= 0 && self.interval_min <= self.interval_max
    }
}

/// `AnimalBreed`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AnimalBreed {
    pub name: Option<String>,
    pub texture: Option<Vec<String>>,
    pub texture_male: Option<String>,
    pub texture_baby: Option<String>,
    pub min_weight_bonus: i32,
    pub max_weight_bonus: i32,
    pub milk_type: Option<String>,
    pub wool_type: Option<String>,
    pub forced_genes: Option<JHashMap<ForcedGenes>>,
    pub inv_icon_male: Option<String>,
    pub inv_icon_female: Option<String>,
    pub inv_icon_baby: Option<String>,
    pub inv_icon_male_dead: Option<String>,
    pub inv_icon_female_dead: Option<String>,
    pub inv_icon_baby_dead: Option<String>,
    pub inv_icon_male_skel: Option<String>,
    pub inv_icon_female_skel: Option<String>,
    pub inv_icon_baby_skel: Option<String>,
    pub leather: Option<String>,
    pub head_item: Option<String>,
    pub feather_item: Option<String>,
    pub max_feather: i32,
    /// Private in the original (`getSound`).
    pub sounds: JHashMap<BreedSound>,
    pub rotten_texture: Option<String>,
}

impl AnimalBreed {
    /// One entry of a definition's `breeds` table
    /// (`AnimalDefinitions.loadBreeds`).
    pub(crate) fn load(name: String, table: &TableRef) -> JResult<AnimalBreed> {
        let mut b = AnimalBreed {
            name: Some(name),
            ..Default::default()
        };
        for (k, v) in entries(table) {
            let key = lower(&java::to_string(&k));
            let s = java::to_string(&v);
            match key.as_str() {
                "texture" => b.texture = Some(java::split_comma(&s)),
                "texturemale" => b.texture_male = Some(s),
                "texturebaby" => b.texture_baby = Some(s),
                "milktype" => b.milk_type = Some(s),
                "wooltype" => b.wool_type = Some(s),
                "forcedgenes" => b.load_forced_genes(&as_table(&v)?)?,
                "inviconmale" => b.inv_icon_male = Some(s),
                "inviconfemale" => b.inv_icon_female = Some(s),
                "inviconbaby" => b.inv_icon_baby = Some(s),
                "inviconmaledead" => b.inv_icon_male_dead = Some(s),
                "inviconfemaledead" => b.inv_icon_female_dead = Some(s),
                "inviconbabydead" => b.inv_icon_baby_dead = Some(s),
                "inviconmaleskel" => b.inv_icon_male_skel = Some(s),
                "inviconfemaleskel" => b.inv_icon_female_skel = Some(s),
                "inviconbabyskel" => b.inv_icon_baby_skel = Some(s),
                "leather" => b.leather = Some(s),
                "headitem" => b.head_item = Some(s),
                "featheritem" => b.feather_item = Some(s),
                "maxfeather" => b.max_feather = java::float_int(&s)?,
                "sounds" => b.load_sounds(&as_table(&v)?)?,
                "rottentexture" => b.rotten_texture = Some(s),
                _ => {}
            }
        }
        Ok(b)
    }

    /// `loadForcedGenes(def)`.
    pub fn load_forced_genes(&mut self, def: &TableRef) -> JResult<()> {
        let mut m = JHashMap::new();
        for (k, v) in entries(def) {
            let name = lower(&java::to_string(&k));
            let mut g = ForcedGenes {
                name: Some(name.clone()),
                ..Default::default()
            };
            for (k2, v2) in entries(&as_table(&v)?) {
                let key = lower(&java::to_string(&k2));
                let s = java::to_string(&v2);
                match key.as_str() {
                    "minvalue" => g.min_value = java::parse_float(&s)?,
                    "maxvalue" => g.max_value = java::parse_float(&s)?,
                    _ => {}
                }
            }
            m.insert(name, g);
        }
        self.forced_genes = Some(m);
        Ok(())
    }

    /// `loadSounds(soundsTable)`.
    pub fn load_sounds(&mut self, sounds: &TableRef) -> JResult<()> {
        self.sounds = JHashMap::new();
        for (k, v) in entries(sounds) {
            let Value::Table(t) = &v else { continue };
            let id = lower(&java::to_string(&k));
            let slot = java::rawget_str(t, "slot")?;
            let sound = BreedSound {
                id: Some(id.clone()),
                sound_name: java::rawget_str(t, "name")?,
                interval_min: rawget_int(t, "intervalMin"),
                interval_max: rawget_int(t, "intervalMax"),
                slot: slot.filter(|s| !s.chars().all(char::is_whitespace)),
                priority: rawget_int(t, "priority").max(0),
            };
            self.sounds.insert(id, sound);
        }
        Ok(())
    }

    /// `getSound(id)`.
    pub fn sound(&self, id: &str) -> Option<&BreedSound> {
        if id.chars().all(char::is_whitespace) {
            return None;
        }
        self.sounds.get(&lower(id))
    }

    /// `isSoundDefined(id)`.
    pub fn is_sound_defined(&self, id: &str) -> bool {
        self.sound(id).is_some()
    }
}

/// `AnimalGrowStage`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AnimalGrowStage {
    pub age_to_grow: i32,
    pub next_stage: Option<String>,
    pub next_stage_male: Option<String>,
    pub stage: Option<String>,
}

impl AnimalGrowStage {
    /// One entry of a definition's `stages` table
    /// (`AnimalDefinitions.loadStages`).
    pub(crate) fn load(stage: String, table: &TableRef) -> JResult<AnimalGrowStage> {
        let mut s = AnimalGrowStage {
            stage: Some(stage),
            ..Default::default()
        };
        for (k, v) in entries(table) {
            let key = lower(&java::to_string(&k));
            let vs = java::to_string(&v);
            match key.as_str() {
                "agetogrow" => s.age_to_grow = java::float_int(&vs)?,
                "nextstage" => s.next_stage = Some(vs),
                "nextstagemale" => s.next_stage_male = Some(vs),
                _ => {}
            }
        }
        Ok(s)
    }

    /// `getAgeToGrow(animal)`, given the current value of the animal's used
    /// `ageToGrow` allele (`None` when it has none).
    pub fn age_to_grow_for(&self, age_to_grow_gene: Option<f32>) -> i32 {
        let a = age_to_grow_gene.unwrap_or(1.0);
        let modifier = 0.25f32 - a / 4.0 + 1.0;
        (self.age_to_grow as f32 * modifier) as i32
    }
}
