//! The naming and literature `ItemCodeOnCreate` callbacks.
//!
//! Each body follows the 42.21 bytecode call for call, including the order
//! of the `Rand` draws and of the translator lookups. Draws come from the
//! shared `Rand.Next` generator, lists from [`super::on_create_data`].

use std::rc::Rc;

use pz_inventory::moddata::{get_mod_data_ref, set_mod_data_value_ref, ModDataError};
use pz_inventory::ItemKind;
use pz_kahlua::{LuaError, LuaResult, NativeCall, TableRef, Value};
use pz_text::JavaArg;

use super::on_create_data::{tables, Entry};
use super::{ItemRef, LuaInventoryItem};

const LITERATURE: &str = "zombie.inventory.types.Literature";
const CLOTHING: &str = "zombie.inventory.types.Clothing";

type Body = fn(&mut NativeCall<'_>, &ItemRef) -> LuaResult<()>;

/// The static parameter type of a callback.
#[derive(Clone, Copy)]
enum Param {
    Item,
    Literature,
    Clothing,
}

impl Param {
    fn class(self) -> Option<&'static str> {
        match self {
            Param::Item => None,
            Param::Literature => Some(LITERATURE),
            Param::Clothing => Some(CLOTHING),
        }
    }

    fn simple(self) -> &'static str {
        match self {
            Param::Item => "InventoryItem",
            Param::Literature => "Literature",
            Param::Clothing => "Clothing",
        }
    }
}

pub(super) fn unsupported(message: impl Into<String>) -> LuaError {
    LuaError::Runtime {
        message: message.into(),
        class: "pz.host.Unsupported",
    }
}

fn runtime(class: &'static str, message: impl Into<String>) -> LuaError {
    LuaError::Runtime {
        message: message.into(),
        class,
    }
}

fn borrowed(what: &'static str) -> LuaError {
    ModDataError::Borrowed(what).to_lua_error()
}

// ---- argument conversion ---------------------------------------------------------------------

/// Native reference conversion of the single parameter: the class check
/// comes before the excess-argument check, and a Java null is a valid
/// reference that faults in the body.
fn typed_argument(call: &NativeCall<'_>, param: Param) -> LuaResult<Option<ItemRef>> {
    if call.nargs() == 0 {
        return Err(LuaError::runtime("expected 1 argument, got 0"));
    }
    let argument = call.arg(0);
    let item = match &argument {
        Value::Nil => None,
        Value::Object(object) => {
            match (&**object as &dyn std::any::Any).downcast_ref::<LuaInventoryItem>() {
                Some(known) if param.class().map_or(true, |class| known.class == class) => {
                    Some(known.item.clone())
                }
                _ => return Err(type_error(&argument, param)),
            }
        }
        _ => return Err(type_error(&argument, param)),
    };
    if call.nargs() != 1 {
        return Err(LuaError::runtime(format!(
            "expected 1 argument, got {}",
            call.nargs()
        )));
    }
    Ok(item)
}

fn type_error(argument: &Value, param: Param) -> LuaError {
    // Value::Native retains JavaFunction's interface, not its concrete Java
    // implementation class.
    if matches!(argument, Value::Native(_)) {
        return unsupported(format!(
            "native JavaFunction implementation class in {} argument diagnostic",
            param.simple()
        ));
    }
    let class = match argument {
        Value::Object(object) => {
            match (&**object as &dyn std::any::Any).downcast_ref::<LuaInventoryItem>() {
                Some(item) => item.class.to_owned(),
                None => pz_kahlua::stdlib::java_class_name(argument),
            }
        }
        _ => pz_kahlua::stdlib::java_class_name(argument),
    };
    LuaError::runtime(format!(
        "expected argument of type {}, got {}",
        param.simple(),
        crate::expose::simple_name(&class)
    ))
}

fn invoke(
    call: &mut NativeCall<'_>,
    name: &'static str,
    param: Param,
    body: Body,
) -> LuaResult<usize> {
    let Some(item) = typed_argument(call, param)? else {
        return Err(unsupported(format!(
            "MethodCaller null-receiver NPE diagnostic state for {name}"
        )));
    };
    body(call, &item)?;
    Ok(0)
}

// ---- Java building blocks ---------------------------------------------------------------------

/// `Rand.Next(int max)`.
fn rand(max: i32) -> i32 {
    pz_core::random::next(max)
}

/// `Rand.NextBool(int)`.
fn rand_bool(inv_probability: i32) -> bool {
    rand(inv_probability) == 0
}

/// `Rand.NextInclusive(min, max)`.
fn rand_inclusive(min: i32, max: i32) -> i32 {
    pz_core::random::next_range(min, max + 1)
}

/// `Rand.Next(List)`: `list.get(Rand.Next(list.size()))`.
fn pick<T: Clone>(list: &[T]) -> LuaResult<T> {
    let index = rand(list.len() as i32);
    usize::try_from(index)
        .ok()
        .and_then(|i| list.get(i))
        .cloned()
        .ok_or_else(|| {
            runtime(
                "java.lang.IndexOutOfBoundsException",
                format!("Index {index} out of bounds for length {}", list.len()),
            )
        })
}

fn text(key: &str, args: &[JavaArg]) -> String {
    crate::bind_text::with_translator(|t| t.get_text(key, args)).unwrap_or_else(|| key.to_string())
}

fn text_plain(key: &str) -> String {
    text(key, &[])
}

fn arg(value: &str) -> JavaArg {
    JavaArg::Str(value.to_string())
}

/// `Translator.getRecipeName`.
fn recipe_name(id: &str) -> String {
    crate::bind_text::with_translator_mut(|t| t.get_recipe_name(id))
        .unwrap_or_else(|| id.to_string())
}

fn display_name(item: &ItemRef) -> LuaResult<String> {
    let body = item
        .try_borrow()
        .map_err(|_| borrowed("ItemCodeOnCreate getDisplayName"))?;
    Ok(crate::bind_item_name::display_name(
        &body,
        crate::bind_items::item_env().as_ref(),
    ))
}

