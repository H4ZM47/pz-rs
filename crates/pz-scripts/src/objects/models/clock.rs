//! `zombie.scripting.objects.ClockScript`.

use crate::base::{index_error, BaseScriptObject, ScriptObject, ScriptResult};
use crate::java;
use crate::manager::ScriptManager;
use crate::parser::Block;
use crate::script_type::ScriptType;

use super::{discard_null_or_whitespace, try_parse_bool, try_parse_float};

/// `ClockScript.HandScript`.
#[derive(Clone, Debug, PartialEq)]
pub struct HandScript {
    pub length: f32,
    pub thickness: f32,
    pub texture: Option<String>,
    pub texture_axis_x: f32,
    pub texture_axis_y: f32,
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Default for HandScript {
    fn default() -> Self {
        HandScript {
            length: 1.0,
            thickness: 0.1,
            texture: None,
            texture_axis_x: f32::NAN,
            texture_axis_y: f32::NAN,
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        }
    }
}

#[derive(Debug)]
pub struct ClockScript {
    pub base: BaseScriptObject,
    pub replacement_sprite: Option<String>,
    pub hand_x: f32,
    pub hand_y: f32,
    pub hand_z: f32,
    pub north: bool,
    pub hour_hand: HandScript,
    pub minute_hand: HandScript,
    pub replacement_only: bool,
}

impl Default for ClockScript {
    fn default() -> Self {
        Self::new()
    }
}

impl ClockScript {
    pub fn new() -> Self {
        ClockScript {
            base: BaseScriptObject::new(ScriptType::Clock),
            replacement_sprite: None,
            hand_x: 0.0,
            hand_y: 0.0,
            hand_z: 0.0,
            north: false,
            hour_hand: HandScript::default(),
            minute_hand: HandScript::default(),
            replacement_only: false,
        }
    }
}

/// `LoadHand`.
fn load_hand(block: &Block, hand: &mut HandScript) {
    hand.length = 1.0;
    hand.thickness = 0.1;
    hand.texture = Some("white.png".to_string());
    hand.texture_axis_y = f32::NAN;
    hand.texture_axis_x = f32::NAN;
    hand.a = 1.0;
    hand.b = 1.0;
    hand.g = 1.0;
    hand.r = 1.0;
    for value in &block.values {
        let k = java::trim(value.get_key());
        let v = java::trim(value.get_value());
        let is = |s: &str| java::eq_ignore_case(k, s);
        if is("length") {
            hand.length = try_parse_float(v, 1.0);
        } else if is("texture") {
            if let Some(t) = discard_null_or_whitespace(v) {
                hand.texture = Some(format!("media/textures/{t}.png"));
            }
        } else if is("textureInfo") {
            let ss = java::split_ws(v);
            if ss.len() != 4 {
                continue;
            }
            let tex_w = try_parse_float(ss[0], 128.0);
            let tex_h = try_parse_float(ss[1], 128.0);
            let axis_x = try_parse_float(ss[2], 0.5);
            let axis_y = try_parse_float(ss[3], 0.5);
            hand.texture_axis_x = axis_x / tex_w;
            hand.texture_axis_y = axis_y / tex_h;
        } else if is("thickness") {
            hand.thickness = try_parse_float(v, 0.1);
        } else if is("rgba") {
            let ss = java::split_ws(v);
            if ss.len() != 4 {
                continue;
            }
            hand.r = try_parse_float(ss[0], 1.0);
            hand.g = try_parse_float(ss[1], 1.0);
            hand.b = try_parse_float(ss[2], 1.0);
            hand.a = try_parse_float(ss[3], 1.0);
        }
    }
}

impl ScriptObject for ClockScript {
    fn base(&self) -> &BaseScriptObject {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseScriptObject {
        &mut self.base
    }

    fn class_name(&self) -> &'static str {
        "zombie.scripting.objects.ClockScript"
    }

    fn load(&mut self, sm: &ScriptManager, _name: &str, body: &str) -> ScriptResult {
        self.replacement_sprite = None;
        self.hand_z = 0.0;
        self.hand_y = 0.0;
        self.hand_x = 0.0;
        self.north = false;
        self.replacement_only = false;
        let root = sm.parse(body);
        let block = root.children.first().ok_or_else(index_error)?;
        self.base.load_common_block(block);
        let mut replacement_only = block.children.is_empty();
        for value in &block.values {
            let k = java::trim(value.get_key());
            let v = java::trim(value.get_value());
            if java::eq_ignore_case(k, "handOffset") {
                let ss = java::split_ws(v);
                if ss.len() == 3 {
                    self.hand_x = try_parse_float(ss[0], 0.0);
                    self.hand_y = try_parse_float(ss[1], 0.0);
                    self.hand_z = try_parse_float(ss[2], 0.0);
                }
                replacement_only = false;
            } else if java::eq_ignore_case(k, "north") {
                self.north = try_parse_bool(v);
                replacement_only = false;
            } else if java::eq_ignore_case(k, "replacementSprite") {
                self.replacement_sprite = discard_null_or_whitespace(v);
            }
        }
        if replacement_only {
            self.replacement_only = true;
            return Ok(());
        }
        for child in &block.children {
            if !java::eq_ignore_case(child.type_str(), "hand") {
                continue;
            }
            let id = child.id.as_deref();
            if id.is_some_and(|id| java::eq_ignore_case("hour", id)) {
                load_hand(child, &mut self.hour_hand);
            }
            if id.is_some_and(|id| java::eq_ignore_case("minute", id)) {
                load_hand(child, &mut self.minute_hand);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_util::load;
    use super::*;

    #[test]
    fn loads_hands() {
        let body = "clock C { handOffset = 1 2 3, north = true, \
                    hand hour { length = 0.5, texture = h, textureInfo = 64 32 16 8, rgba = 1 0 0 1, } \
                    hand minute { thickness = x, } }";
        let (c, r) = load(ClockScript::new(), "C", body);
        r.unwrap();
        assert!(c.north && !c.replacement_only);
        assert_eq!((c.hand_x, c.hand_y, c.hand_z), (1.0, 2.0, 3.0));
        assert_eq!(c.hour_hand.texture.as_deref(), Some("media/textures/h.png"));
        assert_eq!(c.hour_hand.texture_axis_x, 0.25);
        assert_eq!(c.hour_hand.g, 0.0);
        assert_eq!(c.minute_hand.thickness, 0.1);
        assert_eq!(c.minute_hand.texture.as_deref(), Some("white.png"));
    }

    #[test]
    fn replacement_only_without_hands() {
        let (c, _) = load(ClockScript::new(), "C", "clock C { replacementSprite = s_0, }");
        assert!(c.replacement_only);
        assert_eq!(c.replacement_sprite.as_deref(), Some("s_0"));
        assert_eq!(c.hour_hand.texture, None);
    }
}
