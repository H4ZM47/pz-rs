//! `zombie.characters.skills.PerkFactory`, with its nested `Perk` and
//! `Perks`.
//!
//! The perk set is not hard-coded: [`PerkFactory::from_tables`] replays the
//! `PerkFactory.Perks` static initializer and [`PerkFactory::init`] replays
//! `PerkFactory.init()`, both as read from the installed jar into
//! [`BuiltinTables`]. TODO(CustomPerks): mods' `media/perks.txt`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use pz_scripts::registry::builtin::{BuiltinTables, PerkInit};
use pz_scripts::{ScriptError, ScriptResult};

/// `PerkFactory.PerkByIndex.length`.
pub const MAX_PERKS: usize = 256;

/// `PerkFactory.PERK_XP_REQ_MULTIPLIER`.
const PERK_XP_REQ_MULTIPLIER: f32 = 1.5;

/// The `Perks` constant every `Perk` defaults its parent to.
const NONE_FIELD: &str = "None";
/// The `Perks` constant past the last built-in perk.
const MAX_FIELD: &str = "MAX";

/// `PerkFactory.Perk`.
#[derive(Debug)]
pub struct Perk {
    id: String,
    index: i32,
    custom: bool,
    /// `translation`: the `IGUI_perks_` key suffix.
    pub translation: String,
    /// `name`: the translated name.
    pub name: String,
    /// `passiv`.
    pub passiv: bool,
    /// `xp1`..`xp10`.
    pub xp: [i32; 10],
    /// `parent` (`null` only for `Perks.None` before `init`).
    pub parent: Option<PerkRef>,
}

pub type PerkRef = Rc<RefCell<Perk>>;

impl Perk {
    /// `getId` (also `toString`).
    pub fn id(&self) -> &str {
        &self.id
    }

    /// `index()`.
    pub fn index(&self) -> i32 {
        self.index
    }

    /// `setCustom`.
    pub fn set_custom(&mut self) {
        self.custom = true;
    }

    /// `isCustom`.
    pub fn is_custom(&self) -> bool {
        self.custom
    }

    /// `isPassiv`.
    pub fn is_passiv(&self) -> bool {
        self.passiv
    }

    /// `getName`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// `getXp1`..`getXp10` (`n` in 1..=10).
    pub fn xp_n(&self, n: usize) -> i32 {
        self.xp[n - 1]
    }

    /// `getXpForLevel`: -1 outside 1..=10.
    pub fn xp_for_level(&self, level: i32) -> f32 {
        match level {
            1..=10 => self.xp[level as usize - 1] as f32,
            _ => -1.0,
        }
    }

    /// `getTotalXpForLevel`: the int sum of the levels up to `level`.
    pub fn total_xp_for_level(&self, level: i32) -> f32 {
        let mut total: i32 = 0;
        for i in 1..=level {
            let xp = self.xp_for_level(i);
            if xp != -1.0 {
                total = (total as f32 + xp) as i32;
            }
        }
        total as f32
    }
}

/// `PerkFactory`'s statics plus the `Perks` constants.
#[derive(Debug, Default)]
pub struct PerkFactory {
    /// `PerkList`: perks added by `AddPerk`, in order.
    pub perk_list: Vec<PerkRef>,
    perk_by_id: HashMap<String, PerkRef>,
    perk_by_name: HashMap<String, PerkRef>,
    perk_by_index: Vec<Option<PerkRef>>,
    next_perk_id: i32,
    /// The `Perks` constants: (field, perk), in declaration order.
    pub perks: Vec<(String, PerkRef)>,
    none: Option<PerkRef>,
    max: Option<PerkRef>,
}

fn array_index_error(index: i32) -> ScriptError {
    ScriptError::new(format!(
        "java.lang.ArrayIndexOutOfBoundsException: Index {index} out of bounds for length {MAX_PERKS}"
    ))
}

impl PerkFactory {
    /// Runs the `Perks` static initializer described by `tables`.
    pub fn from_tables(tables: &BuiltinTables) -> ScriptResult<Self> {
        let mut f = PerkFactory::default();
        for decl in &tables.perks {
            let parent = match &decl.parent {
                Some(p) => Some(f.constant(p).ok_or_else(|| ScriptError::new("java.lang.NullPointerException"))?),
                None => None,
            };
            let perk = f.new_perk(&decl.id)?;
            if parent.is_some() {
                perk.borrow_mut().parent = parent;
            }
            match decl.field.as_str() {
                NONE_FIELD => f.none = Some(perk.clone()),
                MAX_FIELD => f.max = Some(perk.clone()),
                _ => {}
            }
            f.perks.push((decl.field.clone(), perk));
        }
        Ok(f)
    }

    /// The `Perks` constant named `field`.
    pub fn constant(&self, field: &str) -> Option<PerkRef> {
        self.perks.iter().find(|(f, _)| f == field).map(|(_, p)| p.clone())
    }

