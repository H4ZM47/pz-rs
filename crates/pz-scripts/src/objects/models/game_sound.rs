//! `zombie.scripting.objects.GameSoundScript` and the data it fills,
//! `zombie.audio.GameSound` / `GameSoundClip`.

use crate::base::{index_error, BaseScriptObject, ScriptError, ScriptObject, ScriptResult};
use crate::java;
use crate::manager::ScriptManager;
use crate::parser::Block;
use crate::script_type::ScriptType;

use super::{parse_float, parse_int, split_key_value, try_parse_int};

/// `GameSound.MasterVolume`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MasterVolume {
    #[default]
    Primary,
    Ambient,
    Music,
    VehicleEngine,
}

impl MasterVolume {
    /// `MasterVolume.valueOf`.
    pub fn value_of(s: &str) -> ScriptResult<Self> {
        Ok(match s {
            "Primary" => MasterVolume::Primary,
            "Ambient" => MasterVolume::Ambient,
            "Music" => MasterVolume::Music,
            "VehicleEngine" => MasterVolume::VehicleEngine,
            _ => {
                return Err(ScriptError::new(format!(
                    "java.lang.IllegalArgumentException: No enum constant zombie.audio.GameSound.MasterVolume.{s}"
                )))
            }
        })
    }

    /// `name()`.
    pub fn name(self) -> &'static str {
        match self {
            MasterVolume::Primary => "Primary",
            MasterVolume::Ambient => "Ambient",
            MasterVolume::Music => "Music",
            MasterVolume::VehicleEngine => "VehicleEngine",
        }
    }
}

pub const INIT_FLAG_DISTANCE_MIN: i16 = 1;
pub const INIT_FLAG_DISTANCE_MAX: i16 = 2;
pub const INIT_FLAG_STOP_IMMEDIATE: i16 = 4;

/// `zombie.audio.GameSoundClip` (script data only).
#[derive(Clone, Debug, PartialEq)]
pub struct GameSoundClip {
    pub event: Option<String>,
    // TODO(audio): `eventDescription` / `eventDescriptionMp` (FMOD).
    pub file: Option<String>,
    pub volume: f32,
    pub pitch: f32,
    pub distance_min: f32,
    pub distance_max: f32,
    pub reverb_max_range: f32,
    pub reverb_factor: f32,
    pub priority: i32,
    pub init_flags: i16,
    pub reload_epoch: i16,
}

impl GameSoundClip {
    /// `new GameSoundClip(gameSound)`.
    pub fn new(game_sound: &GameSound) -> Self {
        GameSoundClip {
            event: None,
            file: None,
            volume: 1.0,
            pitch: 1.0,
            distance_min: 10.0,
            distance_max: 10.0,
            reverb_max_range: 10.0,
            reverb_factor: 0.0,
            priority: 5,
            init_flags: 0,
            reload_epoch: game_sound.reload_epoch,
        }
    }

    pub fn has_min_distance(&self) -> bool {
        self.init_flags & INIT_FLAG_DISTANCE_MIN != 0
    }

    pub fn has_max_distance(&self) -> bool {
        self.init_flags & INIT_FLAG_DISTANCE_MAX != 0
    }

    pub fn is_stop_immediate(&self) -> bool {
        self.init_flags & INIT_FLAG_STOP_IMMEDIATE != 0
    }
}

/// `zombie.audio.GameSound` (script data only).
#[derive(Clone, Debug, PartialEq)]
pub struct GameSound {
    pub name: Option<String>,
    pub category: String,
    pub loop_: bool,
    pub is3d: bool,
    pub clips: Vec<GameSoundClip>,
    pub user_volume: f32,
    pub master: MasterVolume,
    pub max_instances_per_emitter: i32,
    pub reload_epoch: i16,
}

impl Default for GameSound {
    fn default() -> Self {
        GameSound {
            name: None,
            category: "General".to_string(),
            loop_: false,
            is3d: true,
            clips: Vec::new(),
            user_volume: 1.0,
            master: MasterVolume::Primary,
            max_instances_per_emitter: -1,
            reload_epoch: 0,
        }
    }
}

impl GameSound {
    /// `getMaxDistanceOfClips`.
    pub fn max_distance_of_clips(&self) -> f32 {
        self.clips
            .iter()
            .filter(|c| c.has_max_distance())
            .fold(0.0, |m, c| super::pz_max(m, c.distance_max))
    }

    /// `reset`: bumps `reloadEpoch` so clips held elsewhere notice.
    pub fn reset(&mut self) {
        let epoch = self.reload_epoch.wrapping_add(1);
        *self = GameSound { reload_epoch: epoch, ..GameSound::default() };
    }
}

