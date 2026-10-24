//! `zombie.network.ServerOptions`.

use std::collections::HashMap;
use std::path::Path;

use pz_classfile::Jar;
use pz_core::config::{ConfigFile, ConfigObject, ConfigOption, OptionKind as CfgKind};
use pz_sandbox::defs::{ctor_calls, Arg};
use pz_text::jfmt::{self, JavaArg};
use pz_text::Translator;

const CLASS: &str = "zombie/network/ServerOptions";

/// Which `ServerOptions.*ServerOption` class an option is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServerOptionKind {
    Boolean,
    String,
    /// `TextServerOption`: a string option whose type is "text".
    Text,
    Integer,
    Double,
    Enum,
}

impl ServerOptionKind {
    pub fn java_class(self) -> &'static str {
        match self {
            ServerOptionKind::Boolean => "zombie.network.ServerOptions$BooleanServerOption",
            ServerOptionKind::String => "zombie.network.ServerOptions$StringServerOption",
            ServerOptionKind::Text => "zombie.network.ServerOptions$TextServerOption",
            ServerOptionKind::Integer => "zombie.network.ServerOptions$IntegerServerOption",
            ServerOptionKind::Double => "zombie.network.ServerOptions$DoubleServerOption",
            ServerOptionKind::Enum => "zombie.network.ServerOptions$EnumServerOption",
        }
    }
}

/// One server option definition as read from the jar.
#[derive(Clone, Debug, PartialEq)]
pub struct ServerOptionDef {
    pub kind: ServerOptionKind,
    pub args: Vec<Arg>,
}

/// The option list of `ServerOptions.<init>` and the names its constructor
/// leaves out of `publicOptions`.
#[derive(Clone, Debug, Default)]
pub struct ServerOptionDefs {
    pub options: Vec<ServerOptionDef>,
    pub private_names: Vec<String>,
}

/// Reads [`ServerOptionDefs`] from the installed jar.
pub fn read_defs(jar: &Jar) -> pz_classfile::Result<ServerOptionDefs> {
    let scan = ctor_calls(jar, CLASS, "zombie/network/ServerOptions$")?;
    let mut options = Vec::new();
    for (class, args) in scan.calls {
        let kind = match class.rsplit('$').next().unwrap_or("") {
            "BooleanServerOption" => ServerOptionKind::Boolean,
            "StringServerOption" => ServerOptionKind::String,
            "TextServerOption" => ServerOptionKind::Text,
            "IntegerServerOption" => ServerOptionKind::Integer,
            "DoubleServerOption" => ServerOptionKind::Double,
            "EnumServerOption" => ServerOptionKind::Enum,
            other => {
                return Err(pz_classfile::Error::Unsupported(format!(
                    "ServerOptions${other}"
                )))
            }
        };
        options.push(ServerOptionDef { kind, args });
    }
    Ok(ServerOptionDefs {
        options,
        private_names: scan.list_removals,
    })
}

