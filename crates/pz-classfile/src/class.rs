//! Java class-file parsing (JVMS §4): the constant pool, fields and method
//! bytecode, which is enough to read constants out of static initializers.

use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq)]
pub enum Constant {
    Unused,
    Utf8(String),
    Integer(i32),
    Float(f32),
    Long(i64),
    Double(f64),
    Class(u16),
    String(u16),
    FieldRef(u16, u16),
    MethodRef(u16, u16),
    InterfaceMethodRef(u16, u16),
    NameAndType(u16, u16),
    MethodHandle(u8, u16),
    MethodType(u16),
    Dynamic(u16, u16),
    InvokeDynamic(u16, u16),
    Module(u16),
    Package(u16),
}

#[derive(Clone, Debug)]
pub struct Member {
    pub access: u16,
    pub name: String,
    pub descriptor: String,
    /// The `Code` attribute's bytecode, for methods that have one.
    pub code: Option<Vec<u8>>,
    /// The `ConstantValue` attribute's pool index, for constant fields.
    pub constant_value: Option<u16>,
}

pub const ACC_STATIC: u16 = 0x0008;
pub const ACC_FINAL: u16 = 0x0010;
pub const ACC_ENUM: u16 = 0x4000;

#[derive(Clone, Debug)]
pub struct ClassFile {
    pub pool: Vec<Constant>,
    pub access: u16,
    pub this_class: String,
    pub super_class: Option<String>,
    pub fields: Vec<Member>,
    pub methods: Vec<Member>,
}

/// A resolved field or method reference: (owner class, name, descriptor).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberRef {
    pub class: String,
    pub name: String,
    pub descriptor: String,
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn bytes(&mut self, n: usize) -> Result<&[u8]> {
        let s = self.b.get(self.at..self.at + n).ok_or(Error::Truncated)?;
        self.at += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        let s = self.bytes(2)?;
        Ok(u16::from_be_bytes([s[0], s[1]]))
    }
    fn u32(&mut self) -> Result<u32> {
        let s = self.bytes(4)?;
        Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    }
}

/// Decodes the class file "modified UTF-8" (JVMS §4.4.7).
pub fn decode_modified_utf8(b: &[u8]) -> Result<String> {
    let mut units: Vec<u16> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let c = b[i] as u16;
        if c & 0x80 == 0 {
            units.push(c);
            i += 1;
        } else if c & 0xE0 == 0xC0 {
            let c2 = *b.get(i + 1).ok_or(Error::BadUtf8)? as u16;
            units.push(((c & 0x1F) << 6) | (c2 & 0x3F));
            i += 2;
        } else if c & 0xF0 == 0xE0 {
            let c2 = *b.get(i + 1).ok_or(Error::BadUtf8)? as u16;
            let c3 = *b.get(i + 2).ok_or(Error::BadUtf8)? as u16;
            units.push(((c & 0x0F) << 12) | ((c2 & 0x3F) << 6) | (c3 & 0x3F));
            i += 3;
        } else {
            return Err(Error::BadUtf8);
        }
    }
    Ok(String::from_utf16_lossy(&units))
}

impl ClassFile {
    pub fn parse(b: &[u8]) -> Result<ClassFile> {
        let mut r = Reader { b, at: 0 };
        if r.u32()? != 0xCAFE_BABE {
            return Err(Error::NotAClass);
        }
        r.u16()?;
        r.u16()?;
        let count = r.u16()? as usize;
        let mut pool = vec![Constant::Unused];
        while pool.len() < count {
            let tag = r.u8()?;
            let c = match tag {
                1 => {
                    let n = r.u16()? as usize;
                    Constant::Utf8(decode_modified_utf8(r.bytes(n)?)?)
                }
                3 => Constant::Integer(r.u32()? as i32),
                4 => Constant::Float(f32::from_bits(r.u32()?)),
                5 => Constant::Long(((r.u32()? as u64) << 32 | r.u32()? as u64) as i64),
                6 => Constant::Double(f64::from_bits((r.u32()? as u64) << 32 | r.u32()? as u64)),
                7 => Constant::Class(r.u16()?),
                8 => Constant::String(r.u16()?),
                9 => Constant::FieldRef(r.u16()?, r.u16()?),
                10 => Constant::MethodRef(r.u16()?, r.u16()?),
                11 => Constant::InterfaceMethodRef(r.u16()?, r.u16()?),
                12 => Constant::NameAndType(r.u16()?, r.u16()?),
                15 => Constant::MethodHandle(r.u8()?, r.u16()?),
                16 => Constant::MethodType(r.u16()?),
                17 => Constant::Dynamic(r.u16()?, r.u16()?),
                18 => Constant::InvokeDynamic(r.u16()?, r.u16()?),
                19 => Constant::Module(r.u16()?),
                20 => Constant::Package(r.u16()?),
                t => return Err(Error::Unsupported(format!("constant tag {t}"))),
            };
            let wide = matches!(c, Constant::Long(_) | Constant::Double(_));
            pool.push(c);
            if wide {
                pool.push(Constant::Unused);
            }
        }
        let mut cf = ClassFile {
            pool,
            access: 0,
            this_class: String::new(),
            super_class: None,
            fields: Vec::new(),
            methods: Vec::new(),
        };
        cf.access = r.u16()?;
        cf.this_class = cf.class_name(r.u16()?)?.to_string();
        let sup = r.u16()?;
        cf.super_class = if sup == 0 { None } else { Some(cf.class_name(sup)?.to_string()) };
        let n = r.u16()?;
        for _ in 0..n {
            r.u16()?;
        }
        cf.fields = cf.members(&mut r)?;
        cf.methods = cf.members(&mut r)?;
        Ok(cf)
    }

