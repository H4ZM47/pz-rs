//! What the item tooltip asks the tooltip's character, through the
//! character's Lua binding (`chr:getPerkLevel(Perks.Cooking)`, ...). A
//! method the binding lacks answers as a character without that state
//! would (level 0, no trait, nothing read).

use super::{mod_java_string, mod_value, sv, Cx, LABEL};
use crate::bind_items::{item_arg_value, item_value, ItemRef};
use pz_kahlua::{LuaResult, Thread, Value};
use pz_ui::tooltip::Layout;

pub const ISO_PLAYER: &str = "zombie.characters.IsoPlayer";

pub fn instance_of(v: &Value, class: &str) -> bool {
    match v {
        Value::Object(o) => o.class_name() == class || o.super_classes().contains(&class),
        _ => false,
    }
}

pub fn is_player(v: &Value) -> bool {
    instance_of(v, ISO_PLAYER)
}

/// `obj.name(args)`, or `default` when the binding has no such method.
pub fn call_or(
    t: &mut Thread,
    obj: &Value,
    name: &str,
    args: &[Value],
    default: Value,
) -> LuaResult<Value> {
    if obj.is_nil() {
        return Ok(default);
    }
    // An object without a binding (no metatable) has no such method.
    let Ok(f) = t.table_get(obj, &Value::str(name)) else {
        return Ok(default);
    };
    if f.is_nil() {
        return Ok(default);
    }
    let mut all = vec![obj.clone()];
    all.extend_from_slice(args);
    t.call1(&f, &all)
}

/// A static field of an exposed class (`Perks.Cooking`).
fn static_field(t: &Thread, class: &str, field: &str) -> Value {
    match t.globals().get_str(class) {
        Value::Table(tb) => tb.get_str(field),
        _ => Value::Nil,
    }
}

fn num(v: &Value) -> f64 {
    pz_kahlua::exec::raw_tonumber(v).unwrap_or(0.0)
}

fn s(v: &Value) -> Option<String> {
    match v {
        Value::Nil => None,
        v => Some(pz_kahlua::exec::java_value_of(v)),
    }
}

/// `chr.getPerkLevel(PerkFactory.Perks.<perk>)`.
pub fn perk_level(t: &mut Thread, chr: &Value, perk: &str) -> LuaResult<i32> {
    let p = static_field(t, "Perks", perk);
    Ok(num(&call_or(t, chr, "getPerkLevel", &[p], Value::from(0))?) as i32)
}

/// `chr.hasTrait(CharacterTrait.<name>)`.
pub fn has_trait(t: &mut Thread, chr: &Value, name: &str) -> LuaResult<bool> {
    let tr = static_field(t, "CharacterTrait", name);
    Ok(call_or(t, chr, "hasTrait", &[tr], Value::Bool(false))?.truthy())
}

/// `player.tooDarkToRead()`.
pub fn too_dark_to_read(t: &mut Thread, player: &Value) -> LuaResult<bool> {
    Ok(call_or(t, player, "tooDarkToRead", &[], Value::Bool(false))?.truthy())
}

/// `chr.isLiteratureRead(title)`.
pub fn is_literature_read(t: &mut Thread, chr: &Value, title: &str) -> LuaResult<bool> {
    Ok(call_or(
        t,
        chr,
        "isLiteratureRead",
        &[Value::str(title)],
        Value::Bool(false),
    )?
    .truthy())
}

/// `chr.getAlreadyReadPages(fullType)`.
pub fn already_read_pages(t: &mut Thread, chr: &Value, full_type: &str) -> LuaResult<i32> {
    Ok(num(&call_or(
        t,
        chr,
        "getAlreadyReadPages",
        &[Value::str(full_type)],
        Value::from(0),
    )?) as i32)
}

/// `chr.getKnownRecipes().contains(recipe)` for each recipe.
pub fn knows_recipes(t: &mut Thread, chr: &Value, recipes: &[String]) -> LuaResult<bool> {
    let known = call_or(t, chr, "getKnownRecipes", &[], Value::Nil)?;
    for r in recipes {
        if !call_or(t, &known, "contains", &[Value::str(r)], Value::Bool(false))?.truthy() {
            return Ok(false);
        }
    }
    Ok(true)
}

