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
        self.load_saved(r, wv, env).map(|_| ())
    }

    /// `ItemContainer.save`.
    pub fn save(&self, w: &mut Writer, env: &dyn ItemEnv) -> SaveResult<()> {
        self.save_saved(w, env, None).map(|_| ())
    }

    /// `ItemContainer.load` returning `savedItems` (see
    /// [`load_items_saved`]).
    pub fn load_saved(&mut self, r: &mut Reader<'_>, wv: i32, env: &dyn ItemEnv) -> SaveResult<Vec<Option<usize>>> {
        self.kind = Some(r.string()?);
        self.explored = r.bool()?;
        let (items, saved) = load_items_saved(r, wv, env)?;
        self.items = items;
        self.has_been_looted = r.bool()?;
        self.capacity = r.i32()?;
        Ok(saved)
    }

    /// `ItemContainer.save(output, noCompress)` returning `savedItems` (see
    /// [`save_items_saved`]); `no_compress` is `noCompress.isEquipped`.
    pub fn save_saved(
        &self,
        w: &mut Writer,
        env: &dyn ItemEnv,
        no_compress: Option<&dyn Fn(&SavedItem) -> bool>,
    ) -> SaveResult<Vec<usize>> {
        w.opt_string(self.kind.as_deref());
        w.bool(self.explored);
        let saved = save_items_saved(w, &self.items, env, no_compress)?;
        w.bool(self.has_been_looted);
        w.i32(self.capacity);
        Ok(saved)
    }
}

/// `CompressIdenticalItems.load`: every copy of a run is loaded again
/// from the same record (`InventoryItem.loadItem` per copy, with its
/// `CreateItem` and load-time `Rand` draws) and given the saved ids; runs
/// whose item cannot be created are skipped.
pub fn load_items(r: &mut Reader<'_>, wv: i32, env: &dyn ItemEnv) -> SaveResult<Vec<SavedItem>> {
    Ok(load_items_saved(r, wv, env)?.0)
}

/// [`load_items`] with the list `CompressIdenticalItems.load` returns
/// (`savedItems`): one entry per saved item, the index of the item in the
/// returned items, or `None` for an item that could not be created. Saved
/// indices (worn and hand items) refer to this list.
pub fn load_items_saved(
    r: &mut Reader<'_>,
    wv: i32,
    env: &dyn ItemEnv,
) -> SaveResult<(Vec<SavedItem>, Vec<Option<usize>>)> {
    let count = r.i16()?;
    let mut items = Vec::new();
    let mut saved = Vec::new();
    for _ in 0..count.max(0) {
        let identical = r.i32()?;
        let rec = crate::saved::read_record(r)?;
        let reg = i16::from_be_bytes([rec[0], rec[1]]);
        let created = env.exists(reg);
        let first = items.len();
        if created {
            // The first load happens even for a run of 0 or less (its
            // draws are made; nothing is added).
            let item = SavedItem::decode(rec, wv, env);
            if identical > 0 {
                saved.push(Some(items.len()));
                items.push(item);
            }
            for _ in 1..identical {
                saved.push(Some(items.len()));
                items.push(SavedItem::decode(rec, wv, env));
            }
        } else {
            saved.extend((0..identical.max(0)).map(|_| None));
        }
        for i in 1..identical.max(1) as usize {
            let id = r.i32()?;
            if created {
                items[first + i].set_id(id);
            }
        }
    }
    Ok((items, saved))
}

/// `CompressIdenticalItems.save`: items grouped by full type (in order of
/// first appearance), and consecutive items of a type whose saved bytes
/// (with the id zeroed) match written once with the other ids.
pub fn save_items(w: &mut Writer, items: &[SavedItem], env: &dyn ItemEnv) -> SaveResult<()> {
    save_items_saved(w, items, env, None).map(|_| ())
}

/// [`save_items`] returning the list `CompressIdenticalItems.save` returns
/// (`savedItems`): the indices of `items` in the order they were written.
/// `no_compress` is `noCompress.isEquipped`: a run never starts at an
/// equipped item (one that follows the run's first item can still join it,
/// as in the original).
pub fn save_items_saved(
    w: &mut Writer,
    items: &[SavedItem],
    env: &dyn ItemEnv,
    no_compress: Option<&dyn Fn(&SavedItem) -> bool>,
) -> SaveResult<Vec<usize>> {
    let mut saved = Vec::with_capacity(items.len());
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
            let compress = no_compress.is_none_or(|f| !f(&items[first]));
            while compress
                && end < list.len()
                && !items[first].is_nonempty_container(env)
                && !items[list[end]].is_nonempty_container(env)
                && zeroed(list[end]) == key
            {
                end += 1;
            }
            w.i32((end - m) as i32);
            w.i32(records[first].len() as i32);
            w.bytes(&records[first]);
            saved.extend_from_slice(&list[m..end]);
            for &o in &list[m + 1..end] {
                w.i32(items[o].id());
            }
            runs += 1;
            m = end;
        }
    }
    w.patch_i16(count_at, runs);
    Ok(saved)
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
