//! `SandboxOptions.save(ByteBuffer)` / `load(ByteBuffer)`: the save's
//! `map_sand.bin`.
//!
//! Layout (big-endian): `"SAND"`, `i32` world version, `i32` sandbox
//! version, `i32` option count, then per option its name and
//! `getValueAsString()`, then the world preset name. Strings are
//! `GameWindow.WriteString`: an `i16` byte count and UTF-8 bytes.

use pz_classfile::class::Constant;
use pz_classfile::Jar;

use crate::{SandboxOptions, SANDBOX_VERSION};

/// `IsoWorld.WorldVersion`, read from the jar (the version `save` writes).
pub fn world_version(jar: &Jar) -> pz_classfile::Result<i32> {
    let c = pz_classfile::load_class(jar, "zombie/iso/IsoWorld")?;
    let missing = || pz_classfile::Error::Missing("IsoWorld.WorldVersion".into());
    let f = c
        .fields
        .iter()
        .find(|f| f.name == "WorldVersion")
        .ok_or_else(missing)?;
    match c.constant(f.constant_value.ok_or_else(missing)?)? {
        Constant::Integer(v) => Ok(*v),
        _ => Err(missing()),
    }
}

/// `GameWindow.WriteString(ByteBuffer, s)`.
fn write_string(out: &mut Vec<u8>, s: &str) {
    if s.is_empty() {
        out.extend_from_slice(&0i16.to_be_bytes());
        return;
    }
    out.extend_from_slice(&(s.len() as i16).to_be_bytes());
    out.extend_from_slice(s.as_bytes());
}

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let end = self.pos + n;
        let s = self
            .b
            .get(self.pos..end)
            .ok_or("BufferUnderflowException")?;
        self.pos = end;
        Ok(s)
    }

    fn i32(&mut self) -> Result<i32, String> {
        Ok(i32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }

    /// `GameWindow.ReadString(ByteBuffer)`: a non-positive count is "";
    /// fewer bytes than counted reads what is there; malformed UTF-8 is
    /// replaced.
    fn string(&mut self) -> Result<String, String> {
        let n = i16::from_be_bytes(self.take(2)?.try_into().unwrap());
        if n <= 0 {
            return Ok(String::new());
        }
        let n = (n as usize).min(self.b.len() - self.pos);
        Ok(String::from_utf8_lossy(self.take(n)?).into_owned())
    }
}

impl SandboxOptions {
    /// `save(ByteBuffer)`: the bytes of `map_sand.bin`. `world_version` is
    /// [`world_version`]; `preset` is `getWorld().getPreset()`.
    pub fn save_bin(&self, world_version: i32, preset: &str) -> Vec<u8> {
        let mut out = b"SAND".to_vec();
        out.extend_from_slice(&world_version.to_be_bytes());
        out.extend_from_slice(&SANDBOX_VERSION.to_be_bytes());
        out.extend_from_slice(&(self.options.len() as i32).to_be_bytes());
        for o in &self.options {
            write_string(&mut out, o.name());
            write_string(&mut out, &o.config.get_value_as_string());
        }
        write_string(&mut out, preset);
        out
    }

    /// `load(ByteBuffer)`: parses each saved option into the matching one
    /// (after the version upgrades) and returns the preset name the
    /// original passes to `getWorld().setPreset`. Unknown names are
    /// skipped, as the original logs and skips them. `Err` is the
    /// original's `BufferUnderflowException`.
    pub fn load_bin(&mut self, bytes: &[u8]) -> Result<String, String> {
        let mut r = Reader { b: bytes, pos: 0 };
        r.take(4)?;
        r.i32()?;
        let version = r.i32()?;
        let count = r.i32()?;
        for _ in 0..count {
            let name = r.string()?;
            let value = r.string()?;
            let value = self.upgrade_option_value(&name, &value, version);
            match self.option_by_name_mut(&name) {
                Some(o) => o.config.parse(&value),
                None => eprintln!("ERROR unknown SandboxOption \"{name}\""),
            }
        }
        r.string()
    }
}

#[cfg(test)]
mod tests {
    use crate::defs::{OptionDef, OptionKind};
    use crate::SandboxOptions;

    fn opts() -> SandboxOptions {
        let d = |name: &str, kind| OptionDef {
            name: name.into(),
            kind,
            translation: None,
            value_translation: None,
        };
        SandboxOptions::new(&[
            d(
                "A",
                OptionKind::Integer {
                    min: 0,
                    max: 10,
                    default: 3,
                },
            ),
            d("T.B", OptionKind::Boolean { default: false }),
        ])
    }

    #[test]
    fn save_then_load() {
        let mut s = opts();
        s.option_by_name_mut("A").unwrap().config.parse("7");
        let bytes = s.save_bin(249, "Custé");
        assert_eq!(&bytes[..4], b"SAND");
        let mut t = opts();
        assert_eq!(t.load_bin(&bytes).unwrap(), "Custé");
        assert_eq!(t.option_by_name("A").unwrap().config.get_int(), 7);
        assert_eq!(t.save_bin(249, "Custé"), bytes);
        // A truncated file is the original's BufferUnderflowException.
        assert!(opts().load_bin(&bytes[..10]).is_err());
    }
}
