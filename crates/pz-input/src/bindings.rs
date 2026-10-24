//! Character gamepad bindings: `CharacterJoypadButtonBinding` (which button
//! or axis range triggers each action), `CharacterJoypadAxis2dBinding`
//! (which stick moves and aims) and `CharacterInputBindingSet`, the named
//! presets stored as `InputBindings/<name>.xml` in the user folder.
//!
//! Action names and their defaults come from the installed jar (the enums'
//! constructor arguments); joypad button and axis names are the
//! `JoypadButton`/`JoypadAxis1d`/`JoypadAxis2d` constants.

use std::path::{Path, PathBuf};

use crate::controller::pz_java_float;

/// `CharacterJoypadButtonBinding.IsDownBinding`.
#[derive(Clone, Debug, PartialEq)]
pub enum IsDown {
    Button(String),
    Axis1d(String, f32, f32),
    Axis2d(String, f32, f32),
}

impl IsDown {
    pub fn min(&self) -> f32 {
        match self {
            IsDown::Button(_) => 0.0,
            IsDown::Axis1d(_, m, _) | IsDown::Axis2d(_, m, _) => *m,
        }
    }

    pub fn max(&self) -> f32 {
        match self {
            IsDown::Button(_) => 0.0,
            IsDown::Axis1d(_, _, m) | IsDown::Axis2d(_, _, m) => *m,
        }
    }
}

/// One action binding's state.
#[derive(Clone, Debug)]
pub struct ButtonBinding {
    pub name: String,
    pub default: Option<IsDown>,
    pub binding: Option<IsDown>,
}

#[derive(Clone, Debug)]
pub struct Axis2dBinding {
    pub name: String,
    pub default: Option<String>,
    pub binding: Option<String>,
}

/// An entry of a binding set file.
#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    /// `<button binding="…" button="…"/>`; `button` absent means unbound.
    Button { binding: Option<String>, button: Option<String> },
    /// `<buttonAxis1d binding="…" axis="…" min="…" max="…"/>`.
    ButtonAxis1d { binding: Option<String>, axis: Option<String>, min: f32, max: f32 },
    /// `<buttonAxis2d …/>`.
    ButtonAxis2d { binding: Option<String>, axis: Option<String>, min: f32, max: f32 },
    /// `<axis2d binding="…" axis="…"/>`.
    Axis2d { binding: Option<String>, axis: Option<String> },
}

impl Entry {
    /// `isValid()`: both names known.
    fn is_valid(&self) -> bool {
        match self {
            Entry::Button { binding, button } => binding.is_some() && button.is_some(),
            Entry::ButtonAxis1d { binding, axis, .. } | Entry::ButtonAxis2d { binding, axis, .. } | Entry::Axis2d { binding, axis } => binding.is_some() && axis.is_some(),
        }
    }
}

/// `CharacterInputBindingSet`.
#[derive(Clone, Debug, PartialEq)]
pub struct BindingSet {
    pub name: String,
    pub description: Option<String>,
    pub entries: Vec<Entry>,
}

/// The static state of the three classes.
#[derive(Debug, Default)]
pub struct Bindings {
    pub buttons: Vec<ButtonBinding>,
    pub axes: Vec<Axis2dBinding>,
    /// Valid joypad constant names, to drop unknown ones on load as JAXB does.
    pub button_names: Vec<String>,
    pub axis1d_names: Vec<String>,
    pub axis2d_names: Vec<String>,
    /// `loadedBindingSets` (`None` until first read).
    pub loaded: Option<Vec<BindingSet>>,
    /// `getCacheDirSub("InputBindings")`.
    pub dir: PathBuf,
}