fn item_type(item: &ItemRef) -> LuaResult<String> {
    let body = item
        .try_borrow()
        .map_err(|_| borrowed("ItemCodeOnCreate getType"))?;
    Ok(body.type_.clone())
}

fn set_name(item: &ItemRef, name: &str) -> LuaResult<()> {
    item.try_borrow_mut()
        .map_err(|_| borrowed("ItemCodeOnCreate setName"))?
        .set_name(name);
    pz_inventory::shared::observe_item(item);
    Ok(())
}

/// `getModData().rawset(key, value)`.
fn mod_set(item: &ItemRef, key: &str, value: Value) -> LuaResult<()> {
    set_mod_data_value_ref(item, key, Some(value)).map_err(|error| error.to_lua_error())
}

/// `setCustomName(true)`: the flag, then the name copied into the mod data.
fn set_custom_name(item: &ItemRef) -> LuaResult<()> {
    let name = {
        let mut body = item
            .try_borrow_mut()
            .map_err(|_| borrowed("ItemCodeOnCreate setCustomName"))?;
        body.custom_name = true;
        body.name.clone()
    };
    mod_set(item, "customName", Value::from(name))
}

fn has_tag(item: &ItemRef, class: &'static str, field: &'static str) -> LuaResult<bool> {
    let tables = tables()?;
    let id = tables.key(class, field)?;
    let body = item
        .try_borrow()
        .map_err(|_| borrowed("ItemCodeOnCreate hasTag"))?;
    Ok(body.has_tag(id))
}

/// `setTexture(Texture.getSharedTexture(name))`.
fn set_texture(item: &ItemRef, name: &str) -> LuaResult<()> {
    let texture = pz_render::get_shared_texture_ref(name).map_err(|e| {
        crate::bind_world_object::host_error(pz_inventory::item_texture::texture_host_error(e))
    })?;
    pz_inventory::item_texture::set_texture_ref(item, texture.as_ref())
        .map_err(crate::bind_world_object::host_error)
}

/// `setWorldStaticModel(ModelKey)`.
fn set_world_static_model(item: &ItemRef, field: &'static str) -> LuaResult<()> {
    let id = tables()?.key("ModelKey", field)?.to_string();
    mod_set(item, "worldStaticModel", Value::from(id))
}

/// `setStaticModel(ModelKey)`.
fn set_static_model(item: &ItemRef, field: &'static str) -> LuaResult<()> {
    let id = tables()?.key("ModelKey", field)?.to_string();
    mod_set(item, "staticModel", Value::from(id))
}

/// `setMediaName(item, name, key, value)`: the name, then one mod data entry.
fn set_media_name(item: &ItemRef, name: &str, key: &str, value: Value) -> LuaResult<()> {
    set_name(item, name)?;
    mod_set(item, key, value)
}

/// `getRandomForename(female) + " " + getRandomSurname()`.
fn random_person(female: bool) -> LuaResult<String> {
    let characters = crate::bind_characters::shared_characters();
    let characters = characters.borrow();
    let forename = characters
        .random_forename(female)
        .map_err(crate::bind_registry::java_error)?;
    let surname = characters
        .random_surname()
        .map_err(crate::bind_registry::java_error)?;
    Ok(format!("{forename} {surname}"))
}

fn registry(class: &'static str) -> LuaResult<Vec<Rc<Entry>>> {
    Ok(tables()?.values(class)?.to_vec())
}

fn entry_key(entry: &Entry, field: &str) -> String {
    entry.key(field).to_string()
}

// ---- literature, documents and keepsakes ------------------------------------------------------

fn stock_certificate(_: &mut NativeCall<'_>, item: &ItemRef) -> LuaResult<()> {
    let display = display_name(item)?;
    mod_set(item, "collectibleKey", Value::from(display.clone()))?;
    let tables = tables()?;
    let name;
    if rand_bool(2) {
        let business = pick(tables.list("STOCK_CERTIFICATE_2")?)?;
        name = text(
            "IGUI_ItemWithDisplayNameNoQuote",
            &[
                arg(&display),
                JavaArg::Str(text_plain(business.key("translation"))),
            ],
        );
        set_texture(item, "Item_StockCertificate2")?;
        set_world_static_model(item, "STOCK_CERTIFICATE_2")?;
    } else {
        let business = pick(tables.list("STOCK_CERTIFICATE_1")?)?;
        name = text(
            "IGUI_ItemWithDisplayNameNoQuote",
            &[
                arg(&display),
                JavaArg::Str(text_plain(business.key("translation"))),
            ],
        );
    }
    set_name(item, &name)?;
    set_custom_name(item)
}

fn paperwork(_: &mut NativeCall<'_>, item: &ItemRef) -> LuaResult<()> {
    let n = rand_inclusive(1, 6);
    set_texture(item, &format!("Item_Paperwork{n}"))?;
    let field = match n {
        1 => "PAPERWORK_1",
        2 => "PAPERWORK_2",
        3 => "PAPERWORK_3",
        4 => "PAPERWORK_4",
        5 => "PAPERWORK_5",
        _ => "PAPERWORK_6",
    };
    set_world_static_model(item, field)
}

fn monogram(_: &mut NativeCall<'_>, item: &ItemRef) -> LuaResult<()> {
    let display = display_name(item)?;
    let person = random_person(rand_bool(2))?;
    set_name(
        item,
        &text(
            "IGUI_ItemWithDisplayName",
            &[arg(&display), JavaArg::Str(person)],
        ),
    )
}

fn dog_tag_pet(_: &mut NativeCall<'_>, item: &ItemRef) -> LuaResult<()> {
    let pet = pick(tables()?.list("PET_NAMES")?)?;
    let display = display_name(item)?;
    let name = text(
        "IGUI_ItemWithDisplayName",
        &[
            arg(&display),
            JavaArg::Str(text_plain(pet.key("translationKey"))),
        ],
    );
    set_media_name(item, &name, "collectibleKey", Value::from(name.clone()))
}

