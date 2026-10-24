//! `zombie.scripting.objects.SoundTimelineScript`.

use crate::base::{index_error, BaseScriptObject, ScriptObject, ScriptResult};
use crate::java::{self, JHashMap};
use crate::manager::ScriptManager;
use crate::script_type::ScriptType;

use super::try_parse_int;

#[derive(Debug)]
pub struct SoundTimelineScript {
    pub base: BaseScriptObject,
    event_name: Option<String>,
    position_by_name: JHashMap<i32>,
}

impl Default for SoundTimelineScript {
    fn default() -> Self {
        Self::new()
    }
}

impl SoundTimelineScript {
    pub fn new() -> Self {
        SoundTimelineScript {
            base: BaseScriptObject::new(ScriptType::SoundTimeline),
            event_name: None,
            position_by_name: JHashMap::new(),
        }
    }

    /// `getEventName`.
    pub fn event_name(&self) -> Option<&str> {
        self.event_name.as_deref()
    }

    /// `getPosition`: -1 for unknown ids.
    pub fn position(&self, id: &str) -> i32 {
        self.position_by_name.get(id).copied().unwrap_or(-1)
    }

    pub fn positions(&self) -> &JHashMap<i32> {
        &self.position_by_name
    }
}

impl ScriptObject for SoundTimelineScript {
    fn base(&self) -> &BaseScriptObject {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseScriptObject {
        &mut self.base
    }

    fn class_name(&self) -> &'static str {
        "zombie.scripting.objects.SoundTimelineScript"
    }

    fn load(&mut self, sm: &ScriptManager, name: &str, body: &str) -> ScriptResult {
        self.event_name = Some(name.to_string());
        let root = sm.parse(body);
        let block = root.children.first().ok_or_else(index_error)?;
        self.base.load_common_block(block);
        for value in &block.values {
            let k = java::trim(value.get_key());
            let v = java::trim(value.get_value());
            self.position_by_name.insert(k.to_string(), try_parse_int(v, 0));
        }
        Ok(())
    }

    fn reset(&mut self) {
        self.position_by_name.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_util::load;
    use super::*;

    #[test]
    fn loads_positions() {
        let (s, r) = load(SoundTimelineScript::new(), "Ev", "soundTimeline Ev { Start = 10, End = x, }");
        r.unwrap();
        assert_eq!(s.event_name(), Some("Ev"));
        assert_eq!(s.position("Start"), 10);
        assert_eq!(s.position("End"), 0);
        assert_eq!(s.position("Nope"), -1);
    }
}