/// How many of `recipes` the character's profession grants, and how many
/// (trait, recipe) pairs its known traits grant
/// (`CharacterProfessionDefinition.isGrantedRecipe`,
/// `CharacterTraitDefinition.isGrantedRecipe`).
pub fn granted_recipe_counts(
    t: &mut Thread,
    chr: &Value,
    recipes: &[String],
) -> LuaResult<(i32, i32)> {
    let desc = call_or(t, chr, "getDescriptor", &[], Value::Nil)?;
    let prof = call_or(t, &desc, "getCharacterProfession", &[], Value::Nil)?;
    let def_class = static_field(
        t,
        "CharacterProfessionDefinition",
        "getCharacterProfessionDefinition",
    );
    let prof_def = if def_class.is_nil() || prof.is_nil() {
        Value::Nil
    } else {
        t.call1(&def_class, &[prof])?
    };
    let traits = call_or(t, chr, "getCharacterTraits", &[], Value::Nil)?;
    let known = call_or(t, &traits, "getKnownTraits", &[], Value::Nil)?;
    let n = num(&call_or(t, &known, "size", &[], Value::from(0))?) as i32;
    let trait_def = static_field(t, "CharacterTraitDefinition", "getCharacterTraitDefinition");
    let mut defs = Vec::new();
    for i in 0..n {
        let tr = call_or(t, &known, "get", &[Value::from(i)], Value::Nil)?;
        if trait_def.is_nil() {
            continue;
        }
        let d = t.call1(&trait_def, &[tr])?;
        if !d.is_nil() {
            defs.push(d);
        }
    }
    let (mut prof_n, mut trait_n) = (0, 0);
    for r in recipes {
        if call_or(
            t,
            &prof_def,
            "isGrantedRecipe",
            &[Value::str(r)],
            Value::Bool(false),
        )?
        .truthy()
        {
            prof_n += 1;
        }
        for d in &defs {
            if call_or(
                t,
                d,
                "isGrantedRecipe",
                &[Value::str(r)],
                Value::Bool(false),
            )?
            .truthy()
            {
                trait_n += 1;
            }
        }
    }
    Ok((prof_n, trait_n))
}

/// A worn item as `Clothing.DoTooltip` compares it: the item, whether it
/// is clothing, and its body location id.
pub struct Worn {
    pub item: ItemRef,
    pub location: Option<String>,
}

/// `chr.getWornItems()`, in order.
pub fn worn_items(t: &mut Thread, chr: &Value) -> LuaResult<(Vec<Worn>, Value)> {
    let worn = call_or(t, chr, "getWornItems", &[], Value::Nil)?;
    let n = num(&call_or(t, &worn, "size", &[], Value::from(0))?) as i32;
    let mut out = Vec::new();
    for i in 0..n {
        let w = call_or(t, &worn, "get", &[Value::from(i)], Value::Nil)?;
        let item = call_or(t, &w, "getItem", &[], Value::Nil)?;
        let Some(item) = item_arg_value(&item) else {
            continue;
        };
        let loc = call_or(t, &w, "getLocation", &[], Value::Nil)?;
        out.push(Worn {
            item,
            location: s(&loc),
        });
    }
    Ok((out, worn))
}

/// `wornItems.getBodyLocationGroup().isExclusive(a, b)`.
pub fn is_exclusive(t: &mut Thread, worn: &Value, a: &str, b: &str) -> LuaResult<bool> {
    let g = call_or(t, worn, "getBodyLocationGroup", &[], Value::Nil)?;
    Ok(call_or(
        t,
        &g,
        "isExclusive",
        &[Value::str(a), Value::str(b)],
        Value::Bool(false),
    )?
    .truthy())
}