/// The private `onCreatePhoto(item, type, translationKey)`.
fn photo_for(item: &ItemRef, kind: &str, key: &str) -> LuaResult<()> {
    let display = display_name(item)?;
    let title = text(
        "IGUI_PhotoOf",
        &[arg(&display), JavaArg::Str(text_plain(key))],
    );
    let id = format!("{kind}_{key}_{}", rand(1_000_000));
    set_media_name(item, &title, "literatureTitle", Value::from(id))?;
    mod_set(item, "collectibleKey", Value::from(key.to_string()))
}

fn photo_from_list(item: &ItemRef, kind: &str, list: &str) -> LuaResult<()> {
    let photo = pick(tables()?.list(list)?)?;
    photo_for(item, kind, photo.key("translationKey"))
}

fn photo(_: &mut NativeCall<'_>, item: &ItemRef) -> LuaResult<()> {
    photo_from_list(item, "OldPhoto", "PHOTOS")
}

fn secret_photo(_: &mut NativeCall<'_>, item: &ItemRef) -> LuaResult<()> {
    photo_from_list(item, "SecretPhoto", "SECRET_PHOTOS")
}

fn racy_photo(_: &mut NativeCall<'_>, item: &ItemRef) -> LuaResult<()> {
    photo_from_list(item, "RacyPhoto", "RACY_PHOTOS")
}

fn very_old_photo(_: &mut NativeCall<'_>, item: &ItemRef) -> LuaResult<()> {
    photo_from_list(item, "VeryOldPhoto", "VERY_OLD_PHOTOS")
}

fn photo_from_registry(item: &ItemRef, kind: &str, class: &'static str) -> LuaResult<()> {
    let entry = pick(&registry(class)?)?;
    photo_for(item, kind, entry.key("translationKey"))
}

fn doodle(_: &mut NativeCall<'_>, item: &ItemRef) -> LuaResult<()> {
    photo_from_registry(item, "Doodle", "Doodle")
}

fn doodle_kids(_: &mut NativeCall<'_>, item: &ItemRef) -> LuaResult<()> {
    photo_from_registry(item, "Doodle", "DoodleKids")
}

fn postcard(_: &mut NativeCall<'_>, item: &ItemRef) -> LuaResult<()> {
    photo_from_registry(item, "Postcard", "Postcard")
}

// ---- magazines and newspapers ------------------------------------------------------------------

/// `RecipeCodeHelper.DateResult`.
struct DateResult {
    year: i32,
    month: i32,
}

#[cfg(test)]
thread_local! {
    /// The year tests use instead of the sandbox options.
    pub(super) static TEST_YEAR: std::cell::Cell<Option<i32>> = const { std::cell::Cell::new(None) };
}

/// `SandboxOptions.getFirstYear() + startYear.getValue() - 1`.
fn sandbox_year(call: &mut NativeCall<'_>) -> LuaResult<i32> {
    #[cfg(test)]
    if let Some(year) = TEST_YEAR.with(std::cell::Cell::get) {
        return Ok(year);
    }
    let instance = crate::bind_sandbox::instance(call.thread)?;
    let Value::Object(object) = &instance else {
        return Err(unsupported("SandboxOptions instance"));
    };
    let Some(options) =
        (&**object as &dyn std::any::Any).downcast_ref::<crate::bind_sandbox::SandboxObj>()
    else {
        return Err(unsupported("SandboxOptions instance"));
    };
    let options = options.0.borrow();
    let start = options
        .option_by_name("StartYear")
        .ok_or_else(|| unsupported("SandboxOptions.StartYear"))?
        .config
        .get_int();
    Ok(options.first_year() + start - 1)
}

/// `ItemCodeOnCreate.getDate(item, minYear)`.
fn get_date(call: &mut NativeCall<'_>, item: &ItemRef, min_year: i32) -> LuaResult<DateResult> {
    let mut month = rand_inclusive(1, 7);
    let mut year = sandbox_year(call)?;
    if !has_tag(item, "ItemTag", "NEW")? {
        year = min_year + rand(year - min_year);
        if year != 1993 {
            month = rand_inclusive(1, 12);
        }
    }
    Ok(DateResult { year, month })
}

/// `setMagazineName(item, id, display, date)`.
fn set_magazine_name(item: &ItemRef, id: &str, display: &str, date: &DateResult) -> LuaResult<()> {
    let month = text_plain(&format!("Sandbox_StartMonth_option{}", date.month));
    let title = text(
        "IGUI_MagazineName",
        &[arg(display), JavaArg::Str(month), JavaArg::Int(date.year)],
    );
    let id = format!("{id}_{}_{}", date.month, date.year);
    set_media_name(item, &title, "literatureTitle", Value::from(id))
}

fn hottie_z(call: &mut NativeCall<'_>, item: &ItemRef) -> LuaResult<()> {
    let hunk = rand_bool(20);
    let date = get_date(call, item, if hunk { 1973 } else { 1953 })?;
    let mut display = display_name(item)?;
    if hunk {
        set_texture(item, "Item_MagazineNudie2")?;
        set_world_static_model(item, "HOTTIE_ZGROUND_2")?;
        set_static_model(item, "HOTTIE_Z2")?;
        display = text_plain("IGUI_MagazineTitle_HunkZ");
    }
    set_magazine_name(
        item,
        if hunk { "HunkZ" } else { "HottieZ" },
        &display,
        &date,
    )
}

fn tv_magazine(call: &mut NativeCall<'_>, item: &ItemRef) -> LuaResult<()> {
    let year = if has_tag(item, "ItemTag", "NEW")? {
        1993
    } else {
        1953
    };
    let date = get_date(call, item, year)?;
    let display = display_name(item)?;
    set_magazine_name(item, "TVMagazine", &display, &date)
}