/// `StringUtils.equalsIgnoreCase`.
fn eq_ic(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

impl Bindings {
    pub fn button(&self, name: &str) -> Option<usize> {
        self.buttons.iter().position(|b| b.name == name)
    }

    pub fn axis(&self, name: &str) -> Option<usize> {
        self.axes.iter().position(|b| b.name == name)
    }

    /// `resetAllToDefault()`.
    pub fn reset_all_to_default(&mut self) {
        for b in &mut self.buttons {
            b.binding = b.default.clone();
        }
        for a in &mut self.axes {
            a.binding = a.default.clone();
        }
    }

    /// `CharacterInputBindingSet.apply()`.
    pub fn apply(&mut self, set: &BindingSet) {
        self.reset_all_to_default();
        for e in &set.entries {
            match e {
                Entry::Button { binding: Some(b), button } => {
                    if let Some(i) = self.button(b) {
                        self.buttons[i].binding = button.clone().map(IsDown::Button);
                    }
                }
                Entry::ButtonAxis1d { binding: Some(b), axis: Some(a), min, max } => {
                    if let Some(i) = self.button(b) {
                        self.buttons[i].binding = Some(IsDown::Axis1d(a.clone(), *min, *max));
                    }
                }
                Entry::ButtonAxis2d { binding: Some(b), axis: Some(a), min, max } => {
                    if let Some(i) = self.button(b) {
                        self.buttons[i].binding = Some(IsDown::Axis2d(a.clone(), *min, *max));
                    }
                }
                Entry::Axis2d { binding: Some(b), axis } => {
                    if let Some(i) = self.axis(b) {
                        self.axes[i].binding = axis.clone();
                    }
                }
                _ => {}
            }
        }
    }

    /// `setBindingsToCurrent()`: the current bindings as entries.
    pub fn current_entries(&self) -> Vec<Entry> {
        let mut out = Vec::new();
        for b in &self.buttons {
            let binding = Some(b.name.clone());
            out.push(match &b.binding {
                None => Entry::Button { binding, button: None },
                Some(IsDown::Button(n)) => Entry::Button { binding, button: Some(n.clone()) },
                Some(IsDown::Axis1d(a, min, max)) => Entry::ButtonAxis1d { binding, axis: Some(a.clone()), min: *min, max: *max },
                Some(IsDown::Axis2d(a, min, max)) => Entry::ButtonAxis2d { binding, axis: Some(a.clone()), min: *min, max: *max },
            });
        }
        for a in &self.axes {
            out.push(Entry::Axis2d { binding: Some(a.name.clone()), axis: a.binding.clone() });
        }
        out
    }

    /// `getLoadedBindingSets()`.
    pub fn loaded_sets(&mut self) -> &mut Vec<BindingSet> {
        if self.loaded.is_none() {
            self.reload_all();
        }
        self.loaded.as_mut().unwrap()
    }

    /// `reloadAll()`: every `*.xml` in the folder, duplicate names renamed
    /// `<name>_1`, `<name>_2`, …
    pub fn reload_all(&mut self) -> &mut Vec<BindingSet> {
        let mut sets: Vec<BindingSet> = Vec::new();
        let mut files: Vec<PathBuf> = std::fs::read_dir(&self.dir)
            .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_file() && p.to_string_lossy().ends_with(".xml")).collect())
            .unwrap_or_default();
        // `File.listFiles` order is the directory's; sort for determinism.
        files.sort();
        for f in files {
            let Some(mut set) = self.load_from_file(&f) else {
                eprintln!("CharacterInputBindingSet.Parse threw an exception reading file: {}", f.display());
                continue;
            };
            if sets.iter().any(|s| eq_ic(&s.name, &set.name)) {
                let base = set.name.clone();
                let mut alt = format!("{base}_1");
                let mut idx = 1;
                while sets.iter().any(|s| eq_ic(&s.name, &alt)) {
                    alt = format!("{base}_{idx}");
                    idx += 1;
                }
                set.name = alt;
            }
            sets.push(set);
        }
        self.loaded = Some(sets);
        self.loaded.as_mut().unwrap()
    }

    fn known(list: &[String], v: Option<&str>) -> Option<String> {
        v.filter(|v| list.iter().any(|n| n == v)).map(str::to_string)
    }

    fn load_from_file(&self, path: &Path) -> Option<BindingSet> {
        let text = std::fs::read_to_string(path).ok()?;
        let root = xml::parse(&text)?;
        if root.name != "inputBindingSet" {
            return None;
        }
        let mut set = BindingSet { name: path.file_stem()?.to_string_lossy().into_owned(), description: None, entries: Vec::new() };
        let button_names: Vec<String> = self.buttons.iter().map(|b| b.name.clone()).collect();
        let axis_binding_names: Vec<String> = self.axes.iter().map(|b| b.name.clone()).collect();
        let float = |v: Option<&str>, d: f32| v.and_then(|s| s.trim().parse::<f32>().ok()).unwrap_or(d);
        for el in &root.children {
            let attr = |k: &str| el.attr(k);
            let entry = match el.name.as_str() {
                "description" => {
                    set.description = Some(el.text.clone());
                    continue;
                }
                "button" => Entry::Button { binding: Self::known(&button_names, attr("binding")), button: Self::known(&self.button_names, attr("button")) },
                "buttonAxis1d" => Entry::ButtonAxis1d {
                    binding: Self::known(&button_names, attr("binding")),
                    axis: Self::known(&self.axis1d_names, attr("axis")),
                    min: float(attr("min"), 0.0),
                    max: float(attr("max"), f32::MAX),
                },
                "buttonAxis2d" => Entry::ButtonAxis2d {
                    binding: Self::known(&button_names, attr("binding")),
                    axis: Self::known(&self.axis2d_names, attr("axis")),
                    min: float(attr("min"), 0.0),
                    max: float(attr("max"), f32::MAX),
                },
                "axis2d" => Entry::Axis2d { binding: Self::known(&axis_binding_names, attr("binding")), axis: Self::known(&self.axis2d_names, attr("axis")) },
                _ => continue,
            };
            if entry.is_valid() {
                set.entries.push(entry);
            }
        }
        Some(set)
    }

    /// `save()`: `InputBindings/<name>.xml`.
    pub fn save(&self, set: &BindingSet) -> bool {
        if std::fs::create_dir_all(&self.dir).is_err() {
            eprintln!("Failed to create InputBindings folder: {}", self.dir.display());
            return false;
        }
        std::fs::write(self.dir.join(format!("{}.xml", set.name)), to_xml(set)).is_ok()
    }

    /// `saveAll()`: deletes the folder's `*.xml` files, then writes every
    /// loaded set.
    pub fn save_all(&mut self) -> bool {
        if std::fs::create_dir_all(&self.dir).is_err() {
            return false;
        }
        if let Ok(rd) = std::fs::read_dir(&self.dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_file() && p.to_string_lossy().ends_with(".xml") {
                    let _ = std::fs::remove_file(p);
                }
            }
        }
        let sets = self.loaded.clone().unwrap_or_default();
        let mut ok = true;
        for s in &sets {
            ok = self.save(s) && ok;
        }
        ok
    }

    /// `containsSetName(name)`.
    pub fn contains_set_name(&mut self, name: &str) -> bool {
        self.loaded_sets().iter().any(|s| eq_ic(&s.name, name))
    }

    /// `getUniqueSetName(name)`; `custom` is `Translator.getText("IGUI_Custom")`.
    pub fn unique_set_name(&mut self, name: Option<&str>, custom: &str) -> String {
        let name = match name {
            Some(n) if !n.chars().all(char::is_whitespace) => n,
            _ => custom,
        };
        let name = name.trim().to_string();
        if !self.contains_set_name(&name) {
            return name;
        }
        let base = trim_trailing_number(&name).to_string();
        let mut alt = format!("{base}_1");
        let mut idx = 1;
        while self.contains_set_name(&alt) {
            alt = format!("{base}_{idx}");
            idx += 1;
        }
        alt
    }

    /// `createNewFromCurrent(name)`: returns the new set's index.
    pub fn create_new_from_current(&mut self, name: Option<&str>, custom: &str) -> usize {
        let name = self.unique_set_name(name, custom);
        let entries = self.current_entries();
        let sets = self.loaded_sets();
        sets.push(BindingSet { name, description: None, entries });
        sets.len() - 1
    }
}

