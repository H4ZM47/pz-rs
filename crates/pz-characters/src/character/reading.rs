//! Scalar reading-state queries over the same character fields persistence
//! loads. No Java collection is copied or exposed by these helpers.
use super::GameCharacter;
use pz_entity::runtime_load::{HostResult, RuntimeHostError};
use std::cell::RefCell;
use std::rc::Rc;

pub trait ReadingHost {
    fn nights_survived(&self) -> HostResult<i32>;
    /// May initialize Sandbox through Lua. No character guard crosses it.
    fn literature_cooldown(&self) -> HostResult<i32>;
}

fn pin<I: 'static>(character: &Rc<RefCell<GameCharacter<I>>>) {
    let mut retired = pz_entity::RetiredComponents::new();
    retired.push_owner(pz_entity::EntityOwner::from_ref(character));
    pz_inventory::component_host::retire_components(retired);
}
fn null_key(operation: &'static str) -> RuntimeHostError {
    RuntimeHostError::Unsupported {
        operation,
        kind: None,
    }
}

pub fn is_literature_read<I: 'static>(
    character: &Rc<RefCell<GameCharacter<I>>>,
    title: Option<&str>,
    host: &dyn ReadingHost,
) -> HostResult<bool> {
    pin(character);
    let last_read = {
        let character = character
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("character readLiterature field"))?;
        // The current typed map cannot represent a Java null key. Never turn
        // that missing representation into an asserted absent title.
        let title = title.ok_or_else(|| null_key("readLiterature nullable Java map key"))?;
        character.read_literature.get(title).copied()
    };
    let Some(last_read) = last_read else {
        return Ok(false);
    };
    // Native unboxes the saved day before either singleton call. Sandbox
    // reentry can mutate the map without retargeting this captured scalar.
    let nights = host.nights_survived()?;
    let cooldown = host.literature_cooldown()?;
    Ok(nights < last_read.wrapping_add(cooldown))
}

pub fn is_print_media_read<I: 'static>(
    character: &Rc<RefCell<GameCharacter<I>>>,
    title: Option<&str>,
) -> HostResult<bool> {
    pin(character);
    let character = character
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("character readPrintMedia field"))?;
    let title = title.ok_or_else(|| null_key("readPrintMedia nullable Java set key"))?;
    Ok(character.read_print_media.contains_key(title))
}

pub fn already_read_pages<I: 'static>(
    character: &Rc<RefCell<GameCharacter<I>>>,
    full_type: Option<&str>,
) -> HostResult<i32> {
    pin(character);
    let character = character
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("character readBooks field"))?;
    // Native traverses in list order and uses each nonnull ReadBook.fullType
    // as the String.equals receiver, so a null argument matches no entry.
    Ok(character
        .read_books
        .iter()
        .find(|book| Some(book.full_type.as_str()) == full_type)
        .map_or(0, |book| book.already_read_pages))
}

#[cfg(test)]
mod tests;
