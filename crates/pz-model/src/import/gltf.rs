//! glTF 2 binary (`.glb`).
use super::scene::AiScene;
use super::ImportError;

pub fn import(_data: &[u8]) -> Result<AiScene, ImportError> {
    Err(ImportError("glTF: not implemented".into()))
}
