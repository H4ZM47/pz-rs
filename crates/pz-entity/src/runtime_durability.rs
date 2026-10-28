//! The actual Durability leaf body. Nullable material is native state and
//! cannot be replaced by the legacy archive value's nonnullable default.

use pz_scripts::objects::entity::components::combat::{DurabilityScript, MaterialType};

use crate::components::basic::Durability;
use crate::io::{read_header, EntityError, HeaderSize, HeaderWrite, Reader, Writer};
use crate::RuntimeError;

#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeDurability {
    pub current_hit_points: f32,
    pub max_hit_points: f32,
    pub material: Option<MaterialType>,
}

impl Default for RuntimeDurability {
    fn default() -> Self {
        Self {
            current_hit_points: 1.0,
            max_hit_points: 1.0,
            material: Some(MaterialType::Default),
        }
    }
}

impl RuntimeDurability {
    pub(crate) fn from_archive(value: Durability) -> Self {
        Self {
            current_hit_points: value.current_hit_points,
            max_hit_points: value.max_hit_points,
            material: Some(value.material),
        }
    }

    pub fn value_snapshot(&self) -> Result<Durability, RuntimeError> {
        let material = self.material.ok_or(RuntimeError::UnsupportedRuntime(
            "Nullable Durability material cannot be projected into legacy archive data",
        ))?;
        Ok(Durability {
            current_hit_points: self.current_hit_points,
            max_hit_points: self.max_hit_points,
            material,
        })
    }

    pub fn read_from_script(&mut self, script: &DurabilityScript) {
        // Native first calls return-only Component.readFromScript, then
        // assigns these fields in this order, including a null material.
        self.current_hit_points = script.current_hit_points();
        self.max_hit_points = script.max_hit_points();
        self.material = script.material();
    }

    pub fn load(&mut self, reader: &mut Reader<'_>) -> Result<(), RuntimeError> {
        // These native assignments precede even a failed header read.
        self.current_hit_points = 1.0;
        self.max_hit_points = 1.0;
        self.material = Some(MaterialType::Default);
        let flags = read_header(reader, HeaderSize::Short)?;
        if flags & 1 != 0 {
            self.current_hit_points = reader.f32()?;
        }
        if flags & 2 != 0 {
            self.max_hit_points = reader.f32()?;
        }
        if flags & 4 != 0 {
            let name = reader.string()?;
            let material = MaterialType::value_of(&name).map_err(|_| {
                RuntimeError::Codec(EntityError(format!(
                    "No enum constant zombie.iso.enums.MaterialType.{name}"
                )))
            })?;
            self.material = Some(material);
        }
        Ok(())
    }

    pub fn write(&self, writer: &mut Writer) -> Result<(), RuntimeError> {
        // Native BitHeader.create immediately writes a zero placeholder.
        // It is patched only after all scalar/string writes succeed.
        let mut header = HeaderWrite::start(writer, HeaderSize::Short);
        if self.current_hit_points != 1.0 {
            header.add(1);
            writer.f32(self.current_hit_points);
        }
        if self.max_hit_points != 1.0 {
            header.add(2);
            writer.f32(self.max_hit_points);
        }
        if self.material != Some(MaterialType::Default) {
            header.add(4);
            let material = self
                .material
                .ok_or_else(|| RuntimeError::Codec(EntityError("NullPointerException".into())))?;
            writer.string(material.name());
        }
        header.write(writer);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
