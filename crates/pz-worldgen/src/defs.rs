//! Biome definitions (`zombie.iso.worldgen.biomes`) and
//! `WorldGenReader`, which builds them from the Lua `worldgen` table.

use std::collections::HashMap;
use std::rc::Rc;

use pz_kahlua::{TableRef, Value};

use crate::java::hashmap_order;

/// `FeatureType`, in declaration order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FeatureType {
    Ground,
    Plant,
    Bush,
    Tree,
    Ore,
}

impl FeatureType {
    /// `FeatureType.values()`.
    pub const ALL: [FeatureType; 5] = [
        FeatureType::Ground,
        FeatureType::Plant,
        FeatureType::Bush,
        FeatureType::Tree,
        FeatureType::Ore,
    ];

    pub fn name(self) -> &'static str {
        match self {
            FeatureType::Ground => "GROUND",
            FeatureType::Plant => "PLANT",
            FeatureType::Bush => "BUSH",
            FeatureType::Tree => "TREE",
            FeatureType::Ore => "ORE",
        }
    }

    /// `FeatureType.valueOf(name)`.
    pub fn from_name(s: &str) -> Option<FeatureType> {
        FeatureType::ALL.into_iter().find(|t| t.name() == s)
    }

    pub fn index(self) -> usize {
        self as usize
    }
}

/// One of the six `BiomeType` enums: the Lua key of its selection table
/// (`BiomeType.keys`) and its constant names in declaration order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Axis {
    pub key: &'static str,
    pub names: &'static [&'static str],
}

impl Axis {
    pub fn index_of(&self, name: &str) -> Option<u8> {
        self.names.iter().position(|n| *n == name).map(|i| i as u8)
    }
    /// The `NONE` constant.
    pub fn none(&self) -> u8 {
        self.index_of("NONE").expect("NONE")
    }
    /// `EnumSet.allOf`.
    pub fn all(&self) -> u8 {
        ((1u16 << self.names.len()) - 1) as u8
    }
}

/// `BiomeType.Landscape`, `Plant`, `Bush`, `Temperature`, `Hygrometry`,
/// `OreLevel`.
pub const LANDSCAPE: Axis = Axis {
    key: "landscape",
    names: &["LIGHT_FOREST", "FOREST", "PLAIN", "NONE"],
};
pub const PLANT: Axis = Axis {
    key: "plant",
    names: &["FLOWER", "GRASS", "NONE"],
};
pub const BUSH: Axis = Axis {
    key: "bush",
    names: &["DRY", "REGULAR", "FAT", "NONE"],
};
pub const TEMPERATURE: Axis = Axis {
    key: "temperature",
    names: &["COLD", "MEDIUM", "HOT", "NONE"],
};
pub const HYGROMETRY: Axis = Axis {
    key: "hygrometry",
    names: &["FLOODING", "RAIN", "DRY", "NONE"],
};
pub const ORE_LEVEL: Axis = Axis {
    key: "ore_level",
    names: &["VERY_LOW", "LOW", "MEDIUM", "HIGH", "VERY_HIGH", "NONE"],
};

/// The axes in the order a biome's sets are stored.
pub const AXES: [Axis; 6] = [LANDSCAPE, PLANT, BUSH, TEMPERATURE, HYGROMETRY, ORE_LEVEL];

/// `TileGroup`: a `sx × sy` block of tile names (row-major), possibly
/// containing the `$` markers.
#[derive(Clone, Debug, PartialEq)]
pub struct TileGroup {
    pub sx: i32,
    pub sy: i32,
    pub tiles: Vec<String>,
}

/// `Feature`: tile groups and a probability (`Probability.getValue()`).
#[derive(Clone, Debug, PartialEq)]
pub struct Feature {
    pub tile_groups: Vec<TileGroup>,
    pub min_size: i32,
    pub max_size: i32,
    pub probability: f32,
}

/// `Grass` biome parameters.
#[derive(Clone, Debug, PartialEq)]
pub struct Grass {
    pub fern_chance: f32,
    pub no_grass_div: f32,
    pub no_grass_stages: Vec<f64>,
    pub grass_stages: Vec<f64>,
}

pub type Features = HashMap<FeatureType, Vec<Feature>>;
pub type SubBiomes = HashMap<FeatureType, HashMap<FeatureType, Vec<Rc<Biome>>>>;

