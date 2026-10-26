//! `InventoryItem.loadItem`/`saveWithSize`: an item record, decoded when
//! the codec supports its class, else kept as its bytes.

use crate::env::ItemEnv;
use crate::io::{err, Reader, SaveResult, Writer};
use crate::item::InventoryItem;
use crate::template::ItemClass;

/// One item as saved in a container or object.
#[derive(Clone, Debug, PartialEq)]
pub enum SavedItem {
    /// A decoded item.
    Item(Box<InventoryItem>),
    /// An item kept as the bytes after the length (registry id, save type,
    /// item), re-saved unchanged.
    Raw(Vec<u8>),
}

impl SavedItem {
    /// `getRegistry_id()`.
    pub fn registry_id(&self) -> i16 {
        match self {
            SavedItem::Item(i) => i.registry_id,
            SavedItem::Raw(d) => i16::from_be_bytes([d[0], d[1]]),
        }
    }

    /// `id`.
    pub fn id(&self) -> i32 {
        match self {
            SavedItem::Item(i) => i.id,
            SavedItem::Raw(d) => i32::from_be_bytes(d[3..7].try_into().unwrap()),
        }
    }

    pub fn set_id(&mut self, id: i32) {
        match self {
            SavedItem::Item(i) => i.id = id,
            SavedItem::Raw(d) => d[3..7].copy_from_slice(&id.to_be_bytes()),
        }
    }

    pub fn as_item(&self) -> Option<&InventoryItem> {
        match self {
            SavedItem::Item(i) => Some(i),
            SavedItem::Raw(_) => None,
        }
    }

    pub fn as_item_mut(&mut self) -> Option<&mut InventoryItem> {
        match self {
            SavedItem::Item(i) => Some(i),
            SavedItem::Raw(_) => None,
        }
    }

    /// `getFullType()` (raw records: the dictionary name of the id).
    pub fn full_type(&self, env: &dyn ItemEnv) -> String {
        match self {
            SavedItem::Item(i) => i.get_full_type(),
            SavedItem::Raw(_) => env
                .full_type(self.registry_id())
                .unwrap_or_else(|| format!("#{}", self.registry_id())),
        }
    }

    /// Whether this is an `InventoryContainer` holding items (such items
    /// are never grouped). Raw containers count as non-empty.
    pub fn is_nonempty_container(&self, env: &dyn ItemEnv) -> bool {
        match self {
            SavedItem::Item(i) => match &i.kind {
                crate::item::ItemKind::Container(c) => !c.container.borrow().is_empty(),
                _ => false,
            },
            SavedItem::Raw(_) => env
                .template(self.registry_id())
                .is_some_and(|t| t.class == ItemClass::Container),
        }
    }

    /// `InventoryItem.loadItem`: `None` when the original drops the item
    /// (its id cannot be created).
    pub fn load(r: &mut Reader<'_>, wv: i32, env: &dyn ItemEnv) -> SaveResult<Option<Self>> {
        let len = r.i32()?;
        if len <= 0 {
            return err(format!(
                "InventoryItem.loadItem() invalid item data length: {len}"
            ));
        }
        if len < 7 {
            return err(format!("item record of {len} bytes"));
        }
        let data = r.bytes(len as usize)?;
        let reg = i16::from_be_bytes([data[0], data[1]]);
        if !env.exists(reg) {
            return Ok(None);
        }
        Ok(Some(Self::decode(data, wv, env)))
    }

    /// Decodes a record's bytes, falling back to raw.
    pub fn decode(data: &[u8], wv: i32, env: &dyn ItemEnv) -> Self {
        let reg = i16::from_be_bytes([data[0], data[1]]);
        let raw = |why: &str| {
            env.note_raw(reg, why);
            SavedItem::Raw(data.to_vec())
        };
        let Some(t) = env.template(reg) else {
            return raw("no script");
        };
        if !env.decodes(&t) {
            return raw(t.class.java_name());
        }
        let Some(mut item) = crate::env::create_item(env, reg) else {
            return raw("not creatable");
        };
        let mut sub = Reader::new(&data[3..]);
        match item.load(&mut sub, wv, env) {
            Ok(()) if sub.remaining() == 0 => SavedItem::Item(Box::new(item)),
            Ok(()) => raw("length mismatch"),
            Err(_) => raw("load error"),
        }
    }