fn subject_magazine(call: &mut NativeCall<'_>, item: &ItemRef) -> LuaResult<()> {
    let subjects = magazine_subjects(item)?;
    let magazines = registry("Magazine")?;
    let magazine = if subjects.is_empty() {
        pick(&magazines)?
    } else {
        // Magazine.getMagazineBySubject: per subject, the registered
        // magazines carrying it, concatenated without de-duplication.
        let matching: Vec<Rc<Entry>> = subjects
            .iter()
            .flat_map(|subject| {
                magazines
                    .iter()
                    .filter(move |m| m.subjects.contains(subject))
            })
            .cloned()
            .collect();
        pick(&matching)?
    };
    let key = magazine.key("translationKey").to_string();
    let display = text_plain(&key);
    let year = if has_tag(item, "ItemTag", "NEW")? {
        1993
    } else {
        1970
    };
    let date = get_date(call, item, year)?;
    set_magazine_name(item, &key, &display, &date)
}

/// The lower-cased paths of the script item's `magazineSubjects`.
fn magazine_subjects(item: &ItemRef) -> LuaResult<Vec<String>> {
    let body = item
        .try_borrow()
        .map_err(|_| borrowed("ItemCodeOnCreate getMagazineSubjects"))?;
    let script = body
        .template
        .script
        .as_ref()
        .ok_or_else(|| runtime("java.lang.NullPointerException", ""))?;
    let script = script.0.borrow();
    Ok(script
        .magazine_subjects
        .iter()
        .flatten()
        .map(|k| k.path().to_lowercase())
        .collect())
}

fn book_subjects(item: &ItemRef) -> LuaResult<Vec<String>> {
    let body = item
        .try_borrow()
        .map_err(|_| borrowed("ItemCodeOnCreate getBookSubjects"))?;
    let script = body
        .template
        .script
        .as_ref()
        .ok_or_else(|| runtime("java.lang.NullPointerException", ""))?;
    let script = script.0.borrow();
    Ok(script
        .book_subjects
        .iter()
        .flatten()
        .map(|k| k.path().to_lowercase())
        .collect())
}

fn subject_book(_: &mut NativeCall<'_>, item: &ItemRef) -> LuaResult<()> {
    // InventoryItem.getCoverType: a hard-cover tag and a soft-cover tag.
    let hard = has_tag(item, "ItemTag", "HARDCOVER")?
        || has_tag(item, "ItemTag", "HOLLOW_BOOK")?
        || has_tag(item, "ItemTag", "FANCY_BOOK")?;
    let soft = has_tag(item, "ItemTag", "SOFTCOVER")?;
    let cover = if hard == soft {
        "BOTH"
    } else if hard {
        "HARDCOVER"
    } else {
        "SOFTCOVER"
    };
    let subjects = book_subjects(item)?;
    let books: Vec<Rc<Entry>> = registry("Book")?
        .into_iter()
        // CoverType.matches: either side BOTH, or equal.
        .filter(|b| cover == "BOTH" || b.cover == "BOTH" || b.cover == cover)
        .filter(|b| b.subjects.iter().any(|s| subjects.contains(s)))
        .collect();
    let book = pick(&books)?;
    let key = book.key("translationKey").to_string();
    let display = display_name(item)?;
    set_name(
        item,
        &text(
            "IGUI_MagazineNameNoIssue",
            &[arg(&display), JavaArg::Str(text_plain(&key))],
        ),
    )?;
    mod_set(item, "literatureTitle", Value::from(key))
}

fn old_newspaper(_: &mut NativeCall<'_>, item: &ItemRef) -> LuaResult<()> {
    let paper = pick(tables()?.list("OLD_NEWSPAPER")?)?;
    let display = display_name(item)?;
    let name = text(
        "IGUI_Newspaper_Name",
        &[
            arg(&display),
            JavaArg::Str(text_plain(paper.key("translationKey"))),
        ],
    );
    set_media_name(item, &name, "collectibleKey", Value::from(name.clone()))
}

/// `RecipeCodeHelper.setPrintMediaInfo(item, title, info, text, id)`.
fn set_print_media_info(
    item: &ItemRef,
    title: &str,
    info: &str,
    text: &str,
    id: &str,
) -> LuaResult<()> {
    let table = TableRef::new();
    table.set_str("title", Value::from(title.to_string()));
    table.set_str("info", Value::from(info.to_string()));
    table.set_str("id", Value::from(id.to_string()));
    table.set_str("text", Value::from(text.to_string()));
    mod_set(item, "printMedia", Value::Table(table))?;
    mod_set(item, "literatureTitle", Value::from(title.to_string()))
}

/// `setFlierName(id, title, info, text, item)`.
fn set_flier_name(
    item: &ItemRef,
    id: &str,
    title: &str,
    info: &str,
    text_key: &str,
) -> LuaResult<()> {
    let display = display_name(item)?;
    let name = text(
        "IGUI_Newspaper_Name",
        &[arg(&display), JavaArg::Str(text_plain(title))],
    );
    set_print_media_info(item, title, info, text_key, id)?;
    set_name(item, &name)
}

fn print_media(item: &ItemRef, entry: &Entry) -> LuaResult<()> {
    set_flier_name(
        item,
        &entry.path(),
        entry.key("translationKey"),
        entry.key("translationInfoKey"),
        entry.key("translationTextKey"),
    )
}

fn brochure(_: &mut NativeCall<'_>, item: &ItemRef) -> LuaResult<()> {
    let entry = pick(&registry("Brochure")?)?;
    print_media(item, &entry)
}

fn flier(_: &mut NativeCall<'_>, item: &ItemRef) -> LuaResult<()> {
    let entry = pick(&registry("Flier")?)?;
    print_media(item, &entry)
}

/// The registered `Flier.NOLANS_USED_CARS`.
fn nolans_used_cars() -> LuaResult<Rc<Entry>> {
    tables()?
        .registry("Flier")?
        .field("NOLANS_USED_CARS")
        .cloned()
        .ok_or_else(|| unsupported("Flier.NOLANS_USED_CARS not loaded from the installed jar"))
}

fn flier_nolan(_: &mut NativeCall<'_>, item: &ItemRef) -> LuaResult<()> {
    let nolan = nolans_used_cars()?;
    print_media(item, &nolan)
}

