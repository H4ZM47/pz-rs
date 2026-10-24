//! `zombie.scripting.objects.ModelAttachment`.

use crate::base::{ScriptError, ScriptResult};
use crate::java;
use crate::math::Vector3;

use super::pz_max;

/// A named attachment point. The owner callbacks of `setId`
/// (`IModelAttachmentOwner`) are handled by
/// [`ModelScript::rename_attachment`](super::ModelScript::rename_attachment).
#[derive(Clone, Debug, PartialEq)]
pub struct ModelAttachment {
    id: String,
    offset: Vector3,
    rotate: Vector3,
    scale: f32,
    bone: Option<String>,
    can_attach: Option<Vec<String>>,
    zoffset: f32,
    update_constraint: bool,
}

impl ModelAttachment {
    /// `new ModelAttachment(id)`.
    pub fn new(id: &str) -> ScriptResult<Self> {
        check_id(id)?;
        Ok(ModelAttachment {
            id: id.to_string(),
            offset: Vector3::default(),
            rotate: Vector3::default(),
            scale: 1.0,
            bone: None,
            can_attach: None,
            zoffset: 0.0,
            update_constraint: true,
        })
    }

    /// `getId`.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// `setId` on an attachment without an owner.
    pub fn set_id(&mut self, id: &str) -> ScriptResult {
        check_id(id)?;
        self.id = id.to_string();
        Ok(())
    }

    /// `getOffset`.
    pub fn offset(&self) -> &Vector3 {
        &self.offset
    }

    pub fn offset_mut(&mut self) -> &mut Vector3 {
        &mut self.offset
    }

    /// `getRotate`.
    pub fn rotate(&self) -> &Vector3 {
        &self.rotate
    }

    pub fn rotate_mut(&mut self) -> &mut Vector3 {
        &mut self.rotate
    }

    /// `getScale`.
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// `setScale`: clamped to at least 0.01.
    pub fn set_scale(&mut self, scale: f32) {
        self.scale = pz_max(scale, 0.01);
    }

    /// `getBone`.
    pub fn bone(&self) -> Option<&str> {
        self.bone.as_deref()
    }

    /// `setBone`: trimmed, empty becomes null.
    pub fn set_bone(&mut self, bone: &str) {
        let b = java::trim(bone);
        self.bone = (!b.is_empty()).then(|| b.to_string());
    }

    /// `getCanAttach`.
    pub fn can_attach(&self) -> Option<&[String]> {
        self.can_attach.as_deref()
    }

    pub fn set_can_attach(&mut self, can_attach: Option<Vec<String>>) {
        self.can_attach = can_attach;
    }

    /// `getZOffset`.
    pub fn zoffset(&self) -> f32 {
        self.zoffset
    }

    pub fn set_zoffset(&mut self, zoffset: f32) {
        self.zoffset = zoffset;
    }

    /// `isUpdateConstraint`.
    pub fn is_update_constraint(&self) -> bool {
        self.update_constraint
    }

    pub fn set_update_constraint(&mut self, b: bool) {
        self.update_constraint = b;
    }
}

fn check_id(id: &str) -> ScriptResult {
    if java::is_null_or_whitespace(Some(id)) {
        return Err(ScriptError::new("java.lang.IllegalArgumentException: ModelAttachment id is null or empty"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setters_follow_java() {
        assert!(ModelAttachment::new(" ").is_err());
        let mut a = ModelAttachment::new("hand").unwrap();
        a.set_bone("  ");
        assert_eq!(a.bone(), None);
        a.set_bone(" Bip01 ");
        assert_eq!(a.bone(), Some("Bip01"));
        a.set_scale(0.0);
        assert_eq!(a.scale(), 0.01);
    }
}
