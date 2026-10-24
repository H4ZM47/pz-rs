//! `zombie.entity.components.crafting`: `CraftLogic` and `DryingCraftLogic`
//! with their in-progress `CraftRecipeData`.

use pz_scripts::objects::entity::components::crafting::StartMode;

use crate::context::{EntityContext, Lookup};
use crate::io::{block_load, read_header, HeaderSize, HeaderWrite, Reader, Result, Writer};

/// One in-progress `CraftRecipeData` (`save`'s `ByteBlock`). The recipe
/// header is decoded; the inputs (fluid samples and inventory items) stay
/// as saved bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct CraftRecipeData {
    /// The recipe's full type and `getScriptVersion()`, when it has one.
    pub recipe: Option<(String, i64)>,
    pub elapsed_time: f64,
    /// Everything after `elapsedTime`: the input count, inputs, mod data,
    /// `hasConsumedInputs` and `targetVariableInputRatio`.
    pub rest: Vec<u8>,
}

impl CraftRecipeData {
    /// `CraftRecipeData.load` for world version 238 and up. `Ok(None)` when
    /// the original invalidates the craft (recipe missing, its version or
    /// input count changed).
    pub fn load(r: &mut Reader<'_>, ctx: &dyn EntityContext) -> Result<Option<CraftRecipeData>> {
        let (_, len) = block_load(r)?;
        let body = r.bytes(len)?;
        let mut b = Reader::new(body);
        let recipe = if b.bool()? {
            Some((b.string()?, b.i64()?))
        } else {
            None
        };
        let elapsed_time = b.f64()?;
        let rest = body[b.pos()..].to_vec();
        if let Some((name, version)) = &recipe {
            match ctx.craft_recipe(name) {
                Lookup::Missing => return Ok(None),
                Lookup::Found(info) => {
                    let inputs = Reader::new(&rest).i32()?;
                    if info.script_version != *version || inputs as usize != info.input_count {
                        return Ok(None);
                    }
                }
                Lookup::Unknown => {}
            }
        }
        Ok(Some(CraftRecipeData {
            recipe,
            elapsed_time,
            rest,
        }))
    }

    pub fn save(&self, w: &mut Writer) {
        let start = w.block_start();
        w.bool(self.recipe.is_some());
        if let Some((name, version)) = &self.recipe {
            w.string(name);
            w.i64(*version);
        }
        w.f64(self.elapsed_time);
        w.bytes(&self.rest);
        w.block_end(start);
    }
}

/// `zombie.entity.components.crafting.CraftLogic`'s saved state.
#[derive(Clone, Debug, PartialEq)]
pub struct CraftLogic {
    pub recipe_tag_query: Option<String>,
    pub start_mode: StartMode,
    pub inputs_group_name: Option<String>,
    pub outputs_group_name: Option<String>,
    /// `craftDataInProgress`; non-empty means `isRunning()`.
    pub in_progress: Vec<CraftRecipeData>,
    /// Not reset by `load`: kept when the save has none.
    pub action_anim_override: Option<String>,
}

impl Default for CraftLogic {
    fn default() -> Self {
        CraftLogic {
            recipe_tag_query: None,
            start_mode: StartMode::Manual,
            inputs_group_name: None,
            outputs_group_name: None,
            in_progress: Vec::new(),
            action_anim_override: None,
        }
    }
}

fn start_mode_from_byte(b: i8) -> Option<StartMode> {
    StartMode::ALL.iter().copied().find(|m| m.byte_id() == b)
}

impl CraftLogic {
    pub fn load(&mut self, r: &mut Reader<'_>, wv: i32, ctx: &dyn EntityContext) -> Result<()> {
        let h = read_header(r, HeaderSize::Byte)?;
        self.recipe_tag_query = None;
        self.start_mode = StartMode::Manual;
        self.inputs_group_name = None;
        self.outputs_group_name = None;
        if h & 1 != 0 {
            self.recipe_tag_query = Some(r.string()?);
        }
        if h & 2 != 0 {
            let b = r.i8()?;
            match start_mode_from_byte(b) {
                Some(m) => self.start_mode = m,
                None => return crate::io::err(format!("StartMode byte id {b} not found")),
            }
        }
        if h & 4 != 0 {
            self.inputs_group_name = Some(r.string()?);
        }
        if h & 8 != 0 {
            self.outputs_group_name = Some(r.string()?);
        }
        if h & 16 != 0 {
            self.load_in_progress(r, wv, ctx)?;
        } else {
            self.in_progress.clear();
        }
        if h & 32 != 0 {
            self.action_anim_override = Some(r.string()?);
        }
        Ok(())
    }

    /// `loadInProgressCraftData`: once one craft is invalidated, the rest
    /// are dropped too.
    fn load_in_progress(
        &mut self,
        r: &mut Reader<'_>,
        wv: i32,
        ctx: &dyn EntityContext,
    ) -> Result<()> {
        if wv < 238 {
            return crate::io::err("CraftRecipeData before world version 238 is not supported");
        }
        let n = r.i32()?;
        let mut out = Vec::new();
        let mut invalidated = false;
        for _ in 0..n.max(0) {
            if invalidated {
                let (_, len) = block_load(r)?;
                r.bytes(len)?;
                continue;
            }
            match CraftRecipeData::load(r, ctx)? {
                Some(d) => out.push(d),
                None => invalidated = true,
            }
        }
        self.in_progress = out;
        Ok(())
    }

    pub fn save(&self, w: &mut Writer) {
        let mut h = HeaderWrite::start(w, HeaderSize::Byte);
        if let Some(s) = &self.recipe_tag_query {
            h.add(1);
            w.string(s);
        }
        if self.start_mode != StartMode::Manual {
            h.add(2);
            w.i8(self.start_mode.byte_id());
        }
        if let Some(s) = &self.inputs_group_name {
            h.add(4);
            w.string(s);
        }
        if let Some(s) = &self.outputs_group_name {
            h.add(8);
            w.string(s);
        }
        if !self.in_progress.is_empty() {
            h.add(16);
            w.i32(self.in_progress.len() as i32);
            for d in &self.in_progress {
                d.save(w);
            }
        }
        if let Some(s) = &self.action_anim_override {
            h.add(32);
            w.string(s);
        }
        h.write(w);
    }
}

/// `zombie.entity.components.crafting.DryingCraftLogic`.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct DryingCraftLogic {
    pub logic: CraftLogic,
    /// `temporaryWetnesses` per in-progress craft (same order).
    pub wetness: Vec<f64>,
}

impl DryingCraftLogic {
    /// A wetness count that does not match the in-progress crafts zeroes
    /// the wetness values (the doubles are left unread).
    pub fn load(&mut self, r: &mut Reader<'_>, wv: i32, ctx: &dyn EntityContext) -> Result<()> {
        self.logic.load(r, wv, ctx)?;
        let n = r.i32()?;
        let k = self.logic.in_progress.len();
        self.wetness = vec![0.0; k];
        if n >= 0 && n as usize == k {
            for v in &mut self.wetness {
                *v = r.f64()?;
            }
        }
        Ok(())
    }

    pub fn save(&self, w: &mut Writer) {
        self.logic.save(w);
        w.i32(self.logic.in_progress.len() as i32);
        for i in 0..self.logic.in_progress.len() {
            w.f64(self.wetness.get(i).copied().unwrap_or(0.0));
        }
    }
}
