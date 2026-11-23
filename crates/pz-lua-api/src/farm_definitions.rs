//! Raw definition lookups used by new and saved farm objects. These tables
//! are populated by the installed Lua scripts and may also be edited by mods.
use pz_kahlua::TableRef;

fn tables(parent: &TableRef) -> impl Iterator<Item = TableRef> {
    parent
        .entries()
        .into_iter()
        .filter_map(|(_, value)| value.as_table().cloned())
}

pub fn trough(globals: &TableRef, sprite: &str) -> Option<TableRef> {
    let defs = globals.get_str("FeedingTroughDef");
    tables(defs.as_table()?).find(|def| {
        ["spriteW", "spriteN"].iter().any(|key| {
            def.get_str(key).as_table().is_some_and(|list| {
                list.entries()
                    .iter()
                    .any(|(_, v)| v.as_str() == Some(sprite))
            })
        })
    })
}

pub fn hutch(globals: &TableRef, sprite: &str) -> Option<TableRef> {
    if sprite.is_empty() {
        return None;
    }
    let defs = globals.get_str("HutchDefinitions");
    let groups = defs.as_table()?.get_str("hutchs");
    tables(groups.as_table()?).find(|def| {
        def.get_str("baseSprite").as_str() == Some(sprite)
            || [
                ("extraSprites", "spriteOpen"),
                ("eggHatchDoors", "closedSprite"),
            ]
            .iter()
            .any(|(list, alternate)| {
                def.get_str(list).as_table().is_some_and(|items| {
                    tables(items).any(|item| {
                        ["sprite", alternate]
                            .iter()
                            .any(|key| item.get_str(key).as_str() == Some(sprite))
                    })
                })
            })
    })
}