#[derive(Debug)]
pub struct GameSoundScript {
    pub base: BaseScriptObject,
    pub game_sound: GameSound,
}

impl Default for GameSoundScript {
    fn default() -> Self {
        Self::new()
    }
}

impl GameSoundScript {
    pub fn new() -> Self {
        GameSoundScript { base: BaseScriptObject::new(ScriptType::Sound), game_sound: GameSound::default() }
    }

    /// `LoadClip`.
    fn load_clip(&self, block: &Block) -> ScriptResult<GameSoundClip> {
        let mut clip = GameSoundClip::new(&self.game_sound);
        for value in &block.values {
            let (k, v) = split_key_value(&value.string)?;
            match k {
                "distanceMax" => {
                    clip.distance_max = parse_int(v)? as f32;
                    clip.init_flags |= INIT_FLAG_DISTANCE_MAX;
                }
                "distanceMin" => {
                    clip.distance_min = parse_int(v)? as f32;
                    clip.init_flags |= INIT_FLAG_DISTANCE_MIN;
                }
                "event" => clip.event = Some(v.to_string()),
                "file" => clip.file = Some(v.to_string()),
                "pitch" => clip.pitch = parse_float(v)?,
                "volume" => clip.volume = parse_float(v)?,
                "reverbFactor" => clip.reverb_factor = parse_float(v)?,
                "reverbMaxRange" => clip.reverb_max_range = parse_float(v)?,
                "stopImmediate" => clip.init_flags |= INIT_FLAG_STOP_IMMEDIATE,
                _ => {}
            }
        }
        Ok(clip)
    }
}

impl ScriptObject for GameSoundScript {
    fn base(&self) -> &BaseScriptObject {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseScriptObject {
        &mut self.base
    }

    fn class_name(&self) -> &'static str {
        "zombie.scripting.objects.GameSoundScript"
    }

    fn load(&mut self, sm: &ScriptManager, name: &str, body: &str) -> ScriptResult {
        self.game_sound.name = Some(name.to_string());
        let root = sm.parse(body);
        let block = root.children.first().ok_or_else(index_error)?;
        self.base.load_common_block(block);
        for value in &block.values {
            let (k, v) = split_key_value(&value.string)?;
            let gs = &mut self.game_sound;
            match k {
                "category" => gs.category = v.to_string(),
                "is3D" => gs.is3d = java::parse_bool(v),
                "loop" => gs.loop_ = java::parse_bool(v),
                "master" => gs.master = MasterVolume::value_of(v)?,
                "maxInstancesPerEmitter" => gs.max_instances_per_emitter = try_parse_int(v, -1),
                _ => {}
            }
        }
        for child in &block.children {
            if child.type_.as_deref() == Some("clip") {
                let clip = self.load_clip(child)?;
                self.game_sound.clips.push(clip);
            }
        }
        Ok(())
    }

    fn reset(&mut self) {
        self.game_sound.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_util::load;
    use super::*;

    #[test]
    fn loads_sound_and_clips() {
        let body = "sound Ping { category = UI, is3D = false, master = Ambient, maxInstancesPerEmitter = 2, \
                    clip { event = ui/ping, distanceMax = 20, volume = 0.5, stopImmediate = true, } \
                    clip { file = a.ogg, } }";
        let (s, r) = load(GameSoundScript::new(), "Ping", body);
        r.unwrap();
        let gs = &s.game_sound;
        assert_eq!(gs.name.as_deref(), Some("Ping"));
        assert_eq!(gs.category, "UI");
        assert!(!gs.is3d);
        assert_eq!(gs.master, MasterVolume::Ambient);
        assert_eq!(gs.max_instances_per_emitter, 2);
        assert_eq!(gs.clips.len(), 2);
        assert_eq!(gs.clips[0].distance_max, 20.0);
        assert!(gs.clips[0].has_max_distance() && gs.clips[0].is_stop_immediate());
        assert_eq!(gs.max_distance_of_clips(), 20.0);
    }

    #[test]
    fn errors_follow_java() {
        let (_, r) = load(GameSoundScript::new(), "P", "sound P { master = Loud, }");
        assert!(r.unwrap_err().0.ends_with("No enum constant zombie.audio.GameSound.MasterVolume.Loud"));
        let (_, r) = load(GameSoundScript::new(), "P", "sound P { clip { distanceMin = 1.5, } }");
        assert!(r.unwrap_err().0.contains("For input string: \"1.5\""));
    }

    #[test]
    fn reset_bumps_epoch() {
        let mut s = GameSoundScript::new();
        s.game_sound.category = "X".into();
        s.reset();
        assert_eq!(s.game_sound.category, "General");
        assert_eq!(s.game_sound.reload_epoch, 1);
    }
}