    /// `Perks.None`.
    pub fn none(&self) -> Option<PerkRef> {
        self.none.clone()
    }

    /// `Perks.MAX`.
    pub fn max(&self) -> Option<PerkRef> {
        self.max.clone()
    }

    /// `new Perk(id)`: takes the next index and registers the id.
    pub fn new_perk(&mut self, id: &str) -> ScriptResult<PerkRef> {
        let index = self.next_perk_id;
        if index < 0 || index as usize >= MAX_PERKS {
            return Err(array_index_error(index));
        }
        self.next_perk_id += 1;
        let perk = Rc::new(RefCell::new(Perk {
            id: id.to_string(),
            index,
            custom: false,
            translation: id.to_string(),
            name: id.to_string(),
            passiv: false,
            xp: [0; 10],
            parent: self.none.clone(),
        }));
        self.perk_by_id.insert(id.to_string(), perk.clone());
        if self.perk_by_index.len() < MAX_PERKS {
            self.perk_by_index.resize(MAX_PERKS, None);
        }
        self.perk_by_index[index as usize] = Some(perk.clone());
        if let Some(max) = &self.max {
            let mut max = max.borrow_mut();
            max.index = max.index.max(index + 1);
        }
        Ok(perk)
    }

    /// `new Perk(id, parent)`.
    pub fn new_perk_with_parent(&mut self, id: &str, parent: Option<PerkRef>) -> ScriptResult<PerkRef> {
        let perk = self.new_perk(id)?;
        perk.borrow_mut().parent = parent;
        Ok(perk)
    }

    /// `getPerkName`.
    pub fn perk_name(perk: &PerkRef) -> String {
        perk.borrow().name.clone()
    }

    /// `getPerkFromName`.
    pub fn perk_from_name(&self, name: &str) -> Option<PerkRef> {
        self.perk_by_name.get(name).cloned()
    }

    /// `AddPerk` (all overloads; the ones without a parent pass
    /// `Perks.None`, the ones without `passiv` pass false). `get_text` is
    /// `Translator.getText`.
    #[allow(clippy::too_many_arguments)]
    pub fn add_perk(
        &mut self,
        perk: &PerkRef,
        translation: &str,
        parent: Option<PerkRef>,
        xp: [i32; 10],
        passiv: bool,
        get_text: &dyn Fn(&str) -> String,
    ) -> PerkRef {
        let name = {
            let mut p = perk.borrow_mut();
            p.translation = translation.to_string();
            p.name = get_text(&format!("IGUI_perks_{translation}"));
            p.parent = parent;
            p.passiv = passiv;
            for (dst, src) in p.xp.iter_mut().zip(xp) {
                *dst = (src as f32 * PERK_XP_REQ_MULTIPLIER) as i32;
            }
            p.name.clone()
        };
        self.perk_by_name.insert(name, perk.clone());
        self.perk_list.push(perk.clone());
        perk.clone()
    }

    /// `init`: replays `PerkFactory.init()` as read from the jar.
    pub fn init(&mut self, tables: &BuiltinTables, get_text: &dyn Fn(&str) -> String) -> ScriptResult {
        let get = |f: &Self, field: &str| {
            f.constant(field)
                .ok_or_else(|| ScriptError::new(format!("java.lang.NullPointerException: Perks.{field}")))
        };
        for step in &tables.perk_init {
            match step {
                PerkInit::SetParent { perk, parent } => {
                    let parent = get(self, parent)?;
                    get(self, perk)?.borrow_mut().parent = Some(parent);
                }
                PerkInit::AddPerk { perk, translation, parent, xp, passiv } => {
                    let perk = get(self, perk)?;
                    let parent = match parent {
                        Some(p) => Some(get(self, p)?),
                        None => self.none.clone(),
                    };
                    self.add_perk(&perk, translation, parent, *xp, *passiv, get_text);
                }
            }
        }
        Ok(())
    }

    /// `initTranslations`: re-translates every listed perk's name.
    pub fn init_translations(&mut self, get_text: &dyn Fn(&str) -> String) {
        self.perk_by_name.clear();
        for perk in &self.perk_list {
            let name = {
                let mut p = perk.borrow_mut();
                p.name = get_text(&format!("IGUI_perks_{}", p.translation));
                p.name.clone()
            };
            self.perk_by_name.insert(name, perk.clone());
        }
    }

    /// `Reset`: drops custom perks and rewinds the next index past the last
    /// built-in one (ignoring `MAX`).
    pub fn reset(&mut self) {
        self.next_perk_id = 0;
        for i in (0..self.perk_by_index.len()).rev() {
            let Some(perk) = self.perk_by_index[i].clone() else { continue };
            if perk.borrow().is_custom() {
                self.perk_list.retain(|p| !Rc::ptr_eq(p, &perk));
                let p = perk.borrow();
                self.perk_by_id.remove(&p.id);
                self.perk_by_name.remove(&p.name);
                self.perk_by_index[p.index as usize] = None;
                continue;
            }
            let is_max = self.max.as_ref().is_some_and(|m| Rc::ptr_eq(m, &perk));
            if !is_max && self.next_perk_id == 0 {
                self.next_perk_id = i as i32 + 1;
            }
        }
        if let Some(max) = &self.max {
            max.borrow_mut().index = self.next_perk_id;
        }
    }