/// `Biome` (an `IBiome`). `None` fields are the original's `null`s.
#[derive(Clone, Debug)]
pub struct Biome {
    pub name: String,
    pub parent: Option<String>,
    pub generate: bool,
    pub features: Option<Features>,
    pub replacements: Option<HashMap<String, Vec<Feature>>>,
    /// The `EnumSet`s, one bit per constant, in [`AXES`] order.
    pub sets: [u8; 6],
    pub zombies: f32,
    pub placements: Option<HashMap<FeatureType, Vec<String>>>,
    pub sub_biomes: SubBiomes,
    pub protected: Vec<String>,
    pub grass: Grass,
}

impl Biome {
    /// `Biome.DEFAULT_BIOME`: a `carpentry_02_58` ground and nothing else.
    pub fn default_biome() -> Biome {
        let none = |a: Axis| 1u8 << a.none();
        let mut features = HashMap::new();
        features.insert(
            FeatureType::Ground,
            vec![Feature {
                tile_groups: vec![TileGroup {
                    sx: 1,
                    sy: 1,
                    tiles: vec![MISSING_TILE.to_string()],
                }],
                min_size: 1,
                max_size: 1,
                probability: 1.0,
            }],
        );
        Biome {
            name: "DEFAULT".into(),
            parent: None,
            generate: true,
            features: Some(features),
            replacements: None,
            sets: AXES.map(none),
            zombies: 0.0,
            placements: Some(HashMap::new()),
            sub_biomes: HashMap::new(),
            protected: Vec::new(),
            grass: Grass {
                fern_chance: 0.0,
                no_grass_div: 1.0,
                no_grass_stages: vec![0.4],
                grass_stages: vec![0.33, 0.5],
            },
        }
    }

    pub fn has(&self, axis: usize, value: u8) -> bool {
        self.sets[axis] & (1 << value) != 0
    }
}

/// The sprite `WorldGenTile.applyTile` falls back to (and the default
/// biome's ground).
pub const MISSING_TILE: &str = "carpentry_02_58";

/// `PrefabStructure`.
#[derive(Clone, Debug, PartialEq)]
pub struct PrefabStructure {
    pub dimensions: [i32; 2],
    pub tiles: Vec<String>,
    pub schematic: HashMap<String, Vec<Vec<i32>>>,
    pub zombies: f32,
}

impl PrefabStructure {
    /// `PrefabStructure.categories`, in placement order.
    pub const CATEGORIES: [&'static str; 4] =
        ["Floor", "FloorFurniture", "FloorOverlay", "Furniture"];

    /// `getTileRef(category, x, y)`; `None` where the original would throw.
    pub fn tile_ref(&self, category: &str, x: i32, y: i32) -> Option<i32> {
        let rows = self.schematic.get(category)?;
        rows.get(y.max(0) as usize)?.get(x.max(0) as usize).copied()
    }
}

/// `StaticModule`: a rectangle of world squares generated from a fixed
/// biome or prefab (`worldgen.static_modules`, set by map overrides).
#[derive(Clone, Debug)]
pub struct StaticModule {
    pub biome: Option<Rc<Biome>>,
    pub prefab: Option<PrefabStructure>,
    pub xmin: i32,
    pub xmax: i32,
    pub ymin: i32,
    pub ymax: i32,
}

/// `OreVeinConfig`.
#[derive(Clone, Debug, PartialEq)]
pub struct OreVeinConfig {
    pub tiles: Vec<TileGroup>,
    pub center_radius: i32,
    pub center_prob: f32,
    pub arms_amount_min: i32,
    pub arms_amount_max: i32,
    pub arms_dist_min: i32,
    pub arms_dist_max: i32,
    pub arms_delta_angle: i32,
    pub arms_prob: f32,
    pub probability: f32,
}

/// `RoadConfig`.
#[derive(Clone, Debug, PartialEq)]
pub struct RoadConfig {
    pub tiles: Vec<TileGroup>,
    pub proba_roads: f64,
    pub probability: f64,
    pub filter: f64,
}

/// `BiomeMapEntry`: what a biome-map pixel value means.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BiomeMapEntry {
    pub pixel: i32,
    pub biome: Option<String>,
    pub ore: Option<String>,
    pub zone: Option<String>,
}

/// One axis of `worldgen.selection`: the `[min, max)` noise range of each
/// constant.
pub type Selection = HashMap<u8, Vec<f64>>;

/// Looks up `Sandbox.<name>` probabilities (`ProbaString`): the save's
/// double sandbox option values.
pub type SandboxLookup<'a> = &'a dyn Fn(&str) -> Option<f64>;

/// `WorldGenReader`: Lua tables to definitions.
pub struct Reader<'a> {
    pub sandbox: SandboxLookup<'a>,
}