/// `Rand.Next(n)`: randomly seeded in the original too.
fn rand_next(n: i64) -> i64 {
    if n <= 0 {
        return 0;
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let x = (nanos as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .rotate_left(17);
    (x % n as u64) as i64
}

/// Evaluates a constructor argument the reader could not fold: the random
/// defaults (`Rand.Next(n)`, `Integer.toString(...)`) and statics that are
/// still unset on a client (`GameServer.seed`).
fn eval(a: &Arg) -> Option<ConfigObject> {
    Some(match a {
        Arg::Int(n) => ConfigObject::Double(*n as f64),
        Arg::Float(f) => ConfigObject::Double(*f),
        Arg::Str(s) => ConfigObject::Str(s.clone()),
        Arg::Null => ConfigObject::Null,
        Arg::Static { class, name } if class == "zombie/network/GameServer" && name == "seed" => {
            ConfigObject::Null
        }
        Arg::Call { class, name, args } if class == "zombie/core/random/Rand" && name == "Next" => {
            let n = match args.first() {
                Some(Arg::Int(n)) => *n,
                _ => return None,
            };
            ConfigObject::Double(rand_next(n) as f64)
        }
        Arg::Call { class, name, args } if class == "java/lang/Integer" && name == "toString" => {
            match eval(args.first()?)? {
                ConfigObject::Double(d) => ConfigObject::Str((d as i64).to_string()),
                _ => return None,
            }
        }
        _ => return None,
    })
}

fn num(a: &Arg) -> f64 {
    match eval(a) {
        Some(ConfigObject::Double(d)) => d,
        _ => 0.0,
    }
}

/// One `ServerOptions.*ServerOption`.
#[derive(Clone, Debug)]
pub struct ServerOption {
    pub config: ConfigOption,
    pub kind: ServerOptionKind,
}

impl ServerOption {
    fn from_def(def: &ServerOptionDef) -> Option<Self> {
        let name = match def.args.first() {
            Some(Arg::Str(s)) => s.clone(),
            _ => return None,
        };
        let a = |i: usize| def.args.get(i).unwrap_or(&Arg::Unknown);
        let string = |i: usize| match eval(a(i)) {
            Some(ConfigObject::Str(s)) => Some(s),
            _ => None,
        };
        let config = match def.kind {
            ServerOptionKind::Boolean => ConfigOption::boolean(&name, num(a(1)) != 0.0),
            ServerOptionKind::Integer => {
                ConfigOption::integer(&name, num(a(1)) as i32, num(a(2)) as i32, num(a(3)) as i32)
            }
            ServerOptionKind::Double => {
                ConfigOption::double(&name, num(a(1)), num(a(2)), num(a(3)))
            }
            ServerOptionKind::Enum => {
                ConfigOption::integer(&name, 1, num(a(1)) as i32, num(a(2)) as i32)
            }
            ServerOptionKind::String | ServerOptionKind::Text => {
                // A null default (GameServer.seed before a server starts)
                // reads as "" through every accessor the port has.
                let default = string(1).unwrap_or_default();
                ConfigOption::string_max(&name, &default, num(a(2)) as i32)
            }
        };
        Some(ServerOption {
            config,
            kind: def.kind,
        })
    }

    pub fn name(&self) -> &str {
        self.config.name()
    }

    /// `getType()`.
    pub fn get_type(&self) -> &'static str {
        match self.kind {
            ServerOptionKind::Text => "text",
            ServerOptionKind::Enum => "enum",
            _ => self.config.get_type(),
        }
    }

    /// `getTooltip()`: the option's `_tooltip` text; integer and double
    /// options add a "min/max/default" line, joined with a literal `\n`.
    pub fn tooltip(&self, t: &Translator) -> Option<String> {
        let s1 = t.get_text_or_null(&format!("UI_ServerOption_{}_tooltip", self.name()), &[]);
        let s2 = match &self.config.kind {
            CfgKind::Integer(i) if self.kind == ServerOptionKind::Integer => Some(t.get_text(
                "Sandbox_MinMaxDefault",
                &[
                    JavaArg::Int(i.min),
                    JavaArg::Int(i.max),
                    JavaArg::Int(i.default_value),
                ],
            )),
            CfgKind::Double(d) => {
                let f = |v: f64| jfmt::format("%.02f", &[JavaArg::Double(v)]).unwrap_or_default();
                Some(t.get_text(
                    "Sandbox_MinMaxDefault",
                    &[
                        JavaArg::Str(f(d.min)),
                        JavaArg::Str(f(d.max)),
                        JavaArg::Str(f(d.default_value)),
                    ],
                ))
            }
            _ => return s1,
        };
        match (s1, s2) {
            (None, s2) => s2,
            (s1, None) => s1,
            (Some(a), Some(b)) => Some(format!("{a}\\n{b}")),
        }
    }

    /// `EnumServerOption.getValueTranslationByIndex(index)`; `None` inside
    /// `Ok` when there is no text, `Err` where the original throws.
    pub fn value_translation_by_index(
        &self,
        t: &Translator,
        index: i32,
    ) -> Result<Option<String>, ()> {
        let n = match &self.config.kind {
            CfgKind::Integer(i) => i.max,
            _ => 0,
        };
        if index < 1 || index > n {
            return Err(());
        }
        Ok(t.get_text_or_null(&format!("UI_ServerOption_AntiCheat_option{index}"), &[]))
    }
}

/// `zombie.network.ServerOptions`.
#[derive(Clone, Debug, Default)]
pub struct ServerOptions {
    pub options: Vec<ServerOption>,
    by_name: HashMap<String, usize>,
    public_options: Vec<String>,
}

impl ServerOptions {
    /// `new ServerOptions()`.
    pub fn new(defs: &ServerOptionDefs) -> Self {
        let mut s = ServerOptions::default();
        for d in &defs.options {
            if let Some(o) = ServerOption::from_def(d) {
                s.by_name.insert(o.name().to_string(), s.options.len());
                s.options.push(o);
            }
        }
        let mut public: Vec<String> = s
            .by_name
            .keys()
            .filter(|k| !defs.private_names.contains(k))
            .cloned()
            .collect();
        public.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
        s.public_options = public;
        s
    }

    pub fn public_options(&self) -> &[String] {
        &self.public_options
    }

    pub fn num_options(&self) -> usize {
        self.options.len()
    }

    pub fn option_by_index(&self, i: usize) -> Option<&ServerOption> {
        self.options.get(i)
    }

    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.by_name.get(name).copied()
    }

    pub fn option_by_name(&self, name: &str) -> Option<&ServerOption> {
        self.index_of(name).map(|i| &self.options[i])
    }

    pub fn option_by_name_mut(&mut self, name: &str) -> Option<&mut ServerOption> {
        self.index_of(name).map(move |i| &mut self.options[i])
    }

    /// `getOption(key)`: the value as a string.
    pub fn get_option(&self, key: &str) -> Option<String> {
        self.option_by_name(key)
            .map(|o| o.config.get_value_as_string())
    }

    /// `getBoolean(key)`.
    pub fn get_boolean(&self, key: &str) -> Option<bool> {
        self.option_by_name(key)
            .filter(|o| o.kind == ServerOptionKind::Boolean)
            .map(|o| o.config.get_bool())
    }

    /// `getFloat(key)`.
    pub fn get_float(&self, key: &str) -> Option<f32> {
        self.option_by_name(key)
            .filter(|o| o.kind == ServerOptionKind::Double)
            .map(|o| o.config.get_double() as f32)
    }

    /// `getDouble(key)`.
    pub fn get_double(&self, key: &str) -> Option<f64> {
        self.option_by_name(key)
            .filter(|o| o.kind == ServerOptionKind::Double)
            .map(|o| o.config.get_double())
    }

    /// `getInteger(key)`.
    pub fn get_integer(&self, key: &str) -> Option<i32> {
        self.option_by_name(key)
            .filter(|o| o.kind == ServerOptionKind::Integer)
            .map(|o| o.config.get_int())
    }

    /// `putOption(key, value)`.
    pub fn put_option(&mut self, key: &str, value: &str) {
        if let Some(o) = self.option_by_name_mut(key) {
            o.config.parse(value);
        }
    }

    /// `getMaxPlayers()`.
    pub fn max_players(&self) -> i32 {
        self.get_integer("MaxPlayers").unwrap_or(0).min(254)
    }

    /// `loadServerTextFile(serverName)` from `<cache>/Server/<name>.ini`.
    pub fn load_server_text_file(&mut self, settings_folder: &Path, server_name: &str) -> bool {
        let mut cf = ConfigFile::new();
        if !cf.read(&settings_folder.join(format!("{server_name}.ini"))) {
            return false;
        }
        let entries: Vec<(String, String)> = cf
            .get_options()
            .iter()
            .map(|c| (c.name().to_string(), c.get_value_as_string()))
            .collect();
        for (name, value) in entries {
            if let Some(o) = self.option_by_name_mut(&name) {
                o.config.parse(&value);
            }
        }
        true
    }

    /// `saveServerTextFile(serverName)`; `Err` where the original throws
    /// for a name containing "..".
    pub fn save_server_text_file(
        &self,
        settings_folder: &Path,
        server_name: &str,
        t: &Translator,
    ) -> Result<bool, String> {
        if server_name.contains("..") {
            return Err(format!(
                "Unable to save server options for serverName: {server_name}"
            ));
        }
        let sep = crate::line_separator();
        let mut out = String::new();
        let line_sep = format!("{sep}{sep}");
        let n = self.options.len();
        for (i, o) in self.options.iter().enumerate() {
            if let Some(tooltip) = o.tooltip(t) {
                let tooltip = tooltip
                    .replace("\\n", " ")
                    .replace("\\\"", "\"")
                    .replace('\n', &format!("{sep}# "));
                out.push_str(&format!("# {tooltip}{sep}"));
            }
            out.push_str(&format!(
                "{}={}{}",
                o.name(),
                o.config.get_value_as_string(),
                if i < n - 1 { line_sep.as_str() } else { "" }
            ));
        }
        Ok(std::fs::write(settings_folder.join(format!("{server_name}.ini")), out).is_ok())
    }

    /// `changeOption(key, value)` (the text the original returns).
    pub fn change_option(
        &mut self,
        settings_folder: &Path,
        server_name: &str,
        key: &str,
        value: &str,
        t: &Translator,
    ) -> String {
        let Some(o) = self.option_by_name_mut(key) else {
            return format!("Option {key} doesn't exist.");
        };
        o.config.parse(value);
        let shown = o.config.get_value_as_string();
        if !matches!(
            self.save_server_text_file(settings_folder, server_name, t),
            Ok(true)
        ) {
            return "An error as occured.".into();
        }
        format!("Option : {key} is now : {shown}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defs() -> ServerOptionDefs {
        let d = |kind, args: Vec<Arg>| ServerOptionDef { kind, args };
        ServerOptionDefs {
            options: vec![
                d(
                    ServerOptionKind::Boolean,
                    vec![Arg::Str("Zeta".into()), Arg::Int(1)],
                ),
                d(
                    ServerOptionKind::Integer,
                    vec![
                        Arg::Str("Alpha".into()),
                        Arg::Int(1),
                        Arg::Int(10),
                        Arg::Int(3),
                    ],
                ),
                d(
                    ServerOptionKind::Integer,
                    vec![
                        Arg::Str("Rnd".into()),
                        Arg::Int(0),
                        Arg::Int(i32::MAX as i64),
                        Arg::Call {
                            class: "zombie/core/random/Rand".into(),
                            name: "Next".into(),
                            args: vec![Arg::Int(5)],
                        },
                    ],
                ),
                d(
                    ServerOptionKind::Text,
                    vec![
                        Arg::Str("Secret".into()),
                        Arg::Str("x".into()),
                        Arg::Int(-1),
                    ],
                ),
            ],
            private_names: vec!["Secret".into()],
        }
    }

    #[test]
    fn options_and_public_list() {
        let s = ServerOptions::new(&defs());
        assert_eq!(s.public_options(), ["Alpha", "Rnd", "Zeta"]);
        assert_eq!(s.get_boolean("Zeta"), Some(true));
        assert_eq!(s.get_integer("Alpha"), Some(3));
        assert!((0..5).contains(&s.get_integer("Rnd").unwrap()));
        assert_eq!(s.option_by_name("Secret").unwrap().get_type(), "text");
        assert_eq!(s.get_boolean("Alpha"), None);
    }
}