/// `scriptItem.getResearchableRecipes(player).size()` and
/// `scriptItem.isFavouriteRecipeInput(player)`.
/// TODO(crafting): `CraftRecipe.canResearch` and the recipes an item is
/// used in are not ported; neither row shows.
pub fn recipe_flags(_cx: &mut Cx<'_>, _item: &ItemRef) -> LuaResult<(usize, bool)> {
    Ok((0, false))
}

/// `isNoRecipes(player)`: the item's `itemNoRecipes` mod data equals the
/// player's full name.
pub fn is_no_recipes(cx: &mut Cx<'_>, item: &ItemRef) -> LuaResult<bool> {
    crate::bind_items::row_read_flags::is_no_recipes_value(item, &cx.player)
}

/// `isUnwanted(player)`.
pub fn is_unwanted(cx: &mut Cx<'_>, item: &ItemRef) -> LuaResult<bool> {
    let (media, full) = {
        let it = item.borrow();
        let full = sv(&it, |s| s.module_dot_type.clone());
        (it.recorded_media_index >= 0, full)
    };
    if media {
        // TODO(RecordedMedia): player.isUnwanted(getMediaData().getId()).
        return Ok(false);
    }
    let mut keyed = None;
    for key in ["literatureTitle", "printMedia", "collectibleKey"] {
        if let Some(value) = mod_value(item, key)? {
            // Native invokes Object.toString here, unlike the strict
            // String casts in Literature.DoTooltip.
            keyed = Some(mod_java_string(&value)?);
            break;
        }
    }
    let id = keyed.unwrap_or(full);
    let player = cx.player.clone();
    Ok(call_or(
        cx.t,
        &player,
        "isUnwanted",
        &[Value::str(&id)],
        Value::Bool(false),
    )?
    .truthy())
}

/// `player.hasReadMap(item)`.
pub fn has_read_map(cx: &mut Cx<'_>, item: &ItemRef) -> LuaResult<bool> {
    let player = cx.player.clone();
    Ok(call_or(
        cx.t,
        &player,
        "hasReadMap",
        &[item_value(item)],
        Value::Bool(false),
    )?
    .truthy())
}

/// The recorded media rows (title, subtitle, author, watched, heard).
/// TODO(RecordedMedia): `getMediaData()` (the `RecordedMedia` registry)
/// is not ported; no rows.
pub fn recorded_media_rows(
    _cx: &mut Cx<'_>,
    _item: &ItemRef,
    _layout: &mut Layout,
) -> LuaResult<()> {
    let _ = LABEL;
    Ok(())
}

/// `COMPASS_TOOLTIP.get(getOutermostContainer().getParent().getDir())`.
pub fn compass_key(cx: &mut Cx<'_>, item: &ItemRef) -> LuaResult<Option<String>> {
    let v = item_value(item);
    let mut c = call_or(cx.t, &v, "getContainer", &[], Value::Nil)?;
    // getOutermostContainer: up through the containing items.
    loop {
        let ci = call_or(cx.t, &c, "getContainingItem", &[], Value::Nil)?;
        if ci.is_nil() {
            break;
        }
        let up = call_or(cx.t, &ci, "getContainer", &[], Value::Nil)?;
        if up.is_nil() {
            break;
        }
        c = up;
    }
    let parent = call_or(cx.t, &c, "getParent", &[], Value::Nil)?;
    let dir = call_or(cx.t, &parent, "getDir", &[], Value::Nil)?;
    let n = match s(&dir).as_deref() {
        Some("N") => 9,
        Some("NW") => 8,
        Some("W") => 7,
        Some("SW") => 4,
        Some("S") => 1,
        Some("SE") => 2,
        Some("E") => 3,
        Some("NE") => 6,
        // EnumMap.get of another value: null, and getText(null) throws.
        _ => return Err(super::npe()),
    };
    Ok(Some(format!("Tooltip_compass_{n}")))
}

/// `IsoPlayer.getInstance()`'s Lua object.
pub fn player_instance() -> Value {
    crate::bind_players::player_instance()
        .map(|c| crate::bind_character_sim::character_value(&c))
        .unwrap_or_default()
}