fn iter(t: &TableRef) -> Vec<(Value, Value)> {
    t.entries()
}

fn table(v: &Value) -> Option<TableRef> {
    v.as_table().cloned()
}

fn get_table(t: &TableRef, key: &str) -> Option<TableRef> {
    table(&t.get_str(key))
}

fn string_of(v: &Value) -> Result<String, String> {
    match v {
        Value::Str(s) => Ok(s.to_string()),
        other => Err(format!(
            "ClassCastException: {} is not a String",
            other.type_name()
        )),
    }
}

fn num_of(v: &Value) -> Result<f64, String> {
    v.as_num()
        .ok_or_else(|| format!("ClassCastException: {} is not a Double", v.type_name()))
}

/// `loadList`: the values in iteration order.
fn load_list(t: Option<&TableRef>) -> Vec<Value> {
    t.map(|t| iter(t).into_iter().map(|(_, v)| v).collect())
        .unwrap_or_default()
}

fn load_double(v: &Value, default: f64) -> Result<f64, String> {
    if v.is_nil() {
        Ok(default)
    } else {
        num_of(v)
    }
}

/// `loadInteger`: `((Double) o).intValue()`.
fn load_int(v: &Value, default: i32) -> Result<i32, String> {
    if v.is_nil() {
        Ok(default)
    } else {
        Ok(num_of(v)? as i32)
    }
}

