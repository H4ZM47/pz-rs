//! `ItemContainer` and `CompressIdenticalItems`.

use crate::env::ItemEnv;
use crate::io::{Reader, SaveResult, Writer};
use crate::saved::SavedItem;

/// `ItemContainer`: the fields saving and map loading use, and the items.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemContainer {
    /// `type`; `None` when the sprite has the `container` flag but no
    /// `container` property.
    pub kind: Option<String>,
    pub capacity: i32,
    pub container_position: Option<String>,
    pub freezer_position: Option<String>,
    /// `ID` (saved by dead bodies, mannequins and item containers).
    pub id: i32,
    pub explored: bool,
    pub has_been_looted: bool,
    /// `items`, in list order.
    pub items: Vec<SavedItem>,
}

impl ItemContainer {
    /// `new ItemContainer(type, square, parent)`.
    pub fn new(kind: Option<&str>) -> Self {
        ItemContainer {
            kind: kind.map(str::to_string),
            capacity: 50,
            container_position: None,
            freezer_position: None,
            id: 0,
            explored: false,
            has_been_looted: false,
            items: Vec::new(),
        }
    }

    /// `getItems().size()`.
    pub fn item_count(&self) -> usize {
        self.items.len()
    }

    /// `ItemContainer.load` (type, flags, items, capacity).
    pub fn load(&mut self, r: &mut Reader<'_>, wv: i32, env: &dyn ItemEnv) -> SaveResult<()> {
        self.kind = Some(r.string()?);
        self.explored = r.bool()?;
        self.items = load_items(r, wv, env)?;
        self.has_been_looted = r.bool()?;
        self.capacity = r.i32()?;
        Ok(())
    }

    /// `ItemContainer.save`.
    pub fn save(&self, w: &mut Writer, env: &dyn ItemEnv) -> SaveResult<()> {
        w.opt_string(self.kind.as_deref());
        w.bool(self.explored);
        save_items(w, &self.items, env)?;
        w.bool(self.has_been_looted);
        w.i32(self.capacity);
        Ok(())
    }
}

/// `CompressIdenticalItems.load`: every copy of a run is loaded from the
/// same record and given the saved ids; runs whose item cannot be created
/// are skipped.
pub fn load_items(r: &mut Reader<'_>, wv: i32, env: &dyn ItemEnv) -> SaveResult<Vec<SavedItem>> {
    let count = r.i16()?;
    let mut items = Vec::new();
    for _ in 0..count.max(0) {
        let identical = r.i32()?;
        let item = SavedItem::load(r, wv, env)?;
        let mut ids = Vec::new();
        for _ in 1..identical.max(1) {
            ids.push(r.i32()?);
        }
        if let Some(item) = item {
            for id in ids {
                let mut copy = item.clone();
                copy.set_id(id);
                items.push(copy);
            }
            let at = items.len() + 1 - identical.max(1) as usize;
            items.insert(at, item);
        }
    }
    Ok(items)
}

/// `CompressIdenticalItems.save`: items grouped by full type (in order of
/// first appearance), and consecutive items of a type whose saved bytes
/// (with the id zeroed) match written once with the other ids.
pub fn save_items(w: &mut Writer, items: &[SavedItem], env: &dyn ItemEnv) -> SaveResult<()> {
    let mut types: Vec<(String, Vec<usize>)> = Vec::new();
    for (i, it) in items.iter().enumerate() {
        let t = it.full_type(env);
        match types.iter_mut().find(|(n, _)| *n == t) {
            Some(e) => e.1.push(i),
            None => types.push((t, vec![i])),
        }
    }
    let records: Vec<Vec<u8>> = items
        .iter()
        .map(|i| i.record(env))
        .collect::<SaveResult<_>>()?;
    let zeroed = |i: usize| {
        let mut r = records[i].clone();
        r[3..7].fill(0);
        r
    };
    let count_at = w.pos();
    w.i16(0);
    let mut runs = 0i16;
    for (_, list) in &types {
        let mut m = 0;
        while m < list.len() {
            let first = list[m];
            let key = zeroed(first);
            let mut end = m + 1;
            while end < list.len()
                && !items[first].is_nonempty_container(env)
                && !items[list[end]].is_nonempty_container(env)
                && zeroed(list[end]) == key
            {
                end += 1;
            }
            w.i32((end - m) as i32);
            w.i32(records[first].len() as i32);
            w.bytes(&records[first]);
            for &o in &list[m + 1..end] {
                w.i32(items[o].id());
            }
            runs += 1;
            m = end;
        }
    }
    w.patch_i16(count_at, runs);
    Ok(())
}

/// `CompressIdenticalItems.save(output, item)`: one optional item.
pub fn save_single(w: &mut Writer, item: Option<&SavedItem>, env: &dyn ItemEnv) -> SaveResult<()> {
    match item {
        None => w.i16(0),
        Some(i) => {
            w.i16(1);
            w.i32(1);
            i.save(w, env)?;
        }
    }
    Ok(())
}