/// `trimTrailingNumberFromName`.
fn trim_trailing_number(name: &str) -> &str {
    match name.rfind('_') {
        Some(i) if name[i + 1..].trim().parse::<i32>().is_ok_and(|n| n != -1) => &name[..i],
        _ => name,
    }
}

/// The JAXB document as `PZXmlUtil.write` pretty-prints it (4-space
/// indent, no XML declaration).
pub fn to_xml(set: &BindingSet) -> String {
    let mut s = String::from("<inputBindingSet>\n");
    if let Some(d) = &set.description {
        s.push_str(&format!("    <description>{}</description>\n", xml::escape_text(d)));
    }
    let attr = |k: &str, v: &Option<String>| v.as_ref().map_or(String::new(), |v| format!(" {k}=\"{}\"", xml::escape_attr(v)));
    for e in &set.entries {
        let line = match e {
            Entry::Button { binding, button } => format!("<button{}{}/>", attr("binding", binding), attr("button", button)),
            Entry::ButtonAxis1d { binding, axis, min, max } => {
                format!("<buttonAxis1d{}{} min=\"{}\" max=\"{}\"/>", attr("binding", binding), attr("axis", axis), pz_java_float(*min), pz_java_float(*max))
            }
            Entry::ButtonAxis2d { binding, axis, min, max } => {
                format!("<buttonAxis2d{}{} min=\"{}\" max=\"{}\"/>", attr("binding", binding), attr("axis", axis), pz_java_float(*min), pz_java_float(*max))
            }
            Entry::Axis2d { binding, axis } => format!("<axis2d{}{}/>", attr("binding", binding), attr("axis", axis)),
        };
        s.push_str("    ");
        s.push_str(&line);
        s.push('\n');
    }
    s.push_str("</inputBindingSet>\n");
    s
}