    /// The record without the length (what `save` writes).
    pub fn record(&self, env: &dyn ItemEnv) -> SaveResult<Vec<u8>> {
        match self {
            SavedItem::Item(i) => {
                let mut w = Writer::new();
                i.save(&mut w, env)?;
                Ok(w.buf)
            }
            SavedItem::Raw(d) => Ok(d.clone()),
        }
    }

    /// `saveWithSize`.
    pub fn save(&self, w: &mut Writer, env: &dyn ItemEnv) -> SaveResult<()> {
        let rec = self.record(env)?;
        w.i32(rec.len() as i32);
        w.bytes(&rec);
        Ok(())
    }
}

/// `loadItem`'s framing alone: the record bytes after the length.
pub fn read_record<'a>(r: &mut Reader<'a>) -> SaveResult<&'a [u8]> {
    let len = r.i32()?;
    if len <= 0 {
        return err(format!(
            "InventoryItem.loadItem() invalid item data length: {len}"
        ));
    }
    if len < 7 {
        return err(format!("item record of {len} bytes"));
    }
    r.bytes(len as usize)
}

/// Actual native sized-record phases. The legacy SavedItem/decode APIs above
/// remain named value/raw-archive carriers and do not back this path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveRecordPhase {
    Header,
    Factory,
    Body,
    Repair,
    DebugMismatch,
}

#[derive(Debug)]
#[must_use]
pub struct LiveRecordLoad {
    pub item: Option<crate::shared::ItemRef>,
    pub retired: pz_entity::RetiredComponents,
    /// Native caught body exceptions, kept separately from host failures.
    pub body_exceptions: Vec<pz_entity::EntityError>,
}

#[derive(Debug)]
#[must_use]
pub struct LiveRecordFailure {
    pub error: pz_entity::runtime_load::RuntimeHostError,
    pub receiver: Option<crate::shared::ItemRef>,
    pub retired: pz_entity::RetiredComponents,
    pub phase: LiveRecordPhase,
    pub position: usize,
}

impl LiveRecordFailure {
    pub fn into_error_and_retire(mut self) -> pz_entity::runtime_load::RuntimeHostError {
        if let Some(receiver) = self.receiver.take() {
            self.retired
                .push_owner(pz_entity::EntityOwner::from_ref(&receiver));
        }
        crate::component_host::retire_components(self.retired);
        self.error
    }
}