/// `RecipeCodeHelper.nameNewspaper(item, newspaper)`.
fn name_newspaper(item: &ItemRef, paper: &Entry) -> LuaResult<()> {
    let mut issue = pick(&paper.issues)?;
    if has_tag(item, "ItemTag", "NEWSPAPER_NEW")? {
        issue = paper.issues.last().cloned().ok_or_else(|| {
            runtime(
                "java.lang.IndexOutOfBoundsException",
                "Index -1 out of bounds for length 0",
            )
        })?;
    }
    let title = format!("Print_Media_{issue}_title");
    set_name(item, &text_plain(&title))?;
    set_print_media_info(
        item,
        &title,
        &format!("Print_Media_{issue}_info"),
        &format!("Print_Text_{issue}_info"),
        &paper.path(),
    )
}

fn newspaper_named(item: &ItemRef, field: &str) -> LuaResult<()> {
    let tables = tables()?;
    let paper = tables
        .registry("Newspaper")?
        .field(field)
        .cloned()
        .ok_or_else(|| {
            unsupported(format!(
                "Newspaper.{field} not loaded from the installed jar"
            ))
        })?;
    name_newspaper(item, &paper)
}

fn dispatch_new_newspaper(_: &mut NativeCall<'_>, item: &ItemRef) -> LuaResult<()> {
    newspaper_named(item, "NATIONAL_DISPATCH")
}

fn herald_new_newspaper(_: &mut NativeCall<'_>, item: &ItemRef) -> LuaResult<()> {
    newspaper_named(item, "KENTUCKY_HERALD")
}

fn knews_new_newspaper(_: &mut NativeCall<'_>, item: &ItemRef) -> LuaResult<()> {
    newspaper_named(item, "KNOX_KNEWS")
}

fn times_new_newspaper(_: &mut NativeCall<'_>, item: &ItemRef) -> LuaResult<()> {
    newspaper_named(item, "LOUISVILLE_SUN_TIMES")
}

fn recent_newspaper(_: &mut NativeCall<'_>, item: &ItemRef) -> LuaResult<()> {
    let paper = pick(&registry("Newspaper")?)?;
    name_newspaper(item, &paper)
}

/// `nameComicBook(item, comic)`.
fn name_comic_book(item: &ItemRef, comic: &Entry) -> LuaResult<()> {
    let issues = comic.number;
    let key = entry_key(comic, "translationKey");
    if issues == 0 {
        let display = display_name(item)?;
        set_name(
            item,
            &text(
                "IGUI_MagazineNameNoIssue",
                &[arg(&display), JavaArg::Str(text_plain(&key))],
            ),
        )?;
        return mod_set(item, "literatureTitle", Value::from(key));
    }
    let issue = rand(issues);
    mod_set(
        item,
        "literatureTitle",
        Value::from(format!("{key}#{issue}")),
    )?;
    let width = issues.to_string().len();
    let number = format!("#{issue:0width$}");
    let display = display_name(item)?;
    set_name(
        item,
        &text(
            "IGUI_MagazineName",
            &[
                arg(&display),
                JavaArg::Str(text_plain(&key)),
                JavaArg::Str(number),
            ],
        ),
    )
}

fn comic_book(_: &mut NativeCall<'_>, item: &ItemRef) -> LuaResult<()> {
    let comic = pick(&registry("ComicBook")?)?;
    name_comic_book(item, &comic)
}

fn comic_book_retail(_: &mut NativeCall<'_>, item: &ItemRef) -> LuaResult<()> {
    let comics = registry("ComicBook")?;
    let mut comic = pick(&comics)?;
    while !comic.in_print {
        comic = pick(&comics)?;
    }
    name_comic_book(item, &comic)
}

// ---- cards, catalogues and keepsakes ------------------------------------------------------------

/// `setBusinessCardName(item, job)`.
fn set_business_card_name(item: &ItemRef, job: &str) -> LuaResult<()> {
    let display = display_name(item)?;
    let person = random_person(rand_bool(2))?;
    set_name(
        item,
        &text(
            "IGUI_ItemWithDisplayNameAndJob",
            &[arg(&display), JavaArg::Str(person), arg(job)],
        ),
    )
}

fn business_card(_: &mut NativeCall<'_>, item: &ItemRef) -> LuaResult<()> {
    let job = pick(&registry("Job")?)?;
    set_business_card_name(item, &text_plain(job.key("translationKey")))
}

fn business_card_nolan(_: &mut NativeCall<'_>, item: &ItemRef) -> LuaResult<()> {
    let nolan = nolans_used_cars()?;
    set_business_card_name(item, &text_plain(nolan.key("translationKey")))
}

/// `setLiteratureNameBasic(item, key)`.
fn set_literature_name_basic(item: &ItemRef, key: &str) -> LuaResult<()> {
    let display = display_name(item)?;
    set_name(
        item,
        &text(
            "IGUI_MagazineNameNoIssue",
            &[arg(&display), JavaArg::Str(text_plain(key))],
        ),
    )?;
    let title = format!("{}_{key}", item_type(item)?);
    mod_set(item, "literatureTitle", Value::from(title))
}

fn catalogue(_: &mut NativeCall<'_>, item: &ItemRef) -> LuaResult<()> {
    let entry = pick(&registry("Catalogue")?)?;
    set_literature_name_basic(item, entry.key("translationKey"))
}

fn rpg_manual(_: &mut NativeCall<'_>, item: &ItemRef) -> LuaResult<()> {
    let entry = pick(&registry("RpgManual")?)?;
    set_literature_name_basic(item, entry.key("translationKey"))
}

fn generic_mail(_: &mut NativeCall<'_>, item: &ItemRef) -> LuaResult<()> {
    let letter = tables()?.weighted("GENERIC_MAIL")?.get_random()?;
    set_name(item, &text_plain(letter.key("translationKey")))
}

