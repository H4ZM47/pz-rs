//! Build 42.22 `zombie.text.templating`:
//! `TemplateText`, `TemplateTextBuilder`, `ReplaceProvider`, `ReplaceList`,
//! `ReplaceSingle`.

use std::collections::HashMap;

/// Empty-list sentinel matching Build 42.22 `ReplaceList.getString()`.
pub const ERROR_EMPTY_LIST: &str = "!ERROR_EMPTY_LIST!";

/// Simple deterministic/seeded PRNG backing `TemplateText.RandNext`.
#[derive(Clone, Debug)]
pub struct TemplateRng {
    state: u64,
}

impl Default for TemplateRng {
    fn default() -> Self {
        Self { state: 0x9E37_79B9_7F4A_7C15 }
    }
}

impl TemplateRng {
    pub fn with_seed(seed: u64) -> Self {
        Self { state: if seed == 0 { 0x9E37_79B9_7F4A_7C15 } else { seed } }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    /// `TemplateText.RandNext(int max)` (`[0, max)`).
    pub fn rand_next_int(&mut self, max: i32) -> i32 {
        if max <= 0 {
            return 0;
        }
        (self.next_u64() % (max as u64)) as i32
    }

    /// `TemplateText.RandNext(int min, int max)` (`[min, max)`).
    pub fn rand_next_int_range(&mut self, min: i32, max: i32) -> i32 {
        if max <= min {
            return min;
        }
        let span = (max as i64 - min as i64) as u64;
        min + (self.next_u64() % span) as i32
    }

    /// `TemplateText.RandNext(float max)` (`[0.0, max)`).
    pub fn rand_next_float(&mut self, max: f32) -> f32 {
        if max <= 0.0 {
            return 0.0;
        }
        let unit = ((self.next_u64() >> 40) as f32) / ((1u32 << 24) as f32);
        unit * max
    }

    /// `TemplateText.RandNext(float min, float max)` (`[min, max)`).
    pub fn rand_next_float_range(&mut self, min: f32, max: f32) -> f32 {
        if max <= min {
            return min;
        }
        let unit = ((self.next_u64() >> 40) as f32) / ((1u32 << 24) as f32);
        min + unit * (max - min)
    }
}

/// Built-in dynamic name providers registered by `TemplateText.Initialize()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuiltinNameKind {
    Lastname,
    Firstname,
    MaleName,
    FemaleName,
}

/// A single `IReplace` implementation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Replacer {
    Single(String),
    List(Vec<String>),
    Builtin(BuiltinNameKind),
}

impl Replacer {
    /// Evaluate this replacer using `rng` and optional `builtin` resolver.
    pub fn resolve<F>(&self, rng: &mut TemplateRng, mut builtin: F) -> String
    where
        F: FnMut(BuiltinNameKind, &mut TemplateRng) -> String,
    {
        match self {
            Replacer::Single(s) => s.clone(),
            Replacer::List(items) => {
                if items.is_empty() {
                    ERROR_EMPTY_LIST.to_string()
                } else {
                    let idx = rng.rand_next_int(items.len() as i32) as usize;
                    items[idx].clone()
                }
            }
            Replacer::Builtin(kind) => builtin(*kind, rng),
        }
    }
}

/// Build 42.22 `zombie.text.templating.ReplaceProvider`.
#[derive(Clone, Debug, Default)]
pub struct ReplaceProvider {
    keys: HashMap<String, Replacer>,
}

impl ReplaceProvider {
    pub fn new() -> Self {
        Self::default()
    }

    /// `addKey(String key, String value)`.
    pub fn add_single(&mut self, key: &str, value: impl Into<String>) {
        self.keys.insert(key.to_lowercase(), Replacer::Single(value.into()));
    }

    /// `addKey(String key, KahluaTableImpl table)` — ignores empty lists just
    /// like Build 42.22 `ReplaceProvider.addKey`.
    pub fn add_list(&mut self, key: &str, values: Vec<String>) {
        if values.is_empty() {
            return;
        }
        self.keys.insert(key.to_lowercase(), Replacer::List(values));
    }

    /// `addReplacer(String key, IReplace replace)`.
    pub fn add_replacer(&mut self, key: &str, replacer: Replacer) {
        self.keys.insert(key.to_lowercase(), replacer);
    }

    /// `hasReplacer(String key)`.
    pub fn has_replacer(&self, key: &str) -> bool {
        self.keys.contains_key(&key.to_lowercase())
    }

