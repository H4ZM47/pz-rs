//! Animals (`zombie.characters.animals`): the definition tables the game
//! builds from its Lua definitions, and the genome rules. See the README
//! for the plan of the rest of the port.
//!
//! Every definition is read at runtime from a Kahlua environment holding
//! the game's own Lua tables (`LuaManager.env` in the game, or
//! [`lua::DefinitionsLua`]); nothing is hard-coded. Where the original keeps
//! a definition set in a `java.util.HashMap`, [`java::JHashMap`] reproduces
//! its iteration order.

pub mod animal;
pub mod behavior;
pub mod breed;
pub mod data;
pub mod definitions;
pub mod env;
pub mod genome;
pub mod java;
pub mod lua;
pub mod migration;
pub mod parts;
pub mod population;
pub mod save;
pub mod tracks;

pub use animal::{Animal, Male, NewAnimal};
pub use behavior::{AnimalBehavior, BehaviorAction, BehaviorObject};
pub use breed::{AnimalBreed, AnimalGrowStage, BreedSound, ForcedGenes};
pub use data::AnimalData;
pub use data::EatFollowUp;
pub use definitions::AnimalDefinitions;
pub use env::{character_ref, character_ref_id, square_ref, square_ref_xyz, AnimalZoneJunction};
pub use env::{
    AnimalCharacter, AnimalClock, AnimalCtx, AnimalEnv, AnimalSandbox, AnimalStat, AnimalWorld,
    CharacterValues, NoWorld, ObjectRef,
};
pub use genome::{AnimalAllele, AnimalGene, AnimalGenomeDefinitions, GeneDefinition};
pub use java::{JHashMap, JResult, JavaException};
pub use migration::{MigrationGroupDefinitions, MigrationGroupDefinitionsGrouped};
pub use parts::AnimalPart;
pub use tracks::{AnimalTracksDefinitions, AnimalTracksType};

use pz_kahlua::TableRef;

/// The definition statics of the original (`AnimalGenomeDefinitions`,
/// `AnimalDefinitions.animalDefs`, `AnimalTracksDefinitions` and
/// `MigrationGroupDefinitions`), loaded from one Lua environment.
/// `AnimalPartsDefinitions` keeps no state and reads the environment on each
/// call ([`parts`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AnimalDefinitionTables {
    pub genome: AnimalGenomeDefinitions,
    pub animal_defs: JHashMap<AnimalDefinitions>,
    pub tracks: JHashMap<AnimalTracksDefinitions>,
    pub migration: JHashMap<MigrationGroupDefinitions>,
}

impl AnimalDefinitionTables {
    /// `loadAnimalDefinitions()` (which loads the genome first),
    /// `loadTracksDefinitions()` and `loadMigrationsDefinitions()`.
    pub fn load(env: &TableRef) -> JResult<Self> {
        let genome = AnimalGenomeDefinitions::load(env)?;
        let animal_defs = AnimalDefinitions::load_all(env)?;
        let tracks = AnimalTracksDefinitions::load_all(env)?;
        let migration = MigrationGroupDefinitions::load_all(env)?;
        Ok(AnimalDefinitionTables {
            genome,
            animal_defs,
            tracks,
            migration,
        })
    }

    /// `AnimalDefinitions.getDef(type)`.
    pub fn def(&self, animal_type: &str) -> Option<&AnimalDefinitions> {
        self.animal_defs.get(animal_type)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::Value;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    /// A unique scratch game directory per call.
    fn game_dir() -> std::path::PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let d = std::env::temp_dir().join(format!("pz-animals-test-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("media/lua/shared/Definitions/animal")).unwrap();
        d
    }

    fn write(dir: &std::path::Path, name: &str, src: &str) {
        std::fs::write(
            dir.join("media/lua/shared/Definitions/animal").join(name),
            src,
        )
        .unwrap();
    }

    #[test]
    fn loads_synthetic_definitions() {
        let dir = game_dir();
        write(
            &dir,
            "B.lua",
            r#"
AnimalGenomeDefinitions = { genes = { maxWeight = { minValue = 0.25, ratio = { meatRatio = 0.5 } },
                                      meatRatio = { forcedValues = true } },
                            geneticDisorder = { dwarf = true, Dwarf = true } }
AnimalDefinitions = AnimalDefinitions or {}
AnimalDefinitions.animals = AnimalDefinitions.animals or {}
AnimalDefinitions.animals["b"] = { babyNbr = "2,4", minAge = 3 * 30, hutches = "a,b,",
  stages = { b = { ageToGrow = 10, nextStage = "c" } },
  breeds = { one = { texture = "x,y", forcedGenes = { MaxWeight = { minValue = 0.1, maxValue = 0.2 } } } },
  genes = { "maxWeight", "meatRatio", "maxWeight" } }
"#,
        );
        write(
            &dir,
            "a.lua",
            r#"AnimalDefinitions = { animals = { a = { wild = true, udder = "TRUE" } } }"#,
        );
        let fs = pz_fs::GameFs::new(&dir);
        let lua = lua::DefinitionsLua::load(&fs).unwrap();
        assert!(lua.errors.is_empty(), "{:?}", lua.errors);
        let t = AnimalDefinitionTables::load(&lua.env()).unwrap();
        // a.lua runs first and B.lua extends its table.
        let a = t.def("a").unwrap();
        assert!(a.wild && a.udder);
        let b = t.def("b").unwrap();
        assert_eq!((b.min_baby, b.max_baby, b.min_age), (2, 4, 90));
        assert_eq!(
            b.hutches.as_deref(),
            Some(&["a".to_string(), "b".to_string()][..])
        );
        assert_eq!(
            b.genes.as_ref().unwrap(),
            &vec!["maxWeight".to_string(), "meatRatio".to_string()]
        );
        assert!(b.is_baby());
        let breed = &b.breeds[0];
        assert_eq!(
            breed.forced_genes.as_ref().unwrap().keys(),
            vec!["maxweight"]
        );
        let g = t.genome.full_genome_def.get("maxweight").unwrap();
        assert_eq!(g.min_value, 0.25);
        assert_eq!(g.max_value, 0.6);
        assert_eq!(
            t.genome.genetic_disorder.as_ref().unwrap(),
            &vec!["dwarf".to_string()]
        );
        let save = b.to_save_definition(&t.genome);
        assert_eq!(save.breeds[0].forced_genes, vec!["maxweight".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bad_number_aborts_like_java() {
        let env = TableRef::new();
        let defs = TableRef::new();
        let animals = TableRef::new();
        let cow = TableRef::new();
        cow.set_str("minAge", Value::str("lots"));
        animals.set_str("cow", Value::Table(cow));
        defs.set_str("animals", Value::Table(animals));
        env.set_str("AnimalDefinitions", Value::Table(defs));
        assert!(AnimalDefinitions::load_all(&env).is_err());
        assert!(AnimalDefinitions::load_all(&TableRef::new())
            .unwrap()
            .is_empty());
    }
}
