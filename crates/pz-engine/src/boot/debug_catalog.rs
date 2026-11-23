//! Construct the ordered DebugOptions tree from the installed constructors.
use pz_classfile::{constructor_effects, load_class, Effect, Jar, Value};
use pz_core::debug::{debug_options, DebugOptionDef};
use std::path::Path;

const ROOT: &str = "zombie/debug/DebugOptions";
const GROUP: &str = "zombie/debug/options/OptionGroup";

pub(super) fn install(jar: &Jar, cache_dir: &Path) -> Result<(), String> {
    let mut definitions = Vec::new();
    read_group(jar, ROOT, &[], "", &mut Vec::new(), &mut definitions)?;
    let shared = debug_options();
    let mut options = shared.borrow_mut();
    options.set_definitions(definitions);
    options.cache_dir = Some(cache_dir.to_owned());
    options.load();
    Ok(())
}

fn read_group(
    jar: &Jar,
    class: &str,
    args: &[Value],
    parent: &str,
    ancestors: &mut Vec<String>,
    out: &mut Vec<DebugOptionDef>,
) -> Result<(), String> {
    if ancestors.iter().any(|c| c == class) {
        return Err(format!("cyclic debug option group {class}"));
    }
    ancestors.push(class.to_owned());
    let body = load_class(jar, class).map_err(|e| format!("debug option class {class}: {e}"))?;
    let mut locals = vec![Value::Unknown];
    locals.extend_from_slice(args);
    let effects = constructor_effects(&body, &locals)
        .map_err(|e| format!("debug option constructor {class}: {e}"))?;
    let name = effects
        .iter()
        .find_map(|e| match e {
            Effect::Call(Value::New { class, args }) if class == GROUP => {
                args.last().and_then(Value::as_str)
            }
            _ => None,
        })
        .unwrap_or_else(|| {
            let simple = class.rsplit(['/', '$']).next().unwrap_or(class);
            simple.strip_suffix("OG").unwrap_or(simple)
        });
    let prefix = if class == ROOT {
        String::new()
    } else {
        format!("{parent}{name}.")
    };
    for effect in effects {
        let Effect::PutField { value, field, .. } = effect else {
            continue;
        };
        let values = match &value {
            Value::Array(v) => v.as_slice(),
            other => std::slice::from_ref(other),
        };
        for value in values {
            match value {
                Value::Call { name, args, .. }
                    if name == "newOption" || name == "newDebugOnlyOption" =>
                {
                    let [_, Value::Str(key), Value::Int(default)] = args.as_slice() else {
                        return Err(format!(
                            "unsupported debug option definition {class}.{field}"
                        ));
                    };
                    out.push(DebugOptionDef {
                        name: format!("{prefix}{key}"),
                        default_value: *default != 0,
                        debug_only: name == "newDebugOnlyOption",
                    });
                }
                Value::Call { name, args, .. } if name == "newOptionGroup" => {
                    let Some(Value::New { class, args }) = args.last() else {
                        return Err(format!("unsupported debug option group {class}.{field}"));
                    };
                    read_group(jar, class, args, &prefix, ancestors, out)?;
                }
                Value::New { class, args } if class.starts_with("zombie/debug/options/") => {
                    read_group(jar, class, args, &prefix, ancestors, out)?;
                }
                _ => (),
            }
        }
    }
    ancestors.pop();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires PZ_GAME_DIR with the installed debug option constructors"]
    fn installed_debug_catalog_preserves_group_order_defaults_and_debug_only_bits() {
        let game = std::path::PathBuf::from(std::env::var_os("PZ_GAME_DIR").unwrap());
        let jar = Jar::open(game.join("projectzomboid.jar")).unwrap();
        let mut defs = Vec::new();
        read_group(&jar, ROOT, &[], "", &mut Vec::new(), &mut defs).unwrap();
        for (name, default, debug_only) in [
            ("Cheat.Window.Unlock", false, true),
            ("Multiplayer.DebugFlags.LocalPlayer.Position", false, true),
            ("ThreadCrash.MainThread.2", false, true),
            ("FBORenderChunk.MipMaps", true, true),
            ("UI.DebugConsole.StartVisible", true, false),
        ] {
            let def = defs
                .iter()
                .find(|d| d.name == name)
                .unwrap_or_else(|| panic!("missing {name}"));
            assert_eq!(
                (def.default_value, def.debug_only),
                (default, debug_only),
                "{name}"
            );
        }
        if let Some(path) = std::env::var_os("PZ_DEBUG_OPTION_REFERENCE") {
            let actual: String = defs
                .iter()
                .map(|d| format!("{}\t{}\t{}\n", d.name, d.default_value, d.debug_only))
                .collect();
            assert_eq!(actual, std::fs::read_to_string(path).unwrap());
        }
    }
}
