//! IsoMovingObject fields and its actual ModData slot, over the original
//! reader/writer. Character and manager guards end before VM table codecs.
use super::*;
use pz_inventory::ModDataError;
use pz_kahlua::java_io::JavaReader;
use pz_kahlua::table::RetiredTableValues;
use pz_kahlua::table_save::{try_load_table, Flavor};
use pz_world::square::IsoDirections;

fn edit<I: ItemIdentity + 'static>(
    character: &GameCharacterRef<I>,
    f: impl FnOnce(&mut GameCharacter<I>),
) -> CharacterPersistenceResult<()> {
    let mut body = character
        .try_borrow_mut()
        .map_err(|_| RuntimeHostError::Borrowed("character moving load fields"))?;
    f(&mut body);
    Ok(())
}
pub(super) fn load<I: ItemIdentity + 'static>(
    character: &GameCharacterRef<I>,
    reader: &mut Reader<'_>,
    version: i32,
) -> CharacterPersistenceResult<()> {
    super::super::e(reader.f32())?;
    super::super::e(reader.f32())?;
    let x = super::super::e(reader.f32())?;
    edit(character, |body| {
        body.vars.next_x = x;
        body.vars.last_x = x;
        body.vars.x = x;
    })?;
    let y = super::super::e(reader.f32())?;
    edit(character, |body| {
        body.vars.next_y = y;
        body.vars.last_y = y;
        body.vars.y = y;
    })?;
    let z = super::super::e(reader.f32())?;
    edit(character, |body| {
        let z = z.clamp(-32.0, 31.0);
        body.vars.last_z = z;
        body.vars.z = z;
    })?;
    let dir = IsoDirections::ALL[(super::super::e(reader.i32())? & 7) as usize];
    edit(character, |body| {
        body.vars.dir = dir;
        let (x, y) = crate::character::zombie::math::vector_from_direction(dir);
        body.set_forward_direction(x, y);
    })?;
    if !super::super::e(reader.bool())? {
        return Ok(());
    }
    let table = {
        let mut body = character
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("character moving ModData field"))?;
        body.get_mod_data()
    };
    let mut input = JavaReader::new(reader.buf);
    input
        .set_position(reader.pos)
        .map_err(|_| unsupported("character ModData original reader position"))?;
    let mut removed = RetiredTableValues::new();
    let result = try_load_table(
        &table,
        &mut input,
        version,
        Flavor::ByteBuffer,
        &mut removed,
    );
    reader.pos = input.position();
    let notices = input.take_notices();
    let mut retired = RetiredComponents::new();
    retired.push_payload(Box::new(removed));
    pz_inventory::component_host::retire_components(retired);
    // Table writes and cursor prefixes survive either codec or logger failure.
    pz_inventory::component_host::emit_mod_data_notices(&notices)?;
    result.map_err(|error| ModDataError::from(error).into_host())?;
    Ok(())
}
pub(super) fn save<I: ItemIdentity + 'static>(
    character: &GameCharacterRef<I>,
    env: &SaveEnv<'_>,
    writer: &mut Writer,
) -> CharacterPersistenceResult<()> {
    let table = {
        let body = character
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("character moving save fields"))?;
        writer.i8(1);
        writer.i8(env
            .tables
            .class_id(if body.is_zombie() { "Zombie" } else { "Player" }));
        writer.f32(0.0);
        writer.f32(0.0);
        writer.f32(body.vars.x);
        writer.f32(body.vars.y);
        writer.f32(body.vars.z);
        writer.i32(body.vars.dir as i32);
        body.mod_data.clone()
    };
    let Some(table) = table else {
        writer.bool(false);
        return Ok(());
    };
    let empty = table
        .try_is_empty()
        .map_err(|error| ModDataError::from(error).into_host())?;
    writer.bool(!empty);
    if !empty {
        pz_inventory::moddata::save_live_table_to_writer(&table, writer)
            .map_err(ModDataError::into_host)?;
    }
    Ok(())
}