fn letter_handwritten(_: &mut NativeCall<'_>, item: &ItemRef) -> LuaResult<()> {
    let letter = tables()?.weighted("LETTER_HANDWRITTEN")?.get_random()?;
    set_name(item, &text_plain(letter.key("translationKey")))
}

fn locket(_: &mut NativeCall<'_>, item: &ItemRef) -> LuaResult<()> {
    let locket = pick(&registry("Locket")?)?;
    let key = entry_key(&locket, "translationKey");
    let display = display_name(item)?;
    set_name(
        item,
        &text(
            "IGUI_LocketText",
            &[arg(&display), JavaArg::Str(text_plain(&key))],
        ),
    )?;
    mod_set(
        item,
        "collectibleKey",
        Value::from(format!("{}_{key}", item_type(item)?)),
    )
}

fn snow_globe(_: &mut NativeCall<'_>, item: &ItemRef) -> LuaResult<()> {
    let postcard = tables()?.weighted("POSTCARD")?.get_random()?;
    let display = display_name(item)?;
    let name = text(
        "IGUI_SnowGlobeOf",
        &[
            arg(&display),
            JavaArg::Str(text_plain(postcard.key("translationKey"))),
        ],
    );
    set_name(item, &name)?;
    mod_set(item, "collectibleKey", Value::from(name))
}

// ---- recipes ------------------------------------------------------------------------------------

/// `Literature.setLearnedRecipes(list)`.
fn set_learned_recipes(item: &ItemRef, recipes: Vec<String>) -> LuaResult<()> {
    let mut body = item
        .try_borrow_mut()
        .map_err(|_| borrowed("ItemCodeOnCreate setLearnedRecipes"))?;
    if let ItemKind::Literature(data) = &mut body.kind {
        data.learned_recipes = Some(recipes);
    }
    drop(body);
    pz_inventory::shared::observe_item(item);
    Ok(())
}

fn learned_recipes(item: &ItemRef) -> LuaResult<Vec<String>> {
    let body = item
        .try_borrow()
        .map_err(|_| borrowed("ItemCodeOnCreate getLearnedRecipes"))?;
    Ok(match &body.kind {
        ItemKind::Literature(data) => data.learned_recipes.clone().unwrap_or_default(),
        _ => Vec::new(),
    })
}

fn recipe_clipping(_: &mut NativeCall<'_>, item: &ItemRef) -> LuaResult<()> {
    let id = pick(tables()?.recipes("FOOD_RECIPES")?)?;
    let recipe = recipe_name(&id);
    // The original stores the translated name as the learned recipe.
    set_learned_recipes(item, vec![recipe.clone()])?;
    let display = display_name(item)?;
    let name = text(
        "IGUI_MagazineNameNoIssue",
        &[arg(&display), JavaArg::Str(recipe)],
    );
    set_name(item, &name)?;
    mod_set(item, "collectibleKey", Value::from(name))
}

/// `setSchematicLearnedRecipes(item, list, chance)`.
fn set_schematic_learned_recipes(item: &ItemRef, list: &str, chance: i32) -> LuaResult<()> {
    let tables = tables()?;
    let list = tables.recipes(list)?;
    let mut count = 1;
    if rand(100) < chance {
        count = 5.min(rand_inclusive(2, list.len() as i32));
    }
    set_learned_recipes(item, Vec::new())?;
    for _ in 0..count {
        // getLearnedRecipes() reads (and creates) the mod data table first.
        get_mod_data_ref(item).map_err(|error| error.to_lua_error())?;
        let id = pick(list)?;
        let mut recipes = learned_recipes(item)?;
        recipes.push(id);
        set_learned_recipes(item, recipes)?;
    }
    get_mod_data_ref(item).map_err(|error| error.to_lua_error())?;
    let recipes = learned_recipes(item)?;
    if recipes.len() == 1 {
        let display = display_name(item)?;
        let name = text(
            "IGUI_MagazineNameNoIssue",
            &[arg(&display), JavaArg::Str(recipe_name(&recipes[0]))],
        );
        set_name(item, &name)?;
        mod_set(item, "collectibleKey", Value::from(name))?;
    }
    Ok(())
}

macro_rules! schematic {
    ($fn:ident, $list:literal, $chance:literal) => {
        fn $fn(_: &mut NativeCall<'_>, item: &ItemRef) -> LuaResult<()> {
            set_schematic_learned_recipes(item, $list, $chance)
        }
    };
}

schematic!(explosives_schematic, "EXPLOSIVE_SCHEMATICS", 40);
schematic!(melee_weapon_schematic, "MELEE_WEAPON_SCHEMATICS", 30);
schematic!(
    blacksmith_tools_schematic,
    "BLACKSMITH_TOOLS_SCHEMATICS",
    50
);
schematic!(armor_schematic, "ARMOR_SCHEMATICS", 30);
schematic!(cookware_schematic, "COOKWARE_SCHEMATIC", 40);
schematic!(survival_schematic, "SURVIVAL_SCHEMATICS", 40);
schematic!(sewing_pattern, "SEWING_PATTERNS", 0);

// ---- not ported ---------------------------------------------------------------------------------

// ---- masks, fluids and tickets ----------------------------------------------------------------

/// `createFilterForMask(clothing, filterId, key)`: records the filter id, then
/// a random fill level as both the mod data entry and the used delta.
fn create_filter_for_mask(item: &ItemRef, filter_id: &str, key: &str) -> LuaResult<()> {
    mod_set(item, key, Value::from(filter_id.to_string()))?;
    let delta = rand(1000) as f32 / 1000.0;
    mod_set(item, "usedDelta", Value::Num(f64::from(delta)))?;
    item.try_borrow_mut()
        .map_err(|_| borrowed("ItemCodeOnCreate setUsedDelta"))?
        .try_set_used_delta(delta)
        .map_err(|error| error.to_lua_error())
}