/// A minimal XML reader for these files: elements, attributes, text and
/// the predefined and numeric entities.
pub mod xml {
    #[derive(Debug, Default)]
    pub struct Element {
        pub name: String,
        pub attrs: Vec<(String, String)>,
        pub text: String,
        pub children: Vec<Element>,
    }

    impl Element {
        pub fn attr(&self, k: &str) -> Option<&str> {
            self.attrs.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
        }
    }

    pub fn escape_text(s: &str) -> String {
        s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
    }

    pub fn escape_attr(s: &str) -> String {
        escape_text(s).replace('"', "&quot;")
    }

    fn unescape(s: &str) -> String {
        let mut out = String::new();
        let mut rest = s;
        while let Some(i) = rest.find('&') {
            out.push_str(&rest[..i]);
            let Some(j) = rest[i..].find(';') else {
                out.push_str(&rest[i..]);
                return out;
            };
            let ent = &rest[i + 1..i + j];
            let c = match ent {
                "lt" => Some('<'),
                "gt" => Some('>'),
                "amp" => Some('&'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ if ent.starts_with("#x") => u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32),
                _ if ent.starts_with('#') => ent[1..].parse().ok().and_then(char::from_u32),
                _ => None,
            };
            match c {
                Some(c) => out.push(c),
                None => out.push_str(&rest[i..i + j + 1]),
            }
            rest = &rest[i + j + 1..];
        }
        out.push_str(rest);
        out
    }

    /// Parses a document; `None` when it is not well formed.
    pub fn parse(text: &str) -> Option<Element> {
        let mut p = Parser { s: text, i: 0 };
        p.skip_prolog();
        let root = p.element()?;
        Some(root)
    }