    /// `Perks.getMaxIndex`.
    pub fn max_index(&self) -> i32 {
        self.max.as_ref().map_or(0, |m| m.borrow().index)
    }

    /// `Perks.fromIndex`.
    pub fn from_index(&self, value: i32) -> ScriptResult<Option<PerkRef>> {
        if value < 0 || value > self.next_perk_id {
            return Ok(None);
        }
        if value as usize >= MAX_PERKS {
            return Err(array_index_error(value));
        }
        Ok(self.perk_by_index.get(value as usize).cloned().flatten())
    }

    /// `Perks.FromString`: the perk with `id`, else `Perks.MAX`.
    pub fn from_string(&self, id: &str) -> Option<PerkRef> {
        self.perk_by_id.get(id).cloned().or_else(|| self.max.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_scripts::registry::builtin::PerkDecl;

    fn tables() -> BuiltinTables {
        let decl = |field: &str| PerkDecl { field: field.into(), id: field.into(), parent: None };
        let xp = [1, 2, 3, 4, 5, 6, 7, 8, 9, 11];
        BuiltinTables {
            perks: vec![decl("None"), decl("Digging"), decl("Outdoors"), decl("Shovel"), decl("MAX")],
            perk_init: vec![
                PerkInit::SetParent { perk: "None".into(), parent: "None".into() },
                PerkInit::SetParent { perk: "MAX".into(), parent: "None".into() },
                PerkInit::AddPerk { perk: "Outdoors".into(), translation: "Out".into(), parent: None, xp, passiv: false },
                PerkInit::AddPerk {
                    perk: "Shovel".into(),
                    translation: "Shov".into(),
                    parent: Some("Outdoors".into()),
                    xp,
                    passiv: true,
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn builds_and_inits_from_tables() {
        let t = tables();
        let mut f = PerkFactory::from_tables(&t).unwrap();
        let none = f.none().unwrap();
        assert!(none.borrow().parent.is_none());
        assert_eq!(f.max_index(), 4);
        f.init(&t, &|k| format!("<{k}>")).unwrap();
        assert!(Rc::ptr_eq(none.borrow().parent.as_ref().unwrap(), &none));
        assert_eq!(f.perk_list.len(), 2);
        let shovel = f.perk_from_name("<IGUI_perks_Shov>").unwrap();
        assert_eq!(shovel.borrow().id(), "Shovel");
        assert!(shovel.borrow().is_passiv());
        assert_eq!(shovel.borrow().parent.as_ref().unwrap().borrow().id(), "Outdoors");
        let out = f.constant("Outdoors").unwrap();
        assert!(Rc::ptr_eq(out.borrow().parent.as_ref().unwrap(), &none));
        let p = shovel.borrow();
        // (int)(xp * 1.5f)
        assert_eq!(p.xp, [1, 3, 4, 6, 7, 9, 10, 12, 13, 16]);
        assert_eq!(p.xp_for_level(0), -1.0);
        assert_eq!(p.xp_for_level(10), 16.0);
        assert_eq!(p.total_xp_for_level(3), 8.0);
        assert_eq!(p.total_xp_for_level(99), 81.0);
        drop(p);
        assert!(f.from_string("Nope").is_some_and(|m| m.borrow().id() == "MAX"));
        assert_eq!(f.from_index(1).unwrap().unwrap().borrow().id(), "Digging");
        assert!(f.from_index(6).unwrap().is_none());
        f.init_translations(&|k| k.to_lowercase());
        assert!(f.perk_from_name("igui_perks_out").is_some());
        assert!(f.perk_from_name("<IGUI_perks_Out>").is_none());
    }

    #[test]
    fn custom_perks_and_reset() {
        let t = tables();
        let mut f = PerkFactory::from_tables(&t).unwrap();
        f.init(&t, &|k| k.to_string()).unwrap();
        let c = f.new_perk("Juggling").unwrap();
        c.borrow_mut().set_custom();
        assert_eq!(c.borrow().index(), 5);
        assert_eq!(f.max_index(), 6);
        let parent = f.none();
        f.add_perk(&c, "Jug", parent, [1; 10], false, &|k| k.to_string());
        assert_eq!(f.perk_list.len(), 3);
        f.reset();
        assert_eq!(f.perk_list.len(), 2);
        assert!(f.from_string("Juggling").is_some_and(|m| m.borrow().id() == "MAX"));
        assert!(f.perk_from_name("IGUI_perks_Jug").is_none());
        // MAX is skipped: the next index follows the last built-in perk.
        assert_eq!(f.max_index(), 4);
        assert_eq!(f.new_perk("Again").unwrap().borrow().index(), 4);
    }
}
