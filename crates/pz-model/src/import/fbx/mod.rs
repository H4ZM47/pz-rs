//! FBX (binary and ASCII).
use super::scene::AiScene;
use super::ImportError;

pub fn import(_data: &[u8]) -> Result<AiScene, ImportError> {
    Err(ImportError("FBX: not implemented".into()))
}
