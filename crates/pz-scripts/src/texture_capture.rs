//! Actual asset capture is opt-in for a native host. Offline parser imports
//! retain an explicitly uncaptured profile rather than guessing missing assets.
use crate::{ScriptError, ScriptManager, ScriptResult};
use pz_render::{TextureHostError, TextureRef};
use std::cell::RefCell;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CapturedScriptLoadError {
    Native(ScriptError),
    Host(TextureHostError),
}
impl std::fmt::Display for CapturedScriptLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Native(error) => error.fmt(f),
            Self::Host(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for CapturedScriptLoadError {}

/// One host load boundary. Only a fresh outer operation clears an earlier
/// operation's fault; nested/reentrant loads keep the first current fault.
#[must_use]
pub struct TextureLoadOperation<'a> {
    manager: &'a ScriptManager,
}
impl TextureLoadOperation<'_> {
    pub fn host_fault(&self) -> Option<TextureHostError> {
        self.manager.texture_host_fault()
    }
}
impl Drop for TextureLoadOperation<'_> {
    fn drop(&mut self) {
        self.manager
            .texture_load_operation_depth
            .set(self.manager.texture_load_operation_depth.get() - 1);
    }
}

thread_local! {
    static RETIRED: RefCell<Vec<TextureRef>> = const { RefCell::new(Vec::new()) };
}
pub(crate) fn retire_texture(texture: TextureRef) {
    RETIRED.with(|retired| retired.borrow_mut().push(texture));
}
/// Actual displaced Texture owners. Drain only after enclosing script/host
/// guards; no native reset, hooks or object-class replacement is involved.
pub fn take_retired_textures() -> Vec<TextureRef> {
    RETIRED.with(|retired| std::mem::take(&mut *retired.borrow_mut()))
}
impl ScriptManager {
    pub fn begin_texture_load_operation(&self) -> TextureLoadOperation<'_> {
        let depth = self.texture_load_operation_depth.get();
        if depth == 0 {
            *self.texture_host_fault.borrow_mut() = None;
        }
        self.texture_load_operation_depth.set(depth + 1);
        TextureLoadOperation { manager: self }
    }
    pub fn enable_texture_capture(&self) {
        self.texture_capture_enabled.set(true);
    }
    pub fn texture_capture_enabled(&self) -> bool {
        self.texture_capture_enabled.get()
    }
    pub fn texture_host_fault(&self) -> Option<TextureHostError> {
        self.texture_host_fault.borrow().clone()
    }
    pub(crate) fn record_texture_host_fault(&self, error: TextureHostError) {
        let mut fault = self.texture_host_fault.borrow_mut();
        if fault.is_none() {
            *fault = Some(error);
        }
    }
    pub(crate) fn texture_capture_aborted(&self) -> bool {
        self.texture_host_fault.borrow().is_some()
    }
    fn finish_texture_load(&self, result: ScriptResult) -> Result<(), CapturedScriptLoadError> {
        if let Some(error) = self.texture_host_fault() {
            return Err(CapturedScriptLoadError::Host(error));
        }
        result.map_err(CapturedScriptLoadError::Native)
    }
    pub fn load_with_textures(
        &self,
        fs: &pz_fs::GameFs,
        mods: &[crate::ModScriptDirs],
    ) -> Result<(), CapturedScriptLoadError> {
        self.enable_texture_capture();
        let operation = self.begin_texture_load_operation();
        if let Some(error) = operation.host_fault() {
            return Err(CapturedScriptLoadError::Host(error));
        }
        let result = self.load(fs, mods);
        self.finish_texture_load(result)
    }
    pub fn reload_scripts_with_textures(
        &self,
        types: &[crate::ScriptType],
    ) -> Result<(), CapturedScriptLoadError> {
        self.enable_texture_capture();
        let operation = self.begin_texture_load_operation();
        if let Some(error) = operation.host_fault() {
            return Err(CapturedScriptLoadError::Host(error));
        }
        self.reload_scripts(types);
        self.finish_texture_load(Ok(()))
    }
}

#[cfg(test)]
mod tests;
