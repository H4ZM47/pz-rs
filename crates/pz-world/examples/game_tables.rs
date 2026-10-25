//! Prints the runtime tables pz-world reads from the installed game's
//! `projectzomboid.jar` (`SpriteTables`, `ObjectTables`, `SaveTables`), each
//! in its `from_text` format.
//!
//! ```text
//! game_tables <path to projectzomboid.jar>
//! ```

use pz_world::save::SaveTables;
use pz_world::{ObjectTables, SpriteTables};

fn main() {
    let jar = std::env::args()
        .nth(1)
        .expect("usage: game_tables <projectzomboid.jar>");
    let jar = std::path::Path::new(&jar);
    print!(
        "{}",
        SpriteTables::from_game_jar(jar)
            .expect("sprite tables")
            .to_text()
    );
    print!(
        "{}",
        ObjectTables::from_game_jar(jar)
            .expect("object tables")
            .to_text()
    );
    print!(
        "{}",
        SaveTables::from_game_jar(jar)
            .expect("save tables")
            .to_text()
    );
}