impl Reader<'_> {
    /// `loadFeatures(main)`: strings become `1 × 1` groups, tables of rows
    /// become `sx × sy` groups.
    pub fn load_tile_groups(&self, main: &TableRef) -> Result<Vec<TileGroup>, String> {
        let mut out = Vec::new();
        for (_, v) in iter(main) {
            match &v {
                Value::Str(s) => out.push(TileGroup {
                    sx: 1,
                    sy: 1,
                    tiles: vec![s.to_string()],
                }),
                Value::Table(t) => {
                    let (mut sx, mut sy) = (0, 0);
                    let mut tiles = Vec::new();
                    for (_, row) in iter(t) {
                        sy += 1;
                        let r = load_list(row.as_table());
                        sx = r.len() as i32;
                        for x in r {
                            tiles.push(string_of(&x)?);
                        }
                    }
                    out.push(TileGroup { sx, sy, tiles });
                }
                _ => return Err("Only strings and tables in there!".into()),
            }
        }
        Ok(out)
    }

    fn probability(&self, v: &Value) -> Result<f32, String> {
        match v {
            Value::Num(d) => Ok(*d as f32),
            Value::Str(s) => {
                let mut parts = s.split('.');
                let class = parts.next().unwrap_or("");
                let field = parts.next().ok_or("ArrayIndexOutOfBoundsException")?;
                if class != "Sandbox" {
                    return Err(format!("Unknown class type: {class}"));
                }
                let d = (self.sandbox)(field).ok_or(format!("Invalid sandbox option: {field}"))?;
                Ok(d as f32)
            }
            _ => Err("Unsupported probability type".into()),
        }
    }

    /// A list of `{ f = { main = ... }, p = ... }` entries.
    fn load_feature_list(&self, t: &TableRef) -> Result<Vec<Feature>, String> {
        let mut features = Vec::new();
        for (_, entry) in iter(t) {
            let e = table(&entry).ok_or("feature entry is not a table")?;
            let f = e.get_str("f");
            let p = e.get_str("p");
            if f.is_nil() || p.is_nil() {
                return Err("Features not found or probability absents".into());
            }
            let main = table(&f)
                .and_then(|f| get_table(&f, "main"))
                .ok_or("NullPointerException: feature without main")?;
            let tile_groups = self.load_tile_groups(&main)?;
            let probability = self.probability(&p)?;
            let mut min_size = i32::MAX;
            let mut max_size = 0;
            for g in &tile_groups {
                min_size = min_size.min(g.sx).min(g.sy);
                max_size = max_size.max(g.sx).max(g.sy);
            }
            features.push(Feature {
                tile_groups,
                min_size,
                max_size,
                probability,
            });
        }
        Ok(features)
    }

    fn load_biome_features(&self, t: Option<TableRef>) -> Result<Option<Features>, String> {
        let Some(t) = t else { return Ok(None) };
        let mut out = HashMap::new();
        for (k, v) in iter(&t) {
            let name = string_of(&k)?;
            let ty = FeatureType::from_name(&name)
                .ok_or(format!("No enum constant FeatureType.{name}"))?;
            let list = table(&v).ok_or("feature type is not a table")?;
            out.insert(ty, self.load_feature_list(&list)?);
        }
        Ok(Some(out))
    }

    fn load_replacements(
        &self,
        t: Option<TableRef>,
    ) -> Result<HashMap<String, Vec<Feature>>, String> {
        let mut out = HashMap::new();
        if let Some(t) = t {
            for (k, v) in iter(&t) {
                let key = string_of(&k)?;
                let list = table(&v).ok_or("replacement is not a table")?;
                out.insert(key, self.load_feature_list(&list)?);
            }
        }
        Ok(out)
    }

    /// `loadBiomeType`: `None` when absent, `{NONE}` when empty.
    fn load_biome_type(&self, axis: Axis, v: &Value) -> Result<Option<u8>, String> {
        let Some(t) = v.as_table() else {
            return Ok(None);
        };
        let mut set = 0u8;
        let mut any = false;
        for (_, v) in iter(t) {
            let name = string_of(&v)?;
            let i = axis
                .index_of(&name)
                .ok_or(format!("No enum constant {}.{name}", axis.key))?;
            set |= 1 << i;
            any = true;
        }
        Ok(Some(if any { set } else { 1 << axis.none() }))
    }

    fn load_placements(
        &self,
        t: Option<TableRef>,
    ) -> Result<Option<HashMap<FeatureType, Vec<String>>>, String> {
        let Some(t) = t else { return Ok(None) };
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        for (k, v) in iter(&t) {
            let list = load_list(v.as_table())
                .iter()
                .map(string_of)
                .collect::<Result<Vec<_>, _>>()?;
            map.insert(string_of(&k)?, list);
        }
        let mut out = HashMap::new();
        for ty in FeatureType::ALL {
            let mut p = Vec::new();
            if let Some(g) = map.get("GENERIC") {
                p.extend(g.iter().cloned());
            }
            if let Some(g) = map.get(ty.name()) {
                p.extend(g.iter().cloned());
            }
            out.insert(ty, p);
        }
        Ok(Some(out))
    }

    fn load_sub_biomes(&self, t: Option<TableRef>) -> Result<SubBiomes, String> {
        let mut out = HashMap::new();
        let Some(t) = t else { return Ok(out) };
        for (k, v) in iter(&t) {
            let ty = FeatureType::from_name(&string_of(&k)?).ok_or("bad subbiome type")?;
            let sub = table(&v).ok_or("subbiome type is not a table")?;
            let mut inner = HashMap::new();
            for (sk, sv) in iter(&sub) {
                let sty = FeatureType::from_name(&string_of(&sk)?).ok_or("bad subbiome type")?;
                let list = table(&sv).ok_or("subbiomes are not a table")?;
                let mut biomes = Vec::new();
                for (bk, bv) in iter(&list) {
                    let name = java_double_string(num_of(&bk)?);
                    let b = self
                        .load_biome(&name, bv.as_table())?
                        .ok_or("NullPointerException: missing subbiome")?;
                    biomes.push(Rc::new(b));
                }
                inner.insert(sty, biomes);
            }
            out.insert(ty, inner);
        }
        Ok(out)
    }

    /// `loadBiome(name, table)`; `None` for a missing table.
    pub fn load_biome(&self, name: &str, t: Option<&TableRef>) -> Result<Option<Biome>, String> {
        let Some(t) = t else { return Ok(None) };
        let parent = match t.get_str("parent") {
            Value::Nil => None,
            v => Some(string_of(&v)?),
        };
        let features = self.load_biome_features(get_table(t, "features"))?;
        let replacements = self.load_replacements(get_table(t, "replacements"))?;
        let params = get_table(t, "params").ok_or("NullPointerException: biome without params")?;
        let mut sets = [0u8; 6];
        for (i, axis) in AXES.iter().enumerate() {
            sets[i] = self
                .load_biome_type(*axis, &params.get_str(axis.key))?
                .unwrap_or(axis.all());
        }
        let placements = self.load_placements(get_table(&params, "placements"))?;
        let sub_biomes = self.load_sub_biomes(get_table(&params, "subbiomes"))?;
        let protected = load_list(get_table(&params, "protected").as_ref())
            .iter()
            .map(string_of)
            .collect::<Result<Vec<_>, _>>()?;
        let grass = match get_table(&params, "grass") {
            Some(g) => {
                let stages = |key: &str, default: Vec<f64>| -> Result<Vec<f64>, String> {
                    let l = load_list(get_table(&g, key).as_ref());
                    if l.is_empty() {
                        Ok(default)
                    } else {
                        l.iter().map(num_of).collect()
                    }
                };
                Grass {
                    fern_chance: load_double(&g.get_str("fernChance"), 0.7)? as f32,
                    no_grass_div: load_double(&g.get_str("noGrassDiv"), 3.0)? as f32,
                    no_grass_stages: stages("noGrassStages", vec![0.4])?,
                    grass_stages: stages("grassStages", vec![0.33, 0.5])?,
                }
            }
            None => Grass {
                fern_chance: 0.7,
                no_grass_div: 3.0,
                no_grass_stages: vec![0.4],
                grass_stages: vec![0.33, 0.5],
            },
        };
        let zombies = load_double(&params.get_str("zombies"), -1.0)? as f32;
        let generate = match params.get_str("generate") {
            Value::Nil => true,
            Value::Bool(b) => b,
            other => {
                return Err(format!(
                    "ClassCastException: {} is not a Boolean",
                    other.type_name()
                ))
            }
        };
        Ok(Some(Biome {
            name: name.to_string(),
            parent,
            generate,
            features,
            replacements: Some(replacements),
            sets,
            zombies,
            placements,
            sub_biomes,
            protected,
            grass,
        }))
    }

    /// `loadBiomes(worldgen, key)` followed by the `generate()` filter and
    /// `Collectors.toMap`: the generating biomes, with parents merged in,
    /// in the resulting `HashMap`'s iteration order.
    pub fn load_biomes(&self, worldgen: &TableRef, key: &str) -> Result<Vec<Rc<Biome>>, String> {
        let main =
            get_table(worldgen, key).ok_or(format!("NullPointerException: worldgen.{key}"))?;
        let mut names = Vec::new();
        let mut raw: HashMap<String, Biome> = HashMap::new();
        for (k, v) in iter(&main) {
            let name = pz_kahlua::exec::java_value_of(&k);
            let b = self
                .load_biome(&name, v.as_table())?
                .ok_or("NullPointerException: biome table")?;
            if !raw.contains_key(&name) {
                names.push(name.clone());
            }
            raw.insert(name, b);
        }
        let mut merged: HashMap<String, Biome> = HashMap::new();
        for name in &names {
            let v = &raw[name];
            let mut chain = vec![v];
            while let Some(p) = chain[0].parent.as_ref().filter(|p| !p.is_empty()) {
                let pb = raw
                    .get(p)
                    .ok_or(format!("NullPointerException: parent {p} of {name}"))?;
                chain.insert(0, pb);
            }
            if chain.len() == 1 {
                continue;
            }
            merged.insert(name.clone(), merge(name, v, &chain));
        }
        for (k, b) in merged {
            raw.insert(k, b);
        }
        // `biomes` (a HashMap filled in Lua table order), then the
        // generating ones collected into a new HashMap in that order.
        let first: Vec<String> = hashmap_order(&names)
            .into_iter()
            .map(|i| names[i].clone())
            .collect();
        let kept: Vec<String> = first.into_iter().filter(|n| raw[n].generate).collect();
        let order = hashmap_order(&kept);
        Ok(order
            .into_iter()
            .map(|i| Rc::new(raw[&kept[i]].clone()))
            .collect())
    }

    /// `loadSelection(axis, worldgen, key)`.
    pub fn load_selection(
        &self,
        axis: Axis,
        worldgen: &TableRef,
        key: &str,
    ) -> Result<Selection, String> {
        let main =
            get_table(worldgen, key).ok_or(format!("NullPointerException: worldgen.{key}"))?;
        let t = get_table(&main, axis.key)
            .ok_or(format!("NullPointerException: selection.{}", axis.key))?;
        let mut out = HashMap::new();
        for (k, v) in iter(&t) {
            let name = string_of(&k)?;
            let i = axis
                .index_of(&name)
                .ok_or(format!("No enum constant {}.{name}", axis.key))?;
            let list = load_list(v.as_table())
                .iter()
                .map(num_of)
                .collect::<Result<Vec<_>, _>>()?;
            out.insert(i, list);
        }
        Ok(out)
    }

    /// `loadStaticModules(worldgen, key)`.
    pub fn load_static_modules(
        &self,
        worldgen: &TableRef,
        key: &str,
    ) -> Result<Vec<StaticModule>, String> {
        let Some(main) = get_table(worldgen, key) else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for (_, v) in iter(&main) {
            let sm = table(&v).ok_or("static module is not a table")?;
            let pos = get_table(&sm, "position").ok_or("NullPointerException: position")?;
            let xmin = load_double(&pos.get_str("xmin"), -f64::MAX)? as i32;
            let xmax = load_double(&pos.get_str("xmax"), f64::MAX)? as i32;
            let ymin = load_double(&pos.get_str("ymin"), -f64::MAX)? as i32;
            let ymax = load_double(&pos.get_str("ymax"), f64::MAX)? as i32;
            let biome = self
                .load_biome("", get_table(&sm, "biome").as_ref())?
                .map(Rc::new);
            let prefab = match get_table(&sm, "prefab") {
                Some(p) => Some(self.load_prefab(&p)?),
                None => None,
            };
            if biome.is_none() && prefab.is_none() {
                return Err("Need at least one of 'biome' or 'prefab' in WorldGenOverride.lua/worlgen.static_modules".into());
            }
            out.push(StaticModule {
                biome,
                prefab,
                xmin,
                xmax,
                ymin,
                ymax,
            });
        }
        Ok(out)
    }

    fn load_prefab(&self, t: &TableRef) -> Result<PrefabStructure, String> {
        let mut dimensions = [0i32; 2];
        let dims = get_table(t, "dimensions").ok_or("NullPointerException: dimensions")?;
        for (k, v) in iter(&dims) {
            let i = num_of(&k)? as i32 - 1;
            *dimensions
                .get_mut(i as usize)
                .ok_or("ArrayIndexOutOfBoundsException")? = num_of(&v)? as i32;
        }
        let tiles = load_list(get_table(t, "tiles").as_ref())
            .iter()
            .map(string_of)
            .collect::<Result<Vec<_>, _>>()?;
        let schem_t = get_table(t, "schematic").ok_or("NullPointerException: schematic")?;
        let mut schematic = HashMap::new();
        for (k, v) in iter(&schem_t) {
            let rows_t = table(&v).ok_or("schematic is not a table")?;
            let mut rows =
                vec![vec![0i32; dimensions[0].max(0) as usize]; dimensions[1].max(0) as usize];
            for (rk, rv) in iter(&rows_t) {
                let i = num_of(&rk)? as i32 - 1;
                let row = string_of(&rv)?
                    .split(',')
                    .map(|s| {
                        s.parse::<i32>()
                            .map_err(|e| format!("NumberFormatException: {e}"))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                *rows
                    .get_mut(i as usize)
                    .ok_or("ArrayIndexOutOfBoundsException")? = row;
            }
            schematic.insert(string_of(&k)?, rows);
        }
        Ok(PrefabStructure {
            dimensions,
            tiles,
            schematic,
            zombies: load_double(&t.get_str("zombies"), 0.0)? as f32,
        })
    }

    /// `loadVeinsConfig(worldgen, key)`, in the resulting `HashMap`'s
    /// iteration order.
    pub fn load_veins(&self, worldgen: &TableRef, key: &str) -> Result<Vec<OreVeinConfig>, String> {
        let Some(main) = get_table(worldgen, key) else {
            return Ok(Vec::new());
        };
        let mut names = Vec::new();
        let mut configs: HashMap<String, OreVeinConfig> = HashMap::new();
        for (k, v) in iter(&main) {
            let key = string_of(&k)?;
            let sub = table(&v).ok_or("vein is not a table")?;
            let feature = get_table(&sub, "feature").ok_or("NullPointerException: feature")?;
            let main = get_table(&feature, "f")
                .and_then(|f| get_table(&f, "main"))
                .ok_or("NullPointerException: vein tiles")?;
            let tiles = self.load_tile_groups(&main)?;
            let arms = get_table(&sub, "arms").ok_or("NullPointerException: arms")?;
            let center = get_table(&sub, "center").ok_or("NullPointerException: center")?;
            let cfg = OreVeinConfig {
                tiles,
                center_radius: load_int(&center.get_str("radius"), 5)?,
                center_prob: load_double(&center.get_str("p"), 0.5)? as f32,
                arms_amount_min: load_int(&arms.get_str("amount_min"), 3)?,
                arms_amount_max: load_int(&arms.get_str("amount_max"), 6)?,
                arms_dist_min: load_int(&arms.get_str("distance_min"), 100)?,
                arms_dist_max: load_int(&arms.get_str("distance_max"), 400)?,
                arms_delta_angle: load_int(&arms.get_str("delta_angle"), 5)?,
                arms_prob: load_double(&arms.get_str("p"), 0.25)? as f32,
                probability: load_double(&sub.get_str("p"), 0.01)? as f32,
            };
            if !configs.contains_key(&key) {
                names.push(key.clone());
            }
            configs.insert(key, cfg);
        }
        Ok(hashmap_order(&names)
            .into_iter()
            .map(|i| configs[&names[i]].clone())
            .collect())
    }

    /// `loadRoadConfig(worldgen, key)`, in the resulting `HashMap`'s
    /// iteration order.
    pub fn load_roads(&self, worldgen: &TableRef, key: &str) -> Result<Vec<RoadConfig>, String> {
        let Some(main) = get_table(worldgen, key) else {
            return Ok(Vec::new());
        };
        let mut names = Vec::new();
        let mut configs: HashMap<String, RoadConfig> = HashMap::new();
        for (k, v) in iter(&main) {
            let key = string_of(&k)?;
            let sub = table(&v).ok_or("road is not a table")?;
            let feature = get_table(&sub, "feature").ok_or("NullPointerException: feature")?;
            let main = get_table(&feature, "f")
                .and_then(|f| get_table(&f, "main"))
                .ok_or("NullPointerException: road tiles")?;
            let cfg = RoadConfig {
                tiles: self.load_tile_groups(&main)?,
                proba_roads: load_double(&feature.get_str("p"), 1.0)?,
                filter: load_double(&sub.get_str("filter_edge"), 5.0e8)?,
                probability: load_double(&sub.get_str("p"), 5.0e-4)?,
            };
            if !configs.contains_key(&key) {
                names.push(key.clone());
            }
            configs.insert(key, cfg);
        }
        Ok(hashmap_order(&names)
            .into_iter()
            .map(|i| configs[&names[i]].clone())
            .collect())
    }

    /// `loadPriorities(worldgen, key)`: tile material to priority.
    pub fn load_priorities(
        &self,
        worldgen: &TableRef,
        key: &str,
    ) -> Result<HashMap<String, f64>, String> {
        let t = get_table(worldgen, key).ok_or(format!("NullPointerException: worldgen.{key}"))?;
        let mut out = HashMap::new();
        for (k, v) in iter(&t) {
            out.insert(string_of(&v)?, num_of(&k)?);
        }
        Ok(out)
    }

    /// `loadBiomeMapConfig(biome_map_config)`: pixel value to entry.
    pub fn load_biome_map_config(
        &self,
        t: &TableRef,
    ) -> Result<HashMap<i32, BiomeMapEntry>, String> {
        let mut out = HashMap::new();
        for (_, v) in iter(t) {
            let sub = table(&v).ok_or("biome map entry is not a table")?;
            let opt = |key: &str| -> Result<Option<String>, String> {
                match sub.get_str(key) {
                    Value::Nil => Ok(None),
                    v => string_of(&v).map(Some),
                }
            };
            let pixel = load_int(&sub.get_str("pixel"), 0)?;
            out.insert(
                pixel,
                BiomeMapEntry {
                    pixel,
                    biome: opt("biome")?,
                    ore: opt("ore")?,
                    zone: opt("zone")?,
                },
            );
        }
        Ok(out)
    }
}

/// The `loadBiomes` merge of a biome with its ancestors (`chain` is root
/// first and ends with `v`).
fn merge(name: &str, v: &Biome, chain: &[&Biome]) -> Biome {
    let root = chain[0];
    let mut features: Features = HashMap::new();
    let mut replacements = HashMap::new();
    let mut sets = root.sets;
    let mut placements = root.placements.clone();
    let mut sub_biomes = root.sub_biomes.clone();
    let mut protected = root.protected.clone();
    let mut zombies = root.zombies;
    let mut grass = root.grass.clone();
    for b in chain {
        if let Some(f) = &b.features {
            for ty in [
                FeatureType::Ground,
                FeatureType::Tree,
                FeatureType::Plant,
                FeatureType::Bush,
                FeatureType::Ore,
            ] {
                if let Some(l) = f.get(&ty) {
                    features.insert(ty, l.clone());
                }
            }
        }
        if let Some(r) = &b.replacements {
            for (k, l) in r {
                replacements.insert(k.clone(), l.clone());
            }
        }
        for (i, axis) in AXES.iter().enumerate() {
            if b.sets[i] != axis.all() {
                sets[i] = b.sets[i];
            }
        }
        if b.placements.as_ref().is_some_and(|p| !p.is_empty()) {
            placements = b.placements.clone();
        }
        if !b.sub_biomes.is_empty() {
            sub_biomes = b.sub_biomes.clone();
        }
        if !b.protected.is_empty() {
            protected = b.protected.clone();
        }
        if b.zombies as f64 >= 0.0 {
            zombies = b.zombies;
        }
        grass = b.grass.clone();
    }
    Biome {
        name: name.to_string(),
        parent: v.parent.clone(),
        generate: v.generate,
        features: Some(features),
        replacements: Some(replacements),
        sets,
        zombies,
        placements,
        sub_biomes,
        protected,
        grass,
    }
}

/// `Double.toString(d)` for the integral keys subbiome lists use.
fn java_double_string(d: f64) -> String {
    if d.fract() == 0.0 && d.abs() < 1e7 {
        format!("{d:.1}")
    } else {
        format!("{d}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(entries: Vec<(Value, Value)>) -> TableRef {
        let t = TableRef::new();
        for (k, v) in entries {
            t.rawset(k, v);
        }
        t
    }
    fn s(x: &str) -> Value {
        Value::str(x)
    }
    fn n(x: f64) -> Value {
        Value::Num(x)
    }
    fn list(vals: Vec<Value>) -> Value {
        Value::Table(t(vals
            .into_iter()
            .enumerate()
            .map(|(i, v)| (n(i as f64 + 1.0), v))
            .collect()))
    }

    #[test]
    fn tile_groups_from_strings_and_rows() {
        let r = Reader { sandbox: &|_| None };
        let main = t(vec![
            (n(1.0), s("a_0")),
            (
                n(2.0),
                list(vec![
                    list(vec![s("b_0"), s("$any")]),
                    list(vec![s("$subbiome"), s("b_1")]),
                ]),
            ),
        ]);
        let g = r.load_tile_groups(&main).unwrap();
        assert_eq!(
            g[0],
            TileGroup {
                sx: 1,
                sy: 1,
                tiles: vec!["a_0".into()]
            }
        );
        assert_eq!(g[1].sx, 2);
        assert_eq!(g[1].sy, 2);
        assert_eq!(g[1].tiles, ["b_0", "$any", "$subbiome", "b_1"]);
    }

    #[test]
    fn biome_merge_and_filter() {
        let r = Reader {
            sandbox: &|k| (k == "Chance").then_some(0.25),
        };
        let feat = |tile: &str, p: Value| {
            Value::Table(t(vec![
                (
                    s("f"),
                    Value::Table(t(vec![(s("main"), list(vec![s(tile)]))])),
                ),
                (s("p"), p),
            ]))
        };
        let parent = t(vec![
            (
                s("features"),
                Value::Table(t(vec![(s("TREE"), list(vec![feat("tree_0", n(0.5))]))])),
            ),
            (
                s("params"),
                Value::Table(t(vec![
                    (s("landscape"), list(vec![s("FOREST")])),
                    (s("plant"), list(vec![])),
                    (s("generate"), Value::Bool(false)),
                ])),
            ),
        ]);
        let child = t(vec![
            (s("parent"), s("p")),
            (
                s("features"),
                Value::Table(t(vec![(
                    s("ORE"),
                    list(vec![feat("rock_0", s("Sandbox.Chance"))]),
                )])),
            ),
            (
                s("params"),
                Value::Table(t(vec![(s("ore_level"), list(vec![s("LOW")]))])),
            ),
        ]);
        let wg = t(vec![(
            s("biomes"),
            Value::Table(t(vec![
                (s("p"), Value::Table(parent)),
                (s("c"), Value::Table(child)),
            ])),
        )]);
        let b = r.load_biomes(&wg, "biomes").unwrap();
        assert_eq!(b.len(), 1);
        let c = &b[0];
        assert_eq!(c.name, "c");
        let f = c.features.as_ref().unwrap();
        assert_eq!(f[&FeatureType::Tree][0].tile_groups[0].tiles[0], "tree_0");
        assert_eq!(f[&FeatureType::Ore][0].probability, 0.25);
        assert_eq!(c.sets[0], 1 << LANDSCAPE.index_of("FOREST").unwrap());
        assert_eq!(c.sets[1], 1 << PLANT.none());
        assert_eq!(c.sets[5], 1 << ORE_LEVEL.index_of("LOW").unwrap());
        assert_eq!(c.sets[2], BUSH.all());
        assert!(c.placements.is_none());
        assert_eq!(c.zombies, -1.0);
    }

    #[test]
    fn double_keys() {
        assert_eq!(java_double_string(1.0), "1.0");
        assert_eq!(java_double_string(12.0), "12.0");
    }
}