    /// `getReplacer(String key)`.
    pub fn get_replacer(&self, key: &str) -> Option<&Replacer> {
        self.keys.get(&key.to_lowercase())
    }

    /// Remove all keys.
    pub fn clear(&mut self) {
        self.keys.clear();
    }
}

/// Build 42.22 `zombie.text.templating.TemplateTextBuilder`.
#[derive(Clone, Debug, Default)]
pub struct TemplateTextBuilder {
    keys: HashMap<String, Replacer>,
}

impl TemplateTextBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// `Reset()`.
    pub fn reset(&mut self) {
        self.keys.clear();
    }

    /// `CopyFrom(other)`.
    pub fn copy_from(&mut self, other: &TemplateTextBuilder) {
        self.keys = other.keys.clone();
    }

    /// `RegisterKey(key, string)`.
    pub fn register_single(&mut self, key: &str, value: impl Into<String>) {
        self.keys.insert(key.to_lowercase(), Replacer::Single(value.into()));
    }

    /// `RegisterKey(key, list)` — empty list is ignored per Build 42.22.
    pub fn register_list(&mut self, key: &str, values: Vec<String>) {
        if values.is_empty() {
            return;
        }
        self.keys.insert(key.to_lowercase(), Replacer::List(values));
    }

    /// `RegisterKey(key, IReplace)`.
    pub fn register_replacer(&mut self, key: &str, replacer: Replacer) {
        self.keys.insert(key.to_lowercase(), replacer);
    }

    /// Register Build 42.22's four default name keys (`lastname`, `firstname`,
    /// `maleName`, `femaleName`).
    pub fn register_default_name_keys(&mut self) {
        self.register_replacer("lastname", Replacer::Builtin(BuiltinNameKind::Lastname));
        self.register_replacer("firstname", Replacer::Builtin(BuiltinNameKind::Firstname));
        self.register_replacer("maleName", Replacer::Builtin(BuiltinNameKind::MaleName));
        self.register_replacer("femaleName", Replacer::Builtin(BuiltinNameKind::FemaleName));
    }

    /// `Build(input, overrides)` replacing every `${key}` token matching
    /// `\$\{([^}]+)\}` using `overrides` first, then `self.keys`, else
    /// `missing_<key>`.
    pub fn build<F>(
        &self,
        input: &str,
        overrides: Option<&ReplaceProvider>,
        rng: &mut TemplateRng,
        mut builtin: F,
    ) -> String
    where
        F: FnMut(BuiltinNameKind, &mut TemplateRng) -> String,
    {
        let mut out = String::with_capacity(input.len());
        let bytes = input.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
                if let Some(rel_end) = input[i + 2..].find('}') {
                    let raw_key = &input[i + 2..i + 2 + rel_end];
                    if !raw_key.is_empty() {
                        let lower = raw_key.to_lowercase();
                        let replacement = if let Some(rep) =
                            overrides.and_then(|ov| ov.get_replacer(&lower))
                        {
                            rep.resolve(rng, &mut builtin)
                        } else if let Some(rep) = self.keys.get(&lower) {
                            rep.resolve(rng, &mut builtin)
                        } else {
                            format!("missing_{raw_key}")
                        };
                        out.push_str(&replacement);
                        i += 2 + rel_end + 1;
                        continue;
                    }
                }
            }
            let ch = input[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_builder_substitutes_overrides_and_missing_keys() {
        let mut rng = TemplateRng::with_seed(42);
        let mut builder = TemplateTextBuilder::new();
        builder.register_default_name_keys();
        builder.register_list("fruit", vec!["Apple".into(), "Orange".into()]);

        let mut ov = ReplaceProvider::new();
        ov.add_single("FIRSTNAME", "Jane");
        ov.add_single("lastname", "Doe");

        let rendered = builder.build(
            "Hello ${firstname} ${LastName}, enjoy a ${fruit} and ${unknown}!",
            Some(&ov),
            &mut rng,
            |k, _| format!("{k:?}"),
        );
        assert!(
            rendered == "Hello Jane Doe, enjoy a Apple and missing_unknown!"
                || rendered == "Hello Jane Doe, enjoy a Orange and missing_unknown!",
            "unexpected rendered string: {rendered}"
        );

        let mut copy = TemplateTextBuilder::new();
        copy.copy_from(&builder);
        let from_copy = copy.build("${maleName}", None, &mut rng, |k, _| match k {
            BuiltinNameKind::MaleName => "Bob".into(),
            _ => "Other".into(),
        });
        assert_eq!(from_copy, "Bob");
    }
}