    struct Parser<'a> {
        s: &'a str,
        i: usize,
    }

    impl Parser<'_> {
        fn rest(&self) -> &str {
            &self.s[self.i..]
        }

        fn skip_ws(&mut self) {
            let n = self.rest().len() - self.rest().trim_start().len();
            self.i += n;
        }

        fn skip_prolog(&mut self) {
            loop {
                self.skip_ws();
                if self.rest().starts_with("<?") {
                    match self.rest().find("?>") {
                        Some(e) => self.i += e + 2,
                        None => return,
                    }
                } else if self.rest().starts_with("<!--") {
                    match self.rest().find("-->") {
                        Some(e) => self.i += e + 3,
                        None => return,
                    }
                } else if self.rest().starts_with("<!") {
                    match self.rest().find('>') {
                        Some(e) => self.i += e + 1,
                        None => return,
                    }
                } else {
                    return;
                }
            }
        }

        fn name(&mut self) -> Option<String> {
            let r = self.rest();
            let n = r.find(|c: char| c.is_whitespace() || c == '>' || c == '/' || c == '=').unwrap_or(r.len());
            if n == 0 {
                return None;
            }
            let name = r[..n].to_string();
            self.i += n;
            Some(name)
        }

        fn element(&mut self) -> Option<Element> {
            if !self.rest().starts_with('<') {
                return None;
            }
            self.i += 1;
            let mut el = Element { name: self.name()?, ..Element::default() };
            loop {
                self.skip_ws();
                if self.rest().starts_with("/>") {
                    self.i += 2;
                    return Some(el);
                }
                if self.rest().starts_with('>') {
                    self.i += 1;
                    break;
                }
                let k = self.name()?;
                self.skip_ws();
                if !self.rest().starts_with('=') {
                    return None;
                }
                self.i += 1;
                self.skip_ws();
                let q = self.rest().chars().next()?;
                if q != '"' && q != '\'' {
                    return None;
                }
                self.i += 1;
                let end = self.rest().find(q)?;
                let v = unescape(&self.rest()[..end]);
                self.i += end + 1;
                el.attrs.push((k, v));
            }
            loop {
                let lt = self.rest().find('<')?;
                el.text.push_str(&unescape(&self.rest()[..lt]));
                self.i += lt;
                if self.rest().starts_with("</") {
                    self.i += 2;
                    let n = self.name()?;
                    if n != el.name {
                        return None;
                    }
                    self.skip_ws();
                    if !self.rest().starts_with('>') {
                        return None;
                    }
                    self.i += 1;
                    return Some(el);
                }
                if self.rest().starts_with("<!--") {
                    let e = self.rest().find("-->")?;
                    self.i += e + 3;
                    continue;
                }
                if self.rest().starts_with("<![CDATA[") {
                    let e = self.rest().find("]]>")?;
                    el.text.push_str(&self.rest()[9..e]);
                    self.i += e + 3;
                    continue;
                }
                let child = self.element()?;
                el.children.push(child);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(dir: &Path) -> Bindings {
        Bindings {
            buttons: vec![
                ButtonBinding { name: "Interact".into(), default: Some(IsDown::Button("A".into())), binding: Some(IsDown::Button("A".into())) },
                ButtonBinding { name: "Attack".into(), default: Some(IsDown::Axis1d("RightTrigger".into(), 0.93, f32::MAX)), binding: None },
                ButtonBinding { name: "WalkTo".into(), default: None, binding: None },
            ],
            axes: vec![Axis2dBinding { name: "Movement".into(), default: Some("LeftStick".into()), binding: Some("LeftStick".into()) }],
            button_names: vec!["A".into(), "B".into()],
            axis1d_names: vec!["RightTrigger".into()],
            axis2d_names: vec!["LeftStick".into(), "RightStick".into()],
            loaded: None,
            dir: dir.to_path_buf(),
        }
    }

    #[test]
    fn create_save_reload_apply() {
        let dir = std::env::temp_dir().join(format!("pz-input-bindings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut b = sample(&dir);
        b.reset_all_to_default();
        b.buttons[0].binding = Some(IsDown::Button("B".into()));
        b.axes[0].binding = Some("RightStick".into());
        let i = b.create_new_from_current(None, "Custom");
        assert_eq!(b.loaded.as_ref().unwrap()[i].name, "Custom");
        assert!(b.save_all());
        let xml = std::fs::read_to_string(dir.join("Custom.xml")).unwrap();
        assert!(xml.contains("<button binding=\"Interact\" button=\"B\"/>"), "{xml}");
        assert!(xml.contains("max=\"3.4028235E38\""), "{xml}");
        assert!(xml.contains("<button binding=\"WalkTo\"/>"), "{xml}");
        assert_eq!(b.unique_set_name(Some("Custom"), "Custom"), "Custom_1");
        b.reset_all_to_default();
        let sets = b.reload_all().clone();
        assert_eq!(sets[0].entries.len(), 3, "WalkTo without a button is dropped");
        b.apply(&sets[0]);
        assert_eq!(b.buttons[0].binding, Some(IsDown::Button("B".into())));
        assert_eq!(b.axes[0].binding.as_deref(), Some("RightStick"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn xml_reader() {
        let e = xml::parse("<?xml version=\"1.0\"?><!-- c --><a x='1 &amp; 2'><description>Hi &lt;3</description><b/></a>").unwrap();
        assert_eq!(e.attr("x"), Some("1 & 2"));
        assert_eq!(e.children[0].text, "Hi <3");
        assert_eq!(e.children[1].name, "b");
        assert!(xml::parse("<a><b></a>").is_none());
        assert_eq!(trim_trailing_number("Custom_12"), "Custom");
        assert_eq!(trim_trailing_number("My_Set"), "My_Set");
    }
}
