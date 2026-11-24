//! Build 42.22 `zombie.network.id.ObjectIDManager` (`id_manager_data.bin`)
//! and `zombie.core.ImportantAreaManager` (`important_area_data.bin`).

use std::io::{self, Read, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// `zombie.network.id.ObjectIDType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(i8)]
pub enum ObjectIDType {
    Unknown = -1,
    Player = 0,
    Zombie = 1,
    Item = 2,
    Container = 3,
    DeadBody = 4,
    Vehicle = 5,
}

impl ObjectIDType {
    pub const ALL: [ObjectIDType; 7] = [
        Self::Unknown,
        Self::Player,
        Self::Zombie,
        Self::Item,
        Self::Container,
        Self::DeadBody,
        Self::Vehicle,
    ];

    pub const PERMANENT_COUNT: u8 = 4;

    pub fn from_index(index: i8) -> Option<Self> {
        match index {
            -1 => Some(Self::Unknown),
            0 => Some(Self::Player),
            1 => Some(Self::Zombie),
            2 => Some(Self::Item),
            3 => Some(Self::Container),
            4 => Some(Self::DeadBody),
            5 => Some(Self::Vehicle),
            _ => None,
        }
    }

    pub fn index(self) -> i8 {
        self as i8
    }

    pub fn is_permanent(self) -> bool {
        matches!(
            self,
            Self::Item | Self::Container | Self::DeadBody | Self::Vehicle
        )
    }

    fn slot(self) -> usize {
        match self {
            Self::Unknown => 0,
            Self::Player => 1,
            Self::Zombie => 2,
            Self::Item => 3,
            Self::Container => 4,
            Self::DeadBody => 5,
            Self::Vehicle => 6,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct TypeCounters {
    last_id: i64,
    count_new_id: i64,
}

/// `zombie.network.id.ObjectIDManager`: persistent per-type object ID
/// counters stored in `id_manager_data.bin`.
#[derive(Clone, Debug, Default)]
pub struct ObjectIDManager {
    counters: [TypeCounters; 7],
    check_limiter: i32,
}

impl ObjectIDManager {
    pub const FILE_NAME: &'static str = "id_manager_data.bin";
    pub const SAVE_THRESHOLD: i64 = 100;
    pub const LOAD_SAFETY_OFFSET: i64 = 100;
    pub const CHECK_LIMITER_INTERVAL: i32 = 100;

    pub fn new() -> Self {
        Self::default()
    }

    /// `ObjectIDManager.clear()` — resets counters for a fresh session.
    pub fn clear(&mut self) {
        self.counters = [TypeCounters::default(); 7];
        self.check_limiter = 0;
    }

    /// `ObjectIDType.allocateID()`.
    pub fn allocate_id(&mut self, ty: ObjectIDType) -> i64 {
        let c = &mut self.counters[ty.slot()];
        c.last_id = c.last_id.wrapping_add(1);
        c.count_new_id = c.count_new_id.wrapping_add(1);
        c.last_id
    }

    pub fn last_id(&self, ty: ObjectIDType) -> i64 {
        self.counters[ty.slot()].last_id
    }

    pub fn count_new_id(&self, ty: ObjectIDType) -> i64 {
        self.counters[ty.slot()].count_new_id
    }

    /// `ObjectIDManager.isNeedToSave()`: true when any type has allocated
    /// at least `SAVE_THRESHOLD` (100) new IDs since the last save.
    pub fn is_need_to_save(&self) -> bool {
        self.counters
            .iter()
            .any(|c| c.count_new_id >= Self::SAVE_THRESHOLD)
    }

    /// `ObjectIDManager.load(DataInputStream, worldVersion)`.
    pub fn load_stream<R: Read>(&mut self, mut r: R) -> io::Result<()> {
        let mut b1 = [0u8; 1];
        r.read_exact(&mut b1)?;
        let count = b1[0];
        let mut b8 = [0u8; 8];
        for _ in 0..count {
            r.read_exact(&mut b1)?;
            let idx = b1[0] as i8;
            r.read_exact(&mut b8)?;
            let last_id = i64::from_be_bytes(b8);
            if let Some(ty) = ObjectIDType::from_index(idx) {
                let c = &mut self.counters[ty.slot()];
                c.last_id = last_id.wrapping_add(Self::LOAD_SAFETY_OFFSET);
                c.count_new_id = 0;
            }
        }
        Ok(())
    }

    /// `ObjectIDManager.save(DataOutputStream)`.
    pub fn save_stream<W: Write>(&mut self, mut w: W) -> io::Result<()> {
        w.write_all(&[ObjectIDType::PERMANENT_COUNT])?;
        for ty in ObjectIDType::ALL {
            let c = &mut self.counters[ty.slot()];
            if ty.is_permanent() {
                w.write_all(&[ty.index() as u8])?;
                w.write_all(&c.last_id.to_be_bytes())?;
            }
            c.count_new_id = 0;
        }
        Ok(())
    }

    /// Loads `<save_dir>/id_manager_data.bin` if present (`i32` world version
    /// header followed by [`Self::load_stream`]).
    pub fn load_file(&mut self, save_dir: &Path) -> io::Result<Option<i32>> {
        let path = save_dir.join(Self::FILE_NAME);
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        if bytes.len() < 4 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "id_manager_data.bin shorter than 4-byte world version",
            ));
        }
        let world_version = i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        self.load_stream(&bytes[4..])?;
        Ok(Some(world_version))
    }

    /// `ObjectIDManager.checkForSaveDataFile(force)` writing
    /// `<save_dir>/id_manager_data.bin` with big-endian `world_version` header.
    pub fn save_file(
        &mut self,
        save_dir: &Path,
        world_version: i32,
        force: bool,
        no_save: bool,
    ) -> io::Result<bool> {
        if no_save {
            return Ok(false);
        }
        if !force {
            self.check_limiter += 1;
            if self.check_limiter < Self::CHECK_LIMITER_INTERVAL {
                return Ok(false);
            }
            self.check_limiter = 0;
            if !self.is_need_to_save() {
                return Ok(false);
            }
        }
        std::fs::create_dir_all(save_dir)?;
        let mut buf = Vec::with_capacity(4 + 1 + 4 * 9);
        buf.extend_from_slice(&world_version.to_be_bytes());
        self.save_stream(&mut buf)?;
        std::fs::write(save_dir.join(Self::FILE_NAME), buf)?;
        Ok(true)
    }
}

/// `zombie.core.ImportantArea`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImportantArea {
    pub sx: i32,
    pub sy: i32,
    pub last_update_ms: i64,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// `zombie.core.ImportantAreaManager`: tracks recently active map areas
/// persisted in `important_area_data.bin`.
#[derive(Clone, Debug)]
pub struct ImportantAreaManager {
    pub areas: Vec<ImportantArea>,
    pub max_areas: usize,
    pub timeout_ms: i64,
}

impl Default for ImportantAreaManager {
    fn default() -> Self {
        Self {
            areas: Vec::new(),
            max_areas: 20,
            timeout_ms: 10_000,
        }
    }
}

impl ImportantAreaManager {
    pub const FILE_NAME: &'static str = "important_area_data.bin";

    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.areas.clear();
    }

    /// `ImportantAreaManager.updateOrAdd(x, y)`: divides world tile coords by
    /// 8 (`PZMath.coorddivision(x, 8)`) and refreshes or inserts the entry.
    pub fn update_or_add(&mut self, x: i32, y: i32) {
        let sx = x.div_euclid(8);
        let sy = y.div_euclid(8);
        let now = now_ms();
        if let Some(a) = self.areas.iter_mut().find(|a| a.sx == sx && a.sy == sy) {
            a.last_update_ms = now;
            return;
        }
        if self.areas.len() >= self.max_areas && !self.areas.is_empty() {
            self.areas.remove(0);
        }
        self.areas.push(ImportantArea {
            sx,
            sy,
            last_update_ms: now,
        });
    }

    /// Removes areas older than `timeout_ms` relative to `now_ms`.
    pub fn prune_expired(&mut self, current_ms: i64) -> usize {
        let before = self.areas.len();
        let timeout = self.timeout_ms;
        self.areas
            .retain(|a| current_ms.saturating_sub(a.last_update_ms) <= timeout);
        before - self.areas.len()
    }

    /// `ImportantAreaManager.load(ByteBuffer, worldVersion)`.
    pub fn load_stream<R: Read>(&mut self, mut r: R) -> io::Result<()> {
        self.areas.clear();
        let mut b4 = [0u8; 4];
        r.read_exact(&mut b4)?;
        let count = i32::from_be_bytes(b4).max(0) as usize;
        let now = now_ms();
        for _ in 0..count {
            r.read_exact(&mut b4)?;
            let sx = i32::from_be_bytes(b4);
            r.read_exact(&mut b4)?;
            let sy = i32::from_be_bytes(b4);
            self.areas.push(ImportantArea {
                sx,
                sy,
                last_update_ms: now,
            });
        }
        Ok(())
    }

    /// `ImportantAreaManager.save(ByteBuffer)`.
    pub fn save_stream<W: Write>(&self, mut w: W) -> io::Result<()> {
        w.write_all(&(self.areas.len() as i32).to_be_bytes())?;
        for a in &self.areas {
            w.write_all(&a.sx.to_be_bytes())?;
            w.write_all(&a.sy.to_be_bytes())?;
        }
        Ok(())
    }

    /// Loads `<save_dir>/important_area_data.bin` if present (`i32` world
    /// version header followed by [`Self::load_stream`]).
    pub fn load_file(&mut self, save_dir: &Path) -> io::Result<Option<i32>> {
        let path = save_dir.join(Self::FILE_NAME);
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        if bytes.len() < 4 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "important_area_data.bin shorter than 4-byte world version",
            ));
        }
        let world_version = i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        self.load_stream(&bytes[4..])?;
        Ok(Some(world_version))
    }

    /// `ImportantAreaManager.saveDataFile()` writing
    /// `<save_dir>/important_area_data.bin` with big-endian `world_version` header.
    pub fn save_file(
        &self,
        save_dir: &Path,
        world_version: i32,
        no_save: bool,
    ) -> io::Result<bool> {
        if no_save {
            return Ok(false);
        }
        std::fs::create_dir_all(save_dir)?;
        let mut buf = Vec::with_capacity(8 + self.areas.len() * 8);
        buf.extend_from_slice(&world_version.to_be_bytes());
        self.save_stream(&mut buf)?;
        std::fs::write(save_dir.join(Self::FILE_NAME), buf)?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_id_manager_persists_permanent_types_with_load_offset() {
        let dir = std::env::temp_dir().join(format!(
            "pz-world-object-ids-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);

        let mut mgr = ObjectIDManager::new();
        assert_eq!(mgr.allocate_id(ObjectIDType::Item), 1);
        assert_eq!(mgr.allocate_id(ObjectIDType::Item), 2);
        assert_eq!(mgr.allocate_id(ObjectIDType::DeadBody), 1);
        // Transient type (Player) is NOT serialized.
        assert_eq!(mgr.allocate_id(ObjectIDType::Player), 1);
        assert!(!mgr.is_need_to_save());

        assert!(mgr.save_file(&dir, 240, true, false).unwrap());
        assert_eq!(mgr.count_new_id(ObjectIDType::Item), 0);

        let mut loaded = ObjectIDManager::new();
        let ver = loaded.load_file(&dir).unwrap();
        assert_eq!(ver, Some(240));
        // Build 42.22 adds +100 safety offset on load for every persisted type.
        assert_eq!(loaded.last_id(ObjectIDType::Item), 102);
        assert_eq!(loaded.last_id(ObjectIDType::DeadBody), 101);
        assert_eq!(loaded.last_id(ObjectIDType::Container), 100);
        assert_eq!(loaded.last_id(ObjectIDType::Vehicle), 100);
        assert_eq!(loaded.last_id(ObjectIDType::Player), 0);
        assert_eq!(loaded.allocate_id(ObjectIDType::Item), 103);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn important_area_manager_round_trips_and_prunes() {
        let dir = std::env::temp_dir().join(format!(
            "pz-world-important-areas-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);

        let mut mgr = ImportantAreaManager::new();
        mgr.update_or_add(16, 24);
        mgr.update_or_add(17, 25); // same chunk (2, 3) -> updates timestamp only
        mgr.update_or_add(-1, 8); // chunk (-1, 1)
        assert_eq!(mgr.areas.len(), 2);
        assert_eq!((mgr.areas[0].sx, mgr.areas[0].sy), (2, 3));
        assert_eq!((mgr.areas[1].sx, mgr.areas[1].sy), (-1, 1));

        assert!(mgr.save_file(&dir, 240, false).unwrap());

        let mut loaded = ImportantAreaManager::new();
        assert_eq!(loaded.load_file(&dir).unwrap(), Some(240));
        assert_eq!(loaded.areas.len(), 2);
        assert_eq!((loaded.areas[0].sx, loaded.areas[0].sy), (2, 3));
        assert_eq!((loaded.areas[1].sx, loaded.areas[1].sy), (-1, 1));

        let future = loaded.areas[0].last_update_ms + loaded.timeout_ms + 1;
        assert_eq!(loaded.prune_expired(future), 2);
        assert!(loaded.areas.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