fn gas_mask(_: &mut NativeCall<'_>, item: &ItemRef) -> LuaResult<()> {
    let tables = tables()?;
    let drainable =
        |field: &'static str| tables.key("ItemKey$Drainable", field).map(str::to_string);
    if has_tag(item, "ItemTag", "GAS_MASK")? {
        create_filter_for_mask(item, &drainable("GASMASK_FILTER")?, "filterType")
    } else if has_tag(item, "ItemTag", "RESPIRATOR")? {
        create_filter_for_mask(item, &drainable("RESPIRATOR_FILTERS")?, "filterType")
    } else if has_tag(item, "ItemTag", "IMPROVISED_GAS_MASK")? {
        create_filter_for_mask(item, &drainable("RAG_FILTER")?, "filterType")
    } else if has_tag(item, "ItemTag", "SCBA")? {
        create_filter_for_mask(item, &drainable("OXYGEN_TANK")?, "tankType")
    } else {
        Ok(())
    }
}

/// `item.getFluidContainer()` with the original's null fault.
fn fluid_component(item: &ItemRef) -> LuaResult<pz_entity::ComponentRef> {
    item.try_borrow()
        .map_err(|_| borrowed("ItemCodeOnCreate getFluidContainer"))?
        .get_fluid_ref()
        .map_err(|error| runtime("java.lang.RuntimeException", format!("{error:?}")))?
        .ok_or_else(|| {
            runtime(
                "java.lang.NullPointerException",
                "getFluidContainer() is null",
            )
        })
}

/// `getFluidContainer().getPrimaryFluid()`: the fluid type string and the
/// fluid's own colour, or `None` for an empty container.
fn primary_fluid(item: &ItemRef) -> LuaResult<Option<(String, [f32; 4])>> {
    use pz_entity::{FluidContainer, Lookup};
    let body = item
        .try_borrow()
        .map_err(|_| borrowed("ItemCodeOnCreate getPrimaryFluid"))?;
    let Some(container): Option<FluidContainer> =
        crate::bind_item_container::fluid_container(&body)
    else {
        return Err(runtime(
            "java.lang.NullPointerException",
            "getFluidContainer() is null",
        ));
    };
    if container.is_empty() || container.fluids.is_empty() {
        return Ok(None);
    }
    let mut primary = &container.fluids[0];
    for test in &container.fluids {
        if test.amount > primary.amount {
            primary = test;
        }
    }
    let Some(name) = crate::bind_item_container::fluid_name(primary) else {
        return Ok(None);
    };
    let env = super::item_env();
    let fallback = pz_entity::BasicContext::default();
    let ctx = env.entity().unwrap_or(&fallback);
    match ctx.fluid_by_name(&name) {
        Lookup::Found(info) => Ok(Some((name, info.color))),
        _ => Err(unsupported(format!(
            "Fluid.getColor: fluid \"{name}\" is not in the script registry"
        ))),
    }
}

/// `getFluidContainer().getColor()`.
fn container_color(item: &ItemRef) -> LuaResult<[f32; 4]> {
    let component = fluid_component(item)?;
    let value = crate::bind_fluid_container::cached_color(&component)?;
    crate::bind_misc_classes::try_color_of(&value)?.ok_or_else(|| {
        runtime(
            "java.lang.NullPointerException",
            "FluidContainer.getColor() is null",
        )
    })
}

fn apply_custom_color(item: &ItemRef, rgba: [f32; 4]) -> LuaResult<()> {
    let color = crate::bind_misc_classes::Color::rgba(rgba[0], rgba[1], rgba[2], rgba[3]);
    super::native_item_code_on_create::set_custom_color(item, &color)
}

fn hair_dye_bottle(_: &mut NativeCall<'_>, item: &ItemRef) -> LuaResult<()> {
    if primary_fluid(item)?.is_none() {
        return Ok(());
    }
    let color = container_color(item)?;
    {
        let mut body = item
            .try_borrow_mut()
            .map_err(|_| borrowed("ItemCodeOnCreate setColorRed"))?;
        body.color_red = color[0];
        body.color_green = color[1];
        body.color_blue = color[2];
    }
    pz_inventory::shared::observe_item(item);
    Ok(())
}

/// `getColorForFluid(fluid, container colour)`.
fn color_for_fluid(
    tables: &super::on_create_data::Tables,
    fluid: &str,
    container: [f32; 4],
) -> LuaResult<[f32; 4]> {
    let is = |field: &'static str| tables.key("FluidKey", field).map(|id| id == fluid);
    Ok(if is("COLA")? {
        [1.0, 0.0, 0.0, 1.0]
    } else if is("GINGER_ALE")? {
        [0.0, 0.0, 1.0, 1.0]
    } else if is("SODA_LIME")? {
        [0.0, 1.0, 0.0, 1.0]
    } else if is("SODA_POP")? {
        [1.0, 0.0, 0.8, 1.0]
    } else {
        container
    })
}

fn pop_bottle(_: &mut NativeCall<'_>, item: &ItemRef) -> LuaResult<()> {
    let Some((fluid, _)) = primary_fluid(item)? else {
        return Err(runtime(
            "java.lang.NullPointerException",
            "getPrimaryFluid() is null",
        ));
    };
    let tables = tables()?;
    let index = i32::from(tables.key("FluidKey", "SODA_POP")? == fluid);
    item.try_borrow_mut()
        .map_err(|_| borrowed("ItemCodeOnCreate setModelIndex"))?
        .model_index = index;
    let color = color_for_fluid(&tables, &fluid, container_color(item)?)?;
    apply_custom_color(item, color)
}

fn soda_can(_: &mut NativeCall<'_>, item: &ItemRef) -> LuaResult<()> {
    let Some((_, color)) = primary_fluid(item)? else {
        return Err(runtime(
            "java.lang.NullPointerException",
            "getPrimaryFluid() is null",
        ));
    };
    apply_custom_color(item, color)
}

