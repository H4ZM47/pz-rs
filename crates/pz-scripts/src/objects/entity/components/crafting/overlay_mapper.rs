//! `zombie.entity.components.crafting.recipe.OverlayMapper`: picks a sprite
//! overlay style from the item used for one input.

use crate::base::{ScriptRef, ScriptResult};
use crate::java;
use crate::manager::ScriptManager;
use crate::objects::entity::exception;
use crate::script_type::ScriptType;

#[derive(Clone, Debug, Default)]
pub struct OverlayMapper {
    default_styles: Vec<String>,
    /// `styleMap`: item to style, in insertion order.
    style_map: Vec<(ScriptRef, String)>,
    /// Index into the recipe's `ioLines` of the registered input.
    input_script: Option<usize>,
}

impl OverlayMapper {
    /// `clear`.
    pub fn clear(&mut self) {
        *self = OverlayMapper::default();
    }

    /// `setDefaultOverlayStyle`: `;`-separated, empty entries skipped.
    pub fn set_default_overlay_style(&mut self, style: &str) {
        for v in java::split_char(style, ';') {
            if !v.is_empty() {
                self.default_styles.push(v.to_string());
            }
        }
    }

    pub fn default_styles(&self) -> &[String] {
        &self.default_styles
    }

    pub fn style_map(&self) -> &[(ScriptRef, String)] {
        &self.style_map
    }

    /// `addOverlayStyleEntry`: maps each named item to the style.
    pub fn add_overlay_style_entry(&mut self, sm: &ScriptManager, style: &str, items: &[&str]) -> ScriptResult {
        for &item_name in items {
            let item = sm
                .get_script(ScriptType::Item, item_name)
                .ok_or_else(|| exception(format!("OverlayMapper item not found: {item_name}")))?;
            if let Some((_, existing)) = self.style_map.iter().find(|(i, _)| i.ptr_eq(&item)) {
                return Err(exception(format!(
                    "OverlayMapper item {item_name} already has style mapping: {existing}. Cannot assign style: {style}"
                )));
            }
            self.style_map.push((item, style.to_string()));
        }
        Ok(())
    }

    /// `registerInputScript`.
    pub fn register_input_script(&mut self, io_index: usize) {
        self.input_script = Some(io_index);
    }

    pub fn input_script(&self) -> Option<usize> {
        self.input_script
    }

    /// `isEmpty`.
    pub fn is_empty(&self) -> bool {
        self.default_styles.is_empty() && self.style_map.is_empty()
    }

    /// `getFirstAvailableDefaultStyle`: the first default style that is the
    /// prefix (before `_`) of an available style.
    pub fn first_available_default_style(&self, available_styles: &[String]) -> Option<&str> {
        self.default_styles
            .iter()
            .find(|style| available_styles.iter().any(|a| java::split_char(a, '_').first().is_some_and(|p| p == style)))
            .map(String::as_str)
    }
    // TODO(CraftRecipeData): `getStyle` reads the items applied to the
    // registered input at craft time.
}
