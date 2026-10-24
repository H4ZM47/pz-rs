//! `zombie.core.skinnedmodel.runtime.RuntimeAnimationScript` and its
//! commands (`CopyFrame`, `CopyFrames`).

use crate::base::{index_error, BaseScriptObject, ScriptObject, ScriptResult};
use crate::java;
use crate::manager::ScriptManager;
use crate::parser::Block;
use crate::script_type::ScriptType;

use super::try_parse_int;

/// `CopyFrame`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CopyFrame {
    pub frame: i32,
    pub fps: i32,
    pub source: Option<String>,
    pub source_frame: i32,
    pub source_fps: i32,
}

impl Default for CopyFrame {
    fn default() -> Self {
        CopyFrame { frame: 0, fps: 30, source: None, source_frame: 0, source_fps: 30 }
    }
}

impl CopyFrame {
    /// `parse`.
    pub fn parse(&mut self, block: &Block) {
        for value in &block.values {
            let k = java::trim(value.get_key());
            let v = java::trim(value.get_value());
            if java::eq_ignore_case(k, "source") {
                self.source = Some(v.to_string());
            } else if java::eq_ignore_case(k, "frame") {
                self.frame = try_parse_int(v, 1);
            } else if java::eq_ignore_case(k, "sourceFrame") {
                self.source_frame = try_parse_int(v, 1);
            }
        }
    }
}

/// `CopyFrames`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CopyFrames {
    pub frame: i32,
    pub fps: i32,
    pub source: Option<String>,
    pub source_frame1: i32,
    pub source_frame2: i32,
    pub source_fps: i32,
}

impl Default for CopyFrames {
    fn default() -> Self {
        CopyFrames { frame: 0, fps: 30, source: None, source_frame1: 0, source_frame2: 0, source_fps: 30 }
    }
}

impl CopyFrames {
    /// `parse`.
    pub fn parse(&mut self, block: &Block) {
        for value in &block.values {
            let k = java::trim(value.get_key());
            let v = java::trim(value.get_value());
            if java::eq_ignore_case(k, "source") {
                self.source = Some(v.to_string());
            } else if java::eq_ignore_case(k, "frame") {
                self.frame = try_parse_int(v, 1);
            } else if java::eq_ignore_case(k, "sourceFrame1") {
                self.source_frame1 = try_parse_int(v, 1);
            } else if java::eq_ignore_case(k, "sourceFrame2") {
                self.source_frame2 = try_parse_int(v, 1);
            }
        }
    }
}

/// `IRuntimeAnimationCommand`. `exec` (building keyframes from a loaded
/// clip) belongs to the animation system.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeAnimationCommand {
    CopyFrame(CopyFrame),
    CopyFrames(CopyFrames),
}

#[derive(Debug)]
pub struct RuntimeAnimationScript {
    pub base: BaseScriptObject,
    /// `name`; `None` stands for the initial `this.toString()` identity
    /// string, which `Load` always replaces.
    pub name: Option<String>,
    pub commands: Vec<RuntimeAnimationCommand>,
}

impl Default for RuntimeAnimationScript {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeAnimationScript {
    pub fn new() -> Self {
        RuntimeAnimationScript {
            base: BaseScriptObject::new(ScriptType::RuntimeAnimation),
            name: None,
            commands: Vec::new(),
        }
    }

    // TODO(animation): `exec` runs the commands against
    // `ModelManager.getAnimationClip` and registers the resulting clip.
}

impl ScriptObject for RuntimeAnimationScript {
    fn base(&self) -> &BaseScriptObject {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseScriptObject {
        &mut self.base
    }

    fn class_name(&self) -> &'static str {
        "zombie.core.skinnedmodel.runtime.RuntimeAnimationScript"
    }

    fn load(&mut self, sm: &ScriptManager, name: &str, body: &str) -> ScriptResult {
        self.name = Some(name.to_string());
        let root = sm.parse(body);
        let block = root.children.first().ok_or_else(index_error)?;
        self.base.load_common_block(block);
        for child in &block.children {
            match child.type_.as_deref() {
                Some("CopyFrame") => {
                    let mut c = CopyFrame::default();
                    c.parse(child);
                    self.commands.push(RuntimeAnimationCommand::CopyFrame(c));
                }
                Some("CopyFrames") => {
                    let mut c = CopyFrames::default();
                    c.parse(child);
                    self.commands.push(RuntimeAnimationCommand::CopyFrames(c));
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn reset(&mut self) {
        self.name = None;
        self.commands.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_util::load;
    use super::*;

    #[test]
    fn loads_commands() {
        let body = "animation Anim { CopyFrame { source = Idle, frame = 2, sourceFrame = x, } \
                    CopyFrames { source = Walk, sourceFrame1 = 3, sourceFrame2 = 5, } Other { } }";
        let (s, r) = load(RuntimeAnimationScript::new(), "Anim", body);
        r.unwrap();
        assert_eq!(s.name.as_deref(), Some("Anim"));
        assert_eq!(
            s.commands,
            vec![
                RuntimeAnimationCommand::CopyFrame(CopyFrame {
                    frame: 2,
                    source: Some("Idle".into()),
                    source_frame: 1,
                    ..Default::default()
                }),
                RuntimeAnimationCommand::CopyFrames(CopyFrames {
                    source: Some("Walk".into()),
                    source_frame1: 3,
                    source_frame2: 5,
                    ..Default::default()
                }),
            ]
        );
    }
}