/// `RecipeCodeHelper.ticketWinnings` in its `Map.ofEntries` order. The JDK
/// shuffles an immutable map's iteration order per run, so no order is
/// "the" original one; the weights are the original's.
const TICKET_WINNINGS: [(&str, i32); 11] = [
    ("$1", 500),
    ("$2", 300),
    ("$5", 200),
    ("$10", 100),
    ("$20", 50),
    ("$50", 25),
    ("$100", 10),
    ("$500", 5),
    ("$1000", 2),
    ("$5000", 1),
    ("$10000", 1),
];

/// `scratchTicketWinner(null, ticket)`: a weighted winning, then the name,
/// texture and world model of a winning ticket. The null player skips the
/// halo text.
fn scratch_ticket_winner(_: &mut NativeCall<'_>, item: &ItemRef) -> LuaResult<()> {
    let total: i32 = TICKET_WINNINGS.iter().map(|(_, w)| w).sum();
    let roll = rand(total);
    let mut winning = None;
    let mut running = 0;
    for (name, weight) in TICKET_WINNINGS {
        running += weight;
        if roll < running {
            winning = Some(name);
            break;
        }
    }
    let display = display_name(item)?;
    let name = text(
        "IGUI_ScratchingTicketNameWinner",
        &[
            arg(&display),
            winning.map_or(JavaArg::Null, |w| JavaArg::Str(w.to_string())),
        ],
    );
    set_name(item, &name)?;
    set_texture(item, "Item_ScratchTicket_Winner")?;
    set_world_static_model(item, "SCRATCH_TICKET_WINNER")
}

macro_rules! not_ported {
    ($fn:ident, $why:literal) => {
        fn $fn(_: &mut NativeCall<'_>, _: &ItemRef) -> LuaResult<()> {
            Err(unsupported($why))
        }
    };
}

not_ported!(
    scarecrow,
    "ItemCodeOnCreate.onCreateScarecrow: IsoMannequin construction and setCustomSettingsToItem"
);
not_ported!(
    skeleton_display,
    "ItemCodeOnCreate.onCreateSkeletonDisplay: IsoMannequin construction and setCustomSettingsToItem"
);

const CALLBACKS: &[(&str, Param, Body)] = &[
    (
        "scratchTicketWinner",
        Param::Literature,
        scratch_ticket_winner,
    ),
    ("onCreateGasMask", Param::Clothing, gas_mask),
    ("onCreateHairDyeBottle", Param::Item, hair_dye_bottle),
    ("onCreatePopBottle", Param::Item, pop_bottle),
    ("onCreateSodaCan", Param::Item, soda_can),
    (
        "onCreateStockCertificate",
        Param::Literature,
        stock_certificate,
    ),
    ("onCreatePaperwork", Param::Literature, paperwork),
    ("onCreateMonogram", Param::Clothing, monogram),
    ("onCreateDogTagPet", Param::Item, dog_tag_pet),
    ("onCreatePhoto", Param::Item, photo),
    ("onCreateSecretPhoto", Param::Item, secret_photo),
    ("onCreateRacyPhoto", Param::Item, racy_photo),
    ("onCreateVeryOldPhoto", Param::Item, very_old_photo),
    ("onCreateHottieZ", Param::Item, hottie_z),
    ("onCreateOldNewspaper", Param::Item, old_newspaper),
    ("onCreateTVMagazine", Param::Item, tv_magazine),
    ("onCreateBrochure", Param::Item, brochure),
    ("onCreateFlier", Param::Item, flier),
    ("onCreateFlierNolan", Param::Item, flier_nolan),
    ("onCreateComicBook", Param::Item, comic_book),
    ("onCreateComicBookRetail", Param::Item, comic_book_retail),
    (
        "onCreateDispatchNewNewspaper",
        Param::Item,
        dispatch_new_newspaper,
    ),
    (
        "onCreateHeraldNewNewspaper",
        Param::Item,
        herald_new_newspaper,
    ),
    (
        "onCreateKnewsNewNewspaper",
        Param::Item,
        knews_new_newspaper,
    ),
    (
        "onCreateTimesNewNewspaper",
        Param::Item,
        times_new_newspaper,
    ),
    ("onCreateRecentNewspaper", Param::Item, recent_newspaper),
    ("onCreateSubjectBook", Param::Item, subject_book),
    ("onCreateSubjectMagazine", Param::Item, subject_magazine),
    ("onCreateBusinessCard", Param::Item, business_card),
    (
        "onCreateBusinessCardNolan",
        Param::Item,
        business_card_nolan,
    ),
    ("onCreateCatalogue", Param::Item, catalogue),
    ("onCreateRpgManual", Param::Item, rpg_manual),
    ("onCreateGenericMail", Param::Item, generic_mail),
    ("onCreateLetterHandwritten", Param::Item, letter_handwritten),
    ("onCreateLocket", Param::Item, locket),
    ("onCreateDoodle", Param::Item, doodle),
    ("onCreateDoodleKids", Param::Item, doodle_kids),
    ("onCreatePostcard", Param::Item, postcard),
    ("onCreateSnowGlobe", Param::Item, snow_globe),
    ("onCreateRecipeClipping", Param::Literature, recipe_clipping),
    (
        "onCreateExplosivesSchematic",
        Param::Literature,
        explosives_schematic,
    ),
    (
        "onCreateMeleeWeaponSchematic",
        Param::Literature,
        melee_weapon_schematic,
    ),
    (
        "onCreateBlacksmithToolsSchematic",
        Param::Literature,
        blacksmith_tools_schematic,
    ),
    ("onCreateArmorSchematic", Param::Literature, armor_schematic),
    (
        "onCreateCookwareSchematic",
        Param::Literature,
        cookware_schematic,
    ),
    (
        "onCreateSurvivalSchematic",
        Param::Literature,
        survival_schematic,
    ),
    ("onCreateSewingPattern", Param::Literature, sewing_pattern),
    ("onCreateScarecrow", Param::Item, scarecrow),
    ("onCreateSkeletonDisplay", Param::Item, skeleton_display),
];

pub(super) fn register(class: &crate::expose::ClassBuilder) {
    for &(name, param, body) in CALLBACKS {
        class.static_fn(name, move |c| invoke(c, name, param, body));
    }
}
