//! `IsoFlagType`, `IsoObjectType` (`zombie.iso.SpriteDetails`) and
//! `FasciaEdge` (`zombie.core.properties`) as runtime tables.
//!
//! The enum constant lists are game code constants and are not part of this
//! repository: they are read from the installed game at runtime and handed to
//! [`SpriteTables::new`]. Each list is in ordinal order and excludes the
//! trailing `MAX` constant of `IsoFlagType`/`IsoObjectType`, whose ordinal is
//! the list length.
//!
//! `IsoPropertyType` needs no table: `lookupOrDefaultStr` returns the name
//! it was given either way, and `isProperty` is a case-insensitive name
//! comparison, so the loader uses the property names directly.

use std::collections::HashMap;

/// An `IsoFlagType` ordinal (`index()`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IsoFlagType(pub u8);

impl IsoFlagType {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// An `IsoObjectType` ordinal (`index()`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IsoObjectType(pub u8);

impl IsoObjectType {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// A `FasciaEdge` ordinal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FasciaEdge(pub u8);

macro_rules! known {
    ($name:ident, $ty:ident, [$($f:ident),* $(,)?]) => {
        /// Constants the loader refers to by name, resolved from the tables.
        #[allow(non_snake_case)]
        #[derive(Clone, Copy, Debug)]
        pub struct $name {
            $(pub $f: $ty,)*
        }

        impl $name {
            fn resolve(map: &HashMap<String, $ty>) -> Result<Self, String> {
                Ok($name {
                    $($f: *map.get(stringify!($f)).ok_or_else(|| {
                        format!("{} table has no {}", stringify!($ty), stringify!($f))
                    })?,)*
                })
            }
        }
    };
}

known!(
    Flags,
    IsoFlagType,
    [
        collideW,
        collideN,
        exterior,
        NoWallLighting,
        doorW,
        doorN,
        transparentW,
        transparentN,
        cutW,
        cutN,
        solid,
        invisible,
        HoppableN,
        HoppableW,
        canPathW,
        canPathN,
        blocksight,
        ForceAmbient,
        WallSE,
        FloorHeightOneThird,
        FloorHeightTwoThirds,
        CantClimb,
        open,
        windowW,
        windowN,
        WindowW,
        WindowN,
        diamondFloor,
        transparentFloor,
        solidfloor,
        hidewalls,
        container,
        WallOverlay,
        attachedSE,
        attachedW,
        attachedN,
        FloorOverlay,
        vegitation,
        NeverCutaway,
        HasLightOnSprite,
        EntityScript,
        water,
        solidtrans,
        blueprint,
        BlockRain,
        trans,
        noStart,
        forceRender,
        burntOut,
    ]
);

known!(
    ObjectTypes,
    IsoObjectType,
    [
        wall,
        doorW,
        doorN,
        tree,
        windowFN,
        windowFW,
        isMoveAbleObject,
        lightswitch,
        curtainN,
        curtainS,
        curtainE,
        curtainW,
        jukebox,
        radio,
        doorFrN,
        doorFrW,
        stairsTN,
        stairsMN,
        stairsBN,
        stairsTW,
        stairsMW,
        stairsBW,
    ]
);

/// The enum tables the sprite loader needs.
#[derive(Clone, Debug)]
pub struct SpriteTables {
    flag_names: Vec<String>,
    object_type_names: Vec<String>,
    fascia_edge_names: Vec<String>,
    flag_by_name: HashMap<String, IsoFlagType>,
    object_type_by_name: HashMap<String, IsoObjectType>,
    fascia_edge_by_name: HashMap<String, FasciaEdge>,
    pub flag: Flags,
    pub ty: ObjectTypes,
}

fn index_map<T>(names: &[String], mk: impl Fn(u8) -> T) -> HashMap<String, T> {
    let mut m = HashMap::new();
    for (i, n) in names.iter().enumerate() {
        m.insert(n.clone(), mk(i as u8));
    }
    m
}

impl SpriteTables {
    /// Builds the tables from the enum constant names in ordinal order
    /// (`IsoFlagType` and `IsoObjectType` without `MAX`).
    pub fn new(
        flag_names: Vec<String>,
        object_type_names: Vec<String>,
        fascia_edge_names: Vec<String>,
    ) -> Result<Self, String> {
        // Flags are kept in two 64-bit words.
        if flag_names.len() > 128 {
            return Err(format!("{} flags do not fit in 128 bits", flag_names.len()));
        }
        if object_type_names.len() >= 255 || fascia_edge_names.len() >= 255 {
            return Err("enum table too large".into());
        }
        let flag_by_name = index_map(&flag_names, IsoFlagType);
        let object_type_by_name = index_map(&object_type_names, IsoObjectType);
        let fascia_edge_by_name = index_map(&fascia_edge_names, FasciaEdge);
        Ok(SpriteTables {
            flag: Flags::resolve(&flag_by_name)?,
            ty: ObjectTypes::resolve(&object_type_by_name)?,
            flag_names,
            object_type_names,
            fascia_edge_names,
            flag_by_name,
            object_type_by_name,
            fascia_edge_by_name,
        })
    }

    /// Reads the tables from the installed game's `projectzomboid.jar`
    /// (see [`SpriteTables::from_jar`]).
    pub fn from_game_jar(jar: &std::path::Path) -> Result<Self, String> {
        let jar = pz_classfile::Jar::open(jar).map_err(|e| format!("{}: {e}", jar.display()))?;
        Self::from_jar(&jar)
    }

