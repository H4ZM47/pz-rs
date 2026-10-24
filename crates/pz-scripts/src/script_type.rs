//! `zombie.scripting.ScriptType`, `ScriptType.Flags` and `ScriptLoadMode`.

/// `ScriptLoadMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScriptLoadMode {
    Init,
    Reload,
}

/// `ScriptType.Flags`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum Flag {
    Clear = 1 << 0,
    FromList = 1 << 1,
    CacheFullType = 1 << 2,
    ResetExisting = 1 << 3,
    RemoveLoadError = 1 << 4,
    SeekImports = 1 << 5,
    ResetOnceOnReload = 1 << 6,
    AllowNewScriptDiscoveryOnReload = 1 << 7,
    NewInstanceOnReload = 1 << 8,
}

macro_rules! script_types {
    ($($name:ident = ($template:expr, $tag:expr)),* $(,)?) => {
        /// `ScriptType`, in original ordinal order.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum ScriptType {
            $($name),*
        }

        impl ScriptType {
            pub const ALL: &'static [ScriptType] = &[$(ScriptType::$name),*];

            /// Enum constant name (`toString()`).
            pub fn name(self) -> &'static str {
                match self {
                    $(ScriptType::$name => stringify!($name)),*
                }
            }

            pub fn is_template(self) -> bool {
                match self {
                    $(ScriptType::$name => $template),*
                }
            }

            pub fn script_tag(self) -> &'static str {
                match self {
                    $(ScriptType::$name => $tag),*
                }
            }
        }
    };
}

script_types! {
    EntityComponent = (false, "entityComponent"),
    VehicleTemplate = (true, "vehicle"),
    EntityTemplate = (true, "entity"),
    Item = (false, "item"),
    Recipe = (false, "recipe"),
    UniqueRecipe = (false, "uniquerecipe"),
    EvolvedRecipe = (false, "evolvedrecipe"),
    Fixing = (false, "fixing"),
    AnimationMesh = (false, "animationsMesh"),
    Mannequin = (false, "mannequin"),
    Model = (false, "model"),
    SpriteModel = (false, "spriteModel"),
    Sound = (false, "sound"),
    SoundTimeline = (false, "soundTimeline"),
    Vehicle = (false, "vehicle"),
    RuntimeAnimation = (false, "animation"),
    VehicleEngineRPM = (false, "vehicleEngineRPM"),
    ItemConfig = (false, "itemConfig"),
    Entity = (false, "entity"),
    XuiLayout = (false, "xuiLayout"),
    XuiStyle = (false, "xuiStyle"),
    XuiDefaultStyle = (false, "xuiDefaultStyle"),
    XuiColor = (false, "xuiGlobalColors"),
    XuiSkin = (false, "xuiSkin"),
    XuiConfig = (false, "xuiConfig"),
    ItemFilter = (false, "itemFilter"),
    CraftRecipe = (false, "craftRecipe"),
    FluidFilter = (false, "fluidFilter"),
    StringList = (false, "stringList"),
    EnergyDefinition = (false, "energy"),
    FluidDefinition = (false, "fluid"),
    PhysicsShape = (false, "physicsShape"),
    TimedAction = (false, "timedAction"),
    Ragdoll = (false, "ragdoll"),
    PhysicsHitReaction = (false, "physicsHitReaction"),
    Clock = (false, "clock"),
    CharacterTraitDefinition = (false, "character_trait_definition"),
    CharacterProfessionDefinition = (false, "character_profession_definition"),
}

const DEFAULT_FLAGS: u16 = Flag::Clear as u16
    | Flag::CacheFullType as u16
    | Flag::ResetExisting as u16
    | Flag::RemoveLoadError as u16
    | Flag::SeekImports as u16
    | Flag::AllowNewScriptDiscoveryOnReload as u16;

impl ScriptType {
    /// The flag set assigned in `ScriptType`'s static initializer.
    pub fn flags(self) -> u16 {
        match self {
            ScriptType::Item | ScriptType::Entity => {
                DEFAULT_FLAGS & !(Flag::AllowNewScriptDiscoveryOnReload as u16)
            }
            ScriptType::Vehicle => {
                (DEFAULT_FLAGS & !(Flag::ResetExisting as u16)) | Flag::NewInstanceOnReload as u16
            }
            ScriptType::XuiSkin => {
                Flag::Clear as u16
                    | Flag::CacheFullType as u16
                    | Flag::RemoveLoadError as u16
                    | Flag::SeekImports as u16
                    | Flag::ResetOnceOnReload as u16
            }
            _ => DEFAULT_FLAGS,
        }
    }

    pub fn has_flag(self, flag: Flag) -> bool {
        self.flags() & flag as u16 != 0
    }

    /// `isVerbose` default: only `XuiSkin` is verbose.
    pub fn default_verbose(self) -> bool {
        self == ScriptType::XuiSkin
    }

    /// `ScriptType.GetEnumListLua()`: every type except `EntityComponent`,
    /// non-templates first, each group sorted by name.
    pub fn enum_list_lua() -> Vec<ScriptType> {
        let mut list: Vec<ScriptType> =
            Self::ALL.iter().copied().filter(|t| *t != ScriptType::EntityComponent).collect();
        list.sort_by(|a, b| a.is_template().cmp(&b.is_template()).then(a.name().cmp(b.name())));
        list
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_match_static_initializer() {
        assert!(!ScriptType::Item.has_flag(Flag::AllowNewScriptDiscoveryOnReload));
        assert!(ScriptType::Item.has_flag(Flag::ResetExisting));
        assert!(!ScriptType::Vehicle.has_flag(Flag::ResetExisting));
        assert!(ScriptType::Vehicle.has_flag(Flag::NewInstanceOnReload));
        assert!(ScriptType::XuiSkin.has_flag(Flag::ResetOnceOnReload));
        assert!(!ScriptType::XuiSkin.has_flag(Flag::ResetExisting));
        assert!(ScriptType::Model.has_flag(Flag::AllowNewScriptDiscoveryOnReload));
        assert!(!ScriptType::Model.has_flag(Flag::FromList));
    }

    #[test]
    fn lua_list_puts_templates_last() {
        let l = ScriptType::enum_list_lua();
        assert_eq!(l.len(), ScriptType::ALL.len() - 1);
        assert_eq!(l[0], ScriptType::AnimationMesh);
        assert_eq!(&l[l.len() - 2..], &[ScriptType::EntityTemplate, ScriptType::VehicleTemplate]);
    }
}