    fn members(&self, r: &mut Reader) -> Result<Vec<Member>> {
        let n = r.u16()?;
        let mut out = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let access = r.u16()?;
            let name = self.utf8(r.u16()?)?.to_string();
            let descriptor = self.utf8(r.u16()?)?.to_string();
            let mut code = None;
            let mut constant_value = None;
            for _ in 0..r.u16()? {
                let attr = self.utf8(r.u16()?)?.to_string();
                let len = r.u32()? as usize;
                let body = r.bytes(len)?;
                if attr == "ConstantValue" && body.len() >= 2 {
                    constant_value = Some(u16::from_be_bytes([body[0], body[1]]));
                }
                if attr == "Code" {
                    let code_len = u32::from_be_bytes(body.get(4..8).ok_or(Error::Truncated)?.try_into().unwrap()) as usize;
                    code = Some(body.get(8..8 + code_len).ok_or(Error::Truncated)?.to_vec());
                }
            }
            out.push(Member { access, name, descriptor, code, constant_value });
        }
        Ok(out)
    }

    pub fn constant(&self, i: u16) -> Result<&Constant> {
        self.pool.get(i as usize).ok_or(Error::BadIndex(i))
    }

    pub fn utf8(&self, i: u16) -> Result<&str> {
        match self.constant(i)? {
            Constant::Utf8(s) => Ok(s),
            _ => Err(Error::BadIndex(i)),
        }
    }

    pub fn class_name(&self, i: u16) -> Result<&str> {
        match self.constant(i)? {
            Constant::Class(n) => self.utf8(*n),
            _ => Err(Error::BadIndex(i)),
        }
    }

    /// Resolves a field, method or interface-method reference.
    pub fn member_ref(&self, i: u16) -> Result<MemberRef> {
        let (c, nt) = match self.constant(i)? {
            Constant::FieldRef(c, nt) | Constant::MethodRef(c, nt) | Constant::InterfaceMethodRef(c, nt) => (*c, *nt),
            _ => return Err(Error::BadIndex(i)),
        };
        let (n, d) = match self.constant(nt)? {
            Constant::NameAndType(n, d) => (*n, *d),
            _ => return Err(Error::BadIndex(nt)),
        };
        Ok(MemberRef {
            class: self.class_name(c)?.to_string(),
            name: self.utf8(n)?.to_string(),
            descriptor: self.utf8(d)?.to_string(),
        })
    }

    pub fn method(&self, name: &str) -> Option<&Member> {
        self.methods.iter().find(|m| m.name == name)
    }

    /// Names of an enum's constants in declaration order (`values()` order).
    pub fn enum_constants(&self) -> Vec<&str> {
        self.fields.iter().filter(|f| f.access & ACC_ENUM != 0).map(|f| f.name.as_str()).collect()
    }

    /// Names of `static` fields of type `descriptor` (e.g. `Lpkg/Cls;`), in
    /// declaration order: the constants of key classes such as `ItemTag`.
    pub fn static_fields_of_type(&self, descriptor: &str) -> Vec<&str> {
        self.fields
            .iter()
            .filter(|f| f.access & ACC_STATIC != 0 && f.descriptor == descriptor)
            .map(|f| f.name.as_str())
            .collect()
    }

    /// `static final String` compile-time constants (`ConstantValue`), in
    /// declaration order.
    pub fn static_final_strings(&self) -> Vec<(&str, &str)> {
        self.fields
            .iter()
            .filter(|f| f.access & (ACC_STATIC | ACC_FINAL) == ACC_STATIC | ACC_FINAL)
            .filter_map(|f| match self.constant(f.constant_value?).ok()? {
                Constant::String(s) => Some((f.name.as_str(), self.utf8(*s).ok()?)),
                _ => None,
            })
            .collect()
    }
}
