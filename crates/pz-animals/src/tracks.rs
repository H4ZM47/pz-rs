//! `AnimalTracksDefinitions` (and its inner `AnimalTracksType`), built from
//! the Lua global `AnimalTracksDefinitions.animallist`
//! (`media/lua/shared/Definitions/animal/AnimalTracksDefinitions.lua`).

use crate::java::{self, as_table, entries, lower, JHashMap, JResult, JavaException};
use pz_kahlua::{TableRef, Value};
use pz_world::{GameRand, IsoDirections};

/// `AnimalTracksDefinitions.AnimalTracksType`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AnimalTracksType {
    pub type_: Option<String>,
    pub name: Option<String>,
    pub need_dir: bool,
    pub sprite: Option<String>,
    pub action_type: Option<String>,
    pub chance_to_find_track: i32,
    pub min_skill: i32,
    pub chance_to_spawn: i32,
    pub item: Option<String>,
    /// `HashMap<IsoDirections, String>`: keyed by an enum whose hash is the
    /// identity hash, so the original's iteration order is not
    /// reproducible; kept in first-insertion order.
    pub sprites: Option<Vec<(IsoDirections, String)>>,
}

impl AnimalTracksType {
    /// The sprite for a direction (`sprites.get(dir)`).
    pub fn sprite_for(&self, dir: IsoDirections) -> Option<&str> {
        self.sprites
            .as_ref()?
            .iter()
            .find(|(d, _)| *d == dir)
            .map(|(_, s)| s.as_str())
    }
}

/// `IsoDirections.fromString(s)`: `valueOf(s.trim().toUpperCase())`.
pub fn iso_direction_from_string(s: &str) -> JResult<IsoDirections> {
    let u = java::java_trim(s).to_uppercase();
    IsoDirections::ALL
        .iter()
        .copied()
        .find(|d| format!("{d:?}") == u)
        .ok_or_else(|| {
            JavaException(format!(
                "IllegalArgumentException: No enum constant zombie.iso.IsoDirections.{u}"
            ))
        })
}

/// One animal (migration group) of `AnimalTracksDefinitions`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AnimalTracksDefinitions {
    pub type_: Option<String>,
    pub tracks: Option<JHashMap<AnimalTracksType>>,
    pub track_chance: Option<JHashMap<i32>>,
    pub skill_to_identify: i32,
    pub track_type: Option<String>,
    pub chance_to_find_track: i32,
}

impl AnimalTracksDefinitions {
    /// `loadTracksDefinitions()` over a Lua environment:
    /// `tracksDefinitions` in its iteration order.
    pub fn load_all(env: &TableRef) -> JResult<JHashMap<AnimalTracksDefinitions>> {
        let mut out = JHashMap::new();
        let Some(definitions) = env.get_str("AnimalTracksDefinitions").as_table().cloned() else {
            return Ok(out);
        };
        let list = as_table(&definitions.rawget(&Value::str("animallist")))?;
        for (k, v) in entries(&list) {
            let type_ = java::to_string(&k);
            // The original puts the definition before filling it, so a
            // failure leaves a partly filled entry; the error aborts here.
            let mut def = AnimalTracksDefinitions {
                type_: Some(type_.clone()),
                ..Default::default()
            };
            for (k2, v2) in entries(&as_table(&v)?) {
                let key = lower(&java::to_string(&k2));
                let s = java::to_string_trim(&v2);
                match key.as_str() {
                    "skilltoidentify" => def.skill_to_identify = java::float_int(&s)?,
                    "chancetofindtrack" => def.chance_to_find_track = java::float_int(&s)?,
                    "tracktype" => def.track_type = Some(s),
                    "tracks" => def.load_tracks(&as_table(&v2)?)?,
                    "trackchance" => {
                        let mut m = JHashMap::new();
                        for (ck, cv) in entries(&as_table(&v2)?) {
                            let key = java::java_trim(&java::to_string(&ck)).to_string();
                            m.insert(key, java::float_int(&java::to_string_trim(&cv))?);
                        }
                        def.track_chance = Some(m);
                    }
                    _ => {}
                }
            }
            out.insert(type_, def);
        }
        Ok(out)
    }

    fn load_tracks(&mut self, table: &TableRef) -> JResult<()> {
        let mut tracks = JHashMap::new();
        for (k, v) in entries(table) {
            let type_ = java::to_string(&k);
            let mut t = AnimalTracksType {
                type_: Some(type_.clone()),
                ..Default::default()
            };
            for (k2, v2) in entries(&as_table(&v)?) {
                let key = lower(&java::to_string(&k2));
                let s = java::to_string_trim(&v2);
                match key.as_str() {
                    "name" => t.name = Some(s),
                    "sprite" => t.sprite = Some(s),
                    "actiontype" => t.action_type = Some(s),
                    "chancetofindtrack" => t.chance_to_find_track = java::float_int(&s)?,
                    "chancetospawn" => t.chance_to_spawn = java::float_int(&s)?,
                    "minskill" => t.min_skill = java::float_int(&s)?,
                    "needdir" => t.need_dir = java::parse_bool(&s),
                    "item" => t.item = Some(s),
                    "sprites" => {
                        let mut sprites: Vec<(IsoDirections, String)> = Vec::new();
                        for (sk, sv) in entries(&as_table(&v2)?) {
                            let dir = iso_direction_from_string(&java::to_string(&sk))?;
                            let val = java::to_string_trim(&sv);
                            match sprites.iter_mut().find(|(d, _)| *d == dir) {
                                Some(e) => e.1 = val,
                                None => sprites.push((dir, val)),
                            }
                        }
                        t.sprites = Some(sprites);
                    }
                    _ => {}
                }
            }
            tracks.insert(type_, t);
        }
        self.tracks = Some(tracks);
        Ok(())
    }

    /// `getTrackType(animal, type)` on this definition.
    pub fn track_type(&self, type_: &str) -> Option<&AnimalTracksType> {
        self.tracks.as_ref()?.get(type_)
    }

    /// `getRandomTrack(animalType, action)` with `this` the animal's
    /// definition and `sandbox_track_chance` the sandbox option
    /// `AnimalTrackChance` (1 = none).
    pub fn random_track(
        &self,
        action: &str,
        sandbox_track_chance: i32,
        rand: &mut dyn GameRand,
    ) -> JResult<Option<&AnimalTracksType>> {
        if sandbox_track_chance == 1 {
            return Ok(None);
        }
        let chance = *self
            .track_chance
            .as_ref()
            .and_then(|m| m.get(action))
            .ok_or_else(|| JavaException("NullPointerException: trackChance".into()))?;
        let chance_mod = match sandbox_track_chance {
            2 => 500,
            3 => 300,
            4 => 150,
            5 => 50,
            _ => 15,
        };
        if rand.next(chance.wrapping_mul(chance_mod)) != 0 {
            return Ok(None);
        }
        let tracks = self
            .tracks
            .as_ref()
            .ok_or_else(|| JavaException("NullPointerException: tracks".into()))?;
        let mut total = 0;
        let mut list = Vec::new();
        for (_, t) in tracks.iter() {
            let at = t
                .action_type
                .as_deref()
                .ok_or_else(|| JavaException("NullPointerException: actionType".into()))?;
            if !java::eq_ignore_case(at, action) {
                continue;
            }
            list.push(t);
            total += t.chance_to_spawn;
        }
        let r = rand.next(total);
        let mut index = 0;
        let mut chosen = None;
        for t in list {
            chosen = Some(t);
            if t.chance_to_spawn + index >= r {
                break;
            }
            index += t.chance_to_spawn;
            chosen = None;
        }
        Ok(chosen)
    }
}