    /// Reads the tables from an open game jar: the enum constants of
    /// `zombie.iso.SpriteDetails.IsoFlagType` and `IsoObjectType` (each
    /// without its trailing `MAX`) and `zombie.core.properties.FasciaEdge`,
    /// in declaration order.
    pub fn from_jar(jar: &pz_classfile::Jar) -> Result<Self, String> {
        let names = |class: &str, drop_max: bool| -> Result<Vec<String>, String> {
            let mut v =
                pz_classfile::enum_constants(jar, class).map_err(|e| format!("{class}: {e}"))?;
            if drop_max && v.last().map(String::as_str) == Some("MAX") {
                v.pop();
            }
            Ok(v)
        };
        Self::new(
            names("zombie/iso/SpriteDetails/IsoFlagType", true)?,
            names("zombie/iso/SpriteDetails/IsoObjectType", true)?,
            names("zombie/core/properties/FasciaEdge", false)?,
        )
    }

    /// Reads the tables from text: three lines, each a comma-separated list
    /// (flag names, object type names, fascia edge names).
    pub fn from_text(text: &str) -> Result<Self, String> {
        let mut lines = text.lines();
        let mut next = || -> Vec<String> {
            lines
                .next()
                .unwrap_or("")
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        };
        let (f, o, e) = (next(), next(), next());
        Self::new(f, o, e)
    }

    /// The tables in the [`SpriteTables::from_text`] format.
    pub fn to_text(&self) -> String {
        format!(
            "{}\n{}\n{}\n",
            self.flag_names.join(","),
            self.object_type_names.join(","),
            self.fascia_edge_names.join(",")
        )
    }

    /// `IsoObjectType.MAX`.
    pub fn object_type_max(&self) -> IsoObjectType {
        IsoObjectType(self.object_type_names.len() as u8)
    }

    /// Number of `IsoFlagType` constants before `MAX`.
    pub fn flag_count(&self) -> usize {
        self.flag_names.len()
    }

    /// `IsoFlagType.FromString`: exact name, `None` for `MAX`/unknown.
    pub fn flag_from_string(&self, s: &str) -> Option<IsoFlagType> {
        self.flag_by_name.get(s).copied()
    }

    /// `IsoObjectType.FromString`: exact name, `MAX` when unknown.
    pub fn object_type_from_string(&self, s: &str) -> IsoObjectType {
        self.object_type_by_name
            .get(s)
            .copied()
            .unwrap_or(self.object_type_max())
    }

    /// `StringUtils.tryParseEnum(FasciaEdge.class, s, null)`.
    pub fn fascia_edge_from_string(&self, s: &str) -> Option<FasciaEdge> {
        self.fascia_edge_by_name.get(s).copied()
    }

    pub fn flag_name(&self, f: IsoFlagType) -> &str {
        self.flag_names.get(f.index()).map_or("MAX", |s| s.as_str())
    }

    pub fn object_type_name(&self, t: IsoObjectType) -> &str {
        self.object_type_names
            .get(t.index())
            .map_or("MAX", |s| s.as_str())
    }

    pub fn fascia_edge_name(&self, e: FasciaEdge) -> &str {
        &self.fascia_edge_names[e.0 as usize]
    }
}

/// Hand-authored tables for tests: the names the loader refers to plus a few
/// others, in an arbitrary order (not the game's ordinals).
/// Exported for other crates' tests with the `test-util` feature.
#[cfg(any(test, feature = "test-util"))]
pub fn test_tables() -> SpriteTables {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    SpriteTables::new(
        s(&[
            "open",
            "solid",
            "cutN",
            "cutW",
            "doorN",
            "doorW",
            "collideN",
            "collideW",
            "canPathN",
            "canPathW",
            "transparentN",
            "transparentW",
            "HoppableN",
            "HoppableW",
            "blocksight",
            "exterior",
            "invisible",
            "WallSE",
            "WindowN",
            "WindowW",
            "CantClimb",
            "NoWallLighting",
            "ForceAmbient",
            "FloorHeightOneThird",
            "FloorHeightTwoThirds",
            "attachedFloor",
            "container",
            "solidfloor",
            "windowW",
            "windowN",
            "diamondFloor",
            "transparentFloor",
            "hidewalls",
            "WallOverlay",
            "attachedSE",
            "attachedW",
            "attachedN",
            "FloorOverlay",
            "vegitation",
            "NeverCutaway",
            "HasLightOnSprite",
            "EntityScript",
            "water",
            "solidtrans",
            "blueprint",
            "BlockRain",
            "trans",
            "noStart",
            "forceRender",
            "burntOut",
        ]),
        s(&[
            "normal",
            "tree",
            "wall",
            "doorN",
            "doorW",
            "windowFN",
            "windowFW",
            "stairsTW",
            "isMoveAbleObject",
            "lightswitch",
            "curtainN",
            "curtainS",
            "curtainE",
            "curtainW",
            "jukebox",
            "radio",
            "doorFrN",
            "doorFrW",
            "stairsTN",
            "stairsMN",
            "stairsBN",
            "stairsMW",
            "stairsBW",
        ]),
        s(&["North", "West"]),
    )
    .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookups() {
        let t = test_tables();
        assert_eq!(t.flag_from_string("cutN"), Some(t.flag.cutN));
        assert_eq!(t.flag_from_string("MAX"), None);
        assert_eq!(t.object_type_from_string("x"), t.object_type_max());
        assert_eq!(t.object_type_name(t.ty.tree), "tree");
        assert_eq!(t.fascia_edge_from_string("West"), Some(FasciaEdge(1)));
        assert!(SpriteTables::new(vec![], vec![], vec![]).is_err());
        let txt = "open,solid\nwall\n";
        assert!(SpriteTables::from_text(txt).is_err());
        let back = SpriteTables::from_text(&t.to_text()).unwrap();
        assert_eq!(back.to_text(), t.to_text());
        assert_eq!(back.flag.cutN, t.flag.cutN);
    }
}