/// InventoryItem.loadItem using the actual original caller buffer and an
/// optional actual supplied receiver. Header/factory and repair are outside
/// the body Exception catch. No minimum7 guard or record-sized slice exists.
pub fn load_item_record(
    reader: &mut Reader<'_>,
    world_version: i32,
    env: &dyn ItemEnv,
    supplied: Option<&crate::shared::ItemRef>,
) -> Result<LiveRecordLoad, LiveRecordFailure> {
    use pz_entity::runtime_load::RuntimeHostError;
    let mut result = LiveRecordLoad {
        item: None,
        retired: pz_entity::RetiredComponents::new(),
        body_exceptions: Vec::new(),
    };
    let header = (|| -> Result<_, RuntimeHostError> {
        if reader.buf.len() > i32::MAX as usize || reader.pos > reader.buf.len() {
            return Err(RuntimeHostError::InvalidHostState(
                "invalid original ByteBuffer domain",
            ));
        }
        let length = reader
            .i32()
            .map_err(|error| RuntimeHostError::NativeException(pz_entity::EntityError(error.0)))?;
        if length <= 0 {
            return Err(RuntimeHostError::NativeException(pz_entity::EntityError(
                format!("InventoryItem.loadItem() invalid item data length: {length}"),
            )));
        }
        let start = reader.pos as i32;
        let registry = reader
            .i16()
            .map_err(|error| RuntimeHostError::NativeException(pz_entity::EntityError(error.0)))?;
        let _save_type = reader
            .i8()
            .map_err(|error| RuntimeHostError::NativeException(pz_entity::EntityError(error.0)))?;
        Ok((start.wrapping_add(length), registry))
    })();
    let (end, registry) = match header {
        Ok(header) => header,
        Err(error) => {
            return Err(LiveRecordFailure {
                error,
                receiver: supplied.cloned(),
                retired: result.retired,
                phase: LiveRecordPhase::Header,
                position: reader.pos,
            })
        }
    };
    result.item = match supplied {
        Some(item) => Some(item.clone()),
        None => match crate::env::create_item_ref(env, registry) {
            Ok(item) => item,
            Err(failure) => {
                return Err(LiveRecordFailure {
                    error: failure.error,
                    receiver: failure.item,
                    retired: failure.retired,
                    phase: LiveRecordPhase::Factory,
                    position: reader.pos,
                })
            }
        },
    };
    if let Some(item) = result.item.as_ref() {
        match crate::component_host::load_item_ref(item, reader, world_version, env) {
            Ok(retired) => result.retired.append(retired),
            Err(failure) => {
                result.retired.append(failure.retired);
                result
                    .retired
                    .push_owner(pz_entity::EntityOwner::from_ref(&failure.item));
                match failure.error {
                    RuntimeHostError::NativeException(error) => {
                        eprintln!("{error}");
                        result.body_exceptions.push(error);
                        result.item = None;
                    }
                    error => {
                        return Err(LiveRecordFailure {
                            error,
                            receiver: result.item,
                            retired: result.retired,
                            phase: LiveRecordPhase::Body,
                            position: reader.pos,
                        })
                    }
                }
            }
        }
    }
    let repaired = (|| -> Result<(), RuntimeHostError> {
        if result.item.is_none() {
            if (reader.pos as i32) < end {
                while (reader.pos as i32) < end {
                    reader.i8().map_err(|error| {
                        RuntimeHostError::NativeException(pz_entity::EntityError(error.0))
                    })?;
                }
                eprintln!(
                    "InventoryItem.loadItem() item == null, skipped bytes. itemtype: {}",
                    env.item_type_debug_string(registry)
                );
            } else {
                let mut buffer = pz_entity::io::Reader::borrow_cursor(reader.buf, &mut reader.pos)
                    .map_err(RuntimeHostError::NativeException)?;
                buffer
                    .seek_java(end)
                    .map_err(RuntimeHostError::NativeException)?;
                drop(buffer);
                eprintln!("InventoryItem.loadItem() item == null, resetting buffer position to '{end}'. itemtype: {}", env.item_type_debug_string(registry));
            }
        } else if reader.pos as i32 != end {
            let mut buffer = pz_entity::io::Reader::borrow_cursor(reader.buf, &mut reader.pos)
                .map_err(RuntimeHostError::NativeException)?;
            buffer
                .seek_java(end)
                .map_err(RuntimeHostError::NativeException)?;
            drop(buffer);
            eprintln!("InventoryItem.loadItem() data length not matching, resetting buffer position to '{end}'. itemtype: {}", env.item_type_debug_string(registry));
            if crate::component_host::component_debug_enabled()? {
                return Err(RuntimeHostError::NativeException(pz_entity::EntityError(
                    format!(
                        "InventoryItem.loadItem() read more data than save() wrote ({})",
                        env.item_type_debug_string(registry)
                    ),
                )));
            }
        }
        Ok(())
    })();
    if let Err(error) = repaired {
        return Err(LiveRecordFailure {
            error,
            receiver: result.item,
            retired: result.retired,
            phase: LiveRecordPhase::Repair,
            position: reader.pos,
        });
    }
    Ok(result)
}
