//! `InventoryItem.DoTooltip` / `DoTooltipEmbedded`: the rows of an item's
//! tooltip, laid out with the [`pz_ui::tooltip`] layout API, with every
//! subclass's `DoTooltip(tooltip, layout)` override ([`kinds`]) and the
//! entity components' rows ([`components`]).
//!
//! Texts come from the shared `Translator` at run time; colours from
//! `Core`'s good/bad highlight colours. What the original reads from the
//! tooltip's character (`tooltipUI.getCharacter()`) goes through that
//! character's Lua binding ([`chr`]), so it works for whatever object the
//! tooltip was given.
//!
//! An original `NullPointerException` (a `Food` tooltip without a
//! character, a ranged weapon without `IsoPlayer.getInstance()`, clothing
//! without a ready `ClothingItem`) is returned as the same Lua error,
//! after the rows added so far.

pub(crate) mod chr;
pub mod components;
pub mod kinds;

use crate::bind_items::{item_env, ItemRef};
use pz_inventory::item::ItemKind;
use pz_inventory::{InventoryItem, ItemClass};
use pz_kahlua::{LuaError, LuaResult, Thread, Value};
use pz_ui::tooltip::{java_float_to_string, Layout, LayoutItem, ObjectTooltip};
use pz_ui::UIElement;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// `1, 1, 0.8, 1`: the label colour most rows use.
pub(crate) const LABEL: [f32; 4] = [1.0, 1.0, 0.8, 1.0];
pub(crate) const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

/// The `DebugOptions` the tooltip reads (`DebugOptions` is not ported;
/// the debug UI sets these).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TooltipDebugOptions {
    /// `DebugOptions.instance.tooltipInfo`.
    pub tooltip_info: bool,
    /// `DebugOptions.instance.tooltipAttributes`.
    pub tooltip_attributes: bool,
}

thread_local! {
    static DEBUG: Cell<TooltipDebugOptions> = const { Cell::new(TooltipDebugOptions {
        tooltip_info: false,
        tooltip_attributes: false,
    }) };
}

/// Sets the `DebugOptions` the tooltip reads.
pub fn set_debug_options(o: TooltipDebugOptions) {
    DEBUG.with(|d| d.set(o));
}

pub fn debug_options() -> TooltipDebugOptions {
    DEBUG.with(|d| d.get())
}

/// The `SandboxOptions` values the tooltip uses while
/// `SandboxOptions.instance` has not been created (the options'
/// defaults unless set).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SandboxFallback {
    /// `DiscomfortFactor`.
    pub discomfort_factor: f64,
    /// `EnableTaintedWaterText`.
    pub tainted_water_text: bool,
}

thread_local! {
    static SANDBOX: Cell<SandboxFallback> = const { Cell::new(SandboxFallback {
        discomfort_factor: 1.0,
        tainted_water_text: true,
    }) };
}

pub fn set_sandbox_fallback(s: SandboxFallback) {
    SANDBOX.with(|c| c.set(s));
}

pub fn sandbox_fallback() -> SandboxFallback {
    SANDBOX.with(|c| c.get())
}

/// A `ColorInfo` value (`r, g, b, a`).
pub type Col = [f32; 4];

/// `ColorInfo.interp(to, delta, dest)`.
pub(crate) fn interp(from: Col, to: Col, d: f32) -> Col {
    [
        from[0] + (to[0] - from[0]) * d,
        from[1] + (to[1] - from[1]) * d,
        from[2] + (to[2] - from[2]) * d,
        from[3] + (to[3] - from[3]) * d,
    ]
}

/// The game state one `DoTooltip` reads, gathered once.
pub struct Cx<'t> {
    pub t: &'t mut Thread,
    /// `tooltipUI.getCharacter()` (nil when unset).
    pub chr: Value,
    /// `Type.tryCastTo(character, IsoPlayer.class)` (nil when not a
    /// player).
    pub player: Value,
    /// `Core.getGoodHighlitedColor()` / `getBadHighlitedColor()`.
    pub good: Col,
    pub bad: Col,
    /// `Core.debug`.
    pub debug: bool,
    pub opts: TooltipDebugOptions,
    /// `LuaManager.GlobalObject.isAdmin()`.
    pub admin: bool,
    /// `SandboxOptions.instance.enableTaintedWaterText`.
    pub tainted_text: bool,
    /// `Core.getOptionShowItemModInfo()`.
    pub show_mod_info: bool,
    /// `Core.animalCheat`.
    pub animal_cheat: bool,
}

impl<'t> Cx<'t> {
    /// The state for a tooltip whose character is `chr`.
    pub fn new(t: &'t mut Thread, chr: Option<Value>) -> Self {
        let chr = chr.unwrap_or_default();
        let player = if chr::is_player(&chr) {
            chr.clone()
        } else {
            Value::Nil
        };
        let core = crate::bind_core::shared_core();
        let mut c = core.borrow_mut();
        let g = c.get_good_highlited_color();
        let b = c.get_bad_highlited_color();
        let show_mod_info = c.get_option_show_item_mod_info();
        let debug = c.debug;
        let animal_cheat = c.animal_cheat;
        // isAdmin(): a client whose role has admin rights; never here.
        let admin = false;
        drop(c);
        Cx {
            t,
            chr,
            player,
            good: [g.r, g.g, g.b, g.a],
            bad: [b.r, b.g, b.b, b.a],
            debug,
            opts: debug_options(),
            admin,
            tainted_text: crate::bind_sandbox::current_bool("EnableTaintedWaterText")
                .unwrap_or_else(|| sandbox_fallback().tainted_water_text),
            show_mod_info,
            animal_cheat,
        }
    }

    /// `Core.debug && DebugOptions.instance.tooltipInfo.getValue()`.
    pub fn tooltip_info(&self) -> bool {
        self.debug && self.opts.tooltip_info
    }

    /// `badHighlight.interp(goodHighlight, f)` (the red-to-green bars).
    pub fn bad_to_good(&self, f: f32) -> Col {
        interp(self.bad, self.good, f)
    }

    /// `goodHighlight.interp(badHighlight, f)`.
    pub fn good_to_bad(&self, f: f32) -> Col {
        interp(self.good, self.bad, f)
    }
}

/// `Translator.getText(key)`.
pub fn tr(key: &str) -> String {
    crate::bind_text::translate(key)
}

/// `Translator.getText(key, args...)`.
pub fn tr_args(key: &str, args: &[&str]) -> String {
    let a: Vec<pz_text::JavaArg> = args
        .iter()
        .map(|s| pz_text::JavaArg::Str(s.to_string()))
        .collect();
    crate::bind_text::with_translator(|t| t.get_text(key, &a)).unwrap_or_else(|| key.to_string())
}

/// `Translator.getText(key) + ":"`.
pub(crate) fn trc(key: &str) -> String {
    format!("{}:", tr(key))
}

/// `Translator.getFluidText(key)`.
pub(crate) fn fluid_text(key: &str) -> String {
    crate::bind_text::with_translator(|t| t.get_fluid_text(key)).unwrap_or_else(|| key.to_string())
}

/// `Translator.getRecipeName(name)`.
pub(crate) fn recipe_name(name: &str) -> String {
    crate::bind_text::with_translator_mut(|t| t.get_recipe_name(name))
        .unwrap_or_else(|| name.to_string())
}

/// `new NullPointerException()` as a Lua error.
pub(crate) fn npe() -> LuaError {
    LuaError::Runtime {
        message: "null".into(),
        class: "java.lang.NullPointerException",
    }
}

/// Row helpers over `LayoutItem`.
pub(crate) trait Row {
    fn label(&mut self, s: &str, c: Col) -> &mut Self;
    fn value(&mut self, s: &str, c: Col) -> &mut Self;
    fn progress(&mut self, f: f32, c: Col) -> &mut Self;
}

impl Row for LayoutItem {
    fn label(&mut self, s: &str, c: Col) -> &mut Self {
        self.set_label(s, c[0], c[1], c[2], c[3]);
        self
    }
    fn value(&mut self, s: &str, c: Col) -> &mut Self {
        self.set_value(s, c[0], c[1], c[2], c[3]);
        self
    }
    fn progress(&mut self, f: f32, c: Col) -> &mut Self {
        self.set_progress(f, c[0], c[1], c[2], c[3]);
        self
    }
}

/// `Integer.toString` / `String.valueOf(int)`.
pub(crate) fn int_str(v: i32) -> String {
    v.to_string()
}

/// `Float.toString`.
pub(crate) fn fstr(v: f32) -> String {
    java_float_to_string(v)
}

/// `InventoryItem.getCleanString(weight)`: two decimals, rounded half up.
pub fn clean_string(weight: f32) -> String {
    let v = (((weight as f64 + 0.005) * 100.0) as i32) as f32 / 100.0;
    java_float_to_string(v)
}

/// The item's script (`getScriptItem()`).
pub(crate) fn script(it: &InventoryItem) -> Option<Rc<RefCell<pz_scripts::objects::items::Item>>> {
    if let Some(s) = &it.template.script {
        return Some(s.0.clone());
    }
    let sm = crate::bind_registry::shared_script_manager();
    pz_characters::character::items::script_item(&sm, it)
}

/// A value of the item's script (the default when it has none).
pub(crate) fn sv<R: Default>(
    it: &InventoryItem,
    f: impl FnOnce(&pz_scripts::objects::items::Item) -> R,
) -> R {
    script(it).map(|s| f(&s.borrow())).unwrap_or_default()
}

/// `ScriptManager.instance.FindItem(name)` / `getItem(name)`.
pub(crate) fn find_script(name: &str) -> Option<Rc<RefCell<pz_scripts::objects::items::Item>>> {
    let sm = crate::bind_registry::shared_script_manager();
    sm.find_item(name, true)?
        .downcast::<pz_scripts::objects::items::Item>()
}

/// `Item.getDisplayName()` after `OnScriptsLoaded` (the translated item
/// name).
pub(crate) fn script_display_name(s: &pz_scripts::objects::items::Item) -> String {
    crate::bind_items::script_display_name(&s.module_dot_type).unwrap_or_else(|| s.display_name())
}

/// `InventoryItemFactory.CreateItem(type).getDisplayName()`: the created
/// item's (translated) name; `None` for an unknown type (the original
/// throws a `NullPointerException`).
pub(crate) fn created_display_name(full_type: &str) -> Option<String> {
    crate::bind_items::script_display_name(full_type)
}

/// `getModData().rawget(key)` as a string.
pub(crate) fn mod_str(it: &InventoryItem, key: &str) -> Option<String> {
    use pz_inventory::lua_table::{LuaKey, LuaValue};
    match it.table.as_ref()?.rawget(&LuaKey::Str(key.into()))? {
        LuaValue::Str(s) => Some(s.clone()),
        LuaValue::Num(n) => Some(pz_core::config::double_to_string(*n)),
        LuaValue::Bool(b) => Some(b.to_string()),
        _ => Some(String::new()),
    }
}

/// `getModData().rawget(key) != null`.
pub(crate) fn mod_has(it: &InventoryItem, key: &str) -> bool {
    use pz_inventory::lua_table::LuaKey;
    it.table
        .as_ref()
        .and_then(|t| t.rawget(&LuaKey::Str(key.into())))
        .is_some()
}

/// `getModData().rawget(key)` as a number.
pub(crate) fn mod_num(it: &InventoryItem, key: &str) -> Option<f64> {
    use pz_inventory::lua_table::{LuaKey, LuaValue};
    match it.table.as_ref()?.rawget(&LuaKey::Str(key.into()))? {
        LuaValue::Num(n) => Some(*n),
        _ => None,
    }
}

/// `getModData().rawset(key, value)`.
pub(crate) fn mod_set(it: &mut InventoryItem, key: &str, v: pz_inventory::lua_table::LuaValue) {
    use pz_inventory::lua_table::{LuaKey, LuaTable};
    it.table
        .get_or_insert_with(LuaTable::default)
        .rawset(LuaKey::Str(key.into()), Some(v));
}

/// `instanceof DrainableComboItem`.
pub(crate) fn is_drainable(it: &InventoryItem) -> bool {
    it.template.class == ItemClass::Drainable
}

/// `instanceof Clothing` (including `AlarmClockClothing`).
pub(crate) fn is_clothing(it: &InventoryItem) -> bool {
    matches!(
        it.kind,
        ItemKind::Clothing(_) | ItemKind::AlarmClockClothing(..)
    )
}

pub(crate) fn is_weapon(it: &InventoryItem) -> bool {
    matches!(it.kind, ItemKind::Weapon(_))
}

/// `getName()` / `getName(player)` (the player is unused), with the
/// subclasses' overrides ([`crate::bind_item_name::item_name`]).
pub fn item_name(it: &mut InventoryItem) -> String {
    crate::bind_item_name::item_name(it, item_env().as_ref())
}

/// The tooltip element and its `ObjectTooltip` state.
pub struct Tip<'a> {
    pub el: &'a UIElement,
    pub tt: &'a ObjectTooltip,
}

impl<'a> Tip<'a> {
    pub fn of(el: &'a UIElement) -> LuaResult<Tip<'a>> {
        let tt =
            ObjectTooltip::of(el).ok_or_else(|| LuaError::runtime("Expected ObjectTooltip"))?;
        Ok(Tip { el, tt })
    }
}

/// `drawTooltipItemTexture`: an icon scaled to fit, returning the drawn
/// width.
#[allow(clippy::too_many_arguments)]
fn draw_item_texture(
    tip: &Tip<'_>,
    tex: Option<&pz_render::Texture>,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
) -> f32 {
    tip.tt.draw_texture_scaled_aspect(
        tip.el, tex, x as f64, y as f64, w as f64, h as f64, 1.0, 1.0, 1.0, 1.0,
    );
    if let Some(t) = tex.filter(|t| t.width > 0 && t.height > 0) {
        let ratio = (w / t.width_orig() as f32).min(h / t.height_orig() as f32);
        return (t.width as f32 * ratio).ceil();
    }
    w
}

/// `getTex()`: the item's icon texture.
pub(crate) fn item_texture(it: &InventoryItem) -> Option<pz_render::Texture> {
    let name = match &it.texture_override {
        Some(n) => n.clone(),
        None => format!("Item_{}", it.template.icon.as_deref()?),
    };
    pz_render::get_shared_texture(&name)
}

/// `InventoryItem.DoTooltip(tooltipUI)`: `DoTooltipEmbedded(tooltipUI,
/// null, 0)`, and `InventoryContainer`'s override that draws the icons of
/// the bag's contents below.
pub fn do_tooltip(t: &mut Thread, item: &ItemRef, el: &UIElement) -> LuaResult<()> {
    let is_container = matches!(item.borrow().kind, ItemKind::Container(_));
    if !is_container {
        return do_tooltip_embedded(t, item, el, None, 0);
    }
    let tip = Tip::of(el)?;
    el.render(t);
    do_tooltip_embedded(t, item, el, None, 0)?;
    kinds::container_icons(&tip, item);
    Ok(())
}

/// `InventoryItem.DoTooltipEmbedded(tooltipUI, layoutOverride, offsetY)`.
/// With a layout the rows are added to it (its `offsetY` set) and nothing
/// is rendered.
pub fn do_tooltip_embedded(
    t: &mut Thread,
    item: &ItemRef,
    el: &UIElement,
    layout_override: Option<&mut Layout>,
    offset_y: i32,
) -> LuaResult<()> {
    let tip = Tip::of(el)?;
    el.render(t);
    let font = ObjectTooltip::font();
    let line_spacing = ObjectTooltip::line_spacing();
    let icon_size = line_spacing;
    let mut y = tip.tt.pad_top.get() + offset_y;
    let mut cx = Cx::new(t, tip.tt.character());
    let name = item_name(&mut item.borrow_mut());
    let pad_left = tip.tt.pad_left.get();
    tip.tt.draw_text(
        el,
        Some(font),
        &name,
        pad_left as f64,
        y as f64,
        1.0,
        1.0,
        0.8,
        1.0,
    );
    tip.tt.adjust_width(el, pad_left, &name);
    y += line_spacing + 5;
    let (extra, cookable, cooked, spices) = {
        let it = item.borrow();
        let f = match &it.kind {
            ItemKind::Food(f) => Some(f),
            _ => None,
        };
        (
            it.extra_items.clone().filter(|x| !x.is_empty()),
            f.is_some_and(|f| f.is_cookable),
            it.is_cooked(),
            f.and_then(|f| f.spices.clone()).filter(|s| !s.is_empty()),
        )
    };
    if let Some(extra) = extra {
        let label = tr("Tooltip_item_Contains");
        tip.tt.draw_text(
            el,
            Some(font),
            &label,
            pad_left as f64,
            y as f64,
            1.0,
            1.0,
            0.8,
            1.0,
        );
        let mut x = pad_left + pz_ui::draw::measure_string_x(Some(font), &label) + 4;
        let dy = (line_spacing - icon_size) / 2;
        let e = item_env();
        for id in extra {
            // The icon only draws; an item the environment cannot make
            // takes the icon size.
            let Some(mut x_item) = e
                .context()
                .item_type_from_id(id)
                .and_then(|full| e.context().create_item(&full))
            else {
                x += icon_size + 2;
                continue;
            };
            if let ItemKind::Food(f) = &mut x_item.kind {
                if (!cookable && f.is_cookable) || (cooked && f.is_cookable) {
                    f.cooked = true;
                }
            }
            let tex = item_texture(&x_item);
            let dx = draw_item_texture(
                &tip,
                tex.as_ref(),
                x as f32,
                (y + dy) as f32,
                icon_size as f32,
                icon_size as f32,
            );
            x += dx.ceil() as i32 + 2;
        }
        y += line_spacing + 5;
    }
    if let Some(spices) = spices {
        let label = tr("Tooltip_item_Spices");
        tip.tt.draw_text(
            el,
            Some(font),
            &label,
            pad_left as f64,
            y as f64,
            1.0,
            1.0,
            0.8,
            1.0,
        );
        let mut x = pad_left + pz_ui::draw::measure_string_x(Some(font), &label) + 4;
        let dy = (line_spacing - icon_size) / 2;
        let e = item_env();
        for s in spices {
            let Some(x_item) = e.context().create_item(&s) else {
                x += icon_size + 2;
                continue;
            };
            let tex = item_texture(&x_item);
            let dx = draw_item_texture(
                &tip,
                tex.as_ref(),
                x as f32,
                (y + dy) as f32,
                icon_size as f32,
                icon_size as f32,
            );
            x += dx.ceil() as i32 + 2;
        }
        y += line_spacing + 5;
    }
    let mut own;
    let rendering = layout_override.is_none();
    let layout: &mut Layout = match layout_override {
        Some(l) => {
            l.offset_y = y;
            l
        }
        None => {
            own = tip.tt.begin_layout();
            own.set_min_label_width(80);
            own.set_min_value_width(80);
            &mut own
        }
    };
    let r = rows(&mut cx, item, &tip, layout);
    r?;
    if rendering {
        let mut y = layout.render(tip.tt.pad_left.get(), y, el);
        let l = std::mem::take(layout);
        tip.tt.end_layout(l);
        y += tip.tt.pad_bottom.get();
        el.set_height(y as f64);
        if el.get_width() < 150.0 {
            el.set_width(150.0);
        }
    }
    Ok(())
}

/// The rows of `DoTooltipEmbedded`, from the script-name report to the
/// item's `tooltip` text.
fn rows(cx: &mut Cx<'_>, item: &ItemRef, tip: &Tip<'_>, layout: &mut Layout) -> LuaResult<()> {
    let e = item_env();
    let ctx = e.context();
    let owner = e.owner(item);
    let has_player = !cx.player.is_nil();
    // SandboxOptions.isUnstableScriptNameSpam() is always false.
    if has_player {
        let (researchable, favourite) = chr::recipe_flags(cx, item)?;
        if researchable > 0 {
            let key = if researchable == 1 {
                "Tooltip_item_CanResearch"
            } else {
                "Tooltip_item_CanResearchPlural"
            };
            layout
                .add_item()
                .label(&tr(key), [cx.good[0], cx.good[1], cx.good[2], 1.0]);
        }
        if favourite {
            layout.add_item().label(
                &tr("Tooltip_item_IsFavouriteInput"),
                [cx.good[0], cx.good[1], cx.good[2], 1.0],
            );
        }
        if chr::is_no_recipes(cx, item)? {
            layout.add_item().label(
                &tr("Tooltip_NoRecipes_More"),
                [cx.bad[0], cx.bad[1], cx.bad[2], 1.0],
            );
        }
        if chr::is_unwanted(cx, item)? {
            layout.add_item().label(
                &tr("Tooltip_Unwanted_More"),
                [cx.bad[0], cx.bad[1], cx.bad[2], 1.0],
            );
        }
    }
    {
        let it = item.borrow();
        let row = layout.add_item();
        row.label(&trc("Tooltip_item_Weight"), LABEL);
        let equipped = it.is_equipped(owner.as_ref());
        let full = it.get_full_type();
        let equipped_weight = || it.get_equipped_weight(ctx, owner.as_ref());
        let unequipped = || it.get_unequipped_weight(ctx, owner.as_ref());
        let hotbar = || it.get_hotbar_equipped_weight(ctx, owner.as_ref());
        if is_weapon(&it)
            || is_clothing(&it)
            || is_drainable(&it)
            || full.contains("Walkie")
            || it.is_key_ring()
        {
            if equipped || it.is_fake_equipped(owner.as_ref()) {
                row.value(
                    &format!(
                        "{}    ({} {})",
                        clean_string(equipped_weight()),
                        clean_string(unequipped()),
                        tr("Tooltip_item_Unequipped")
                    ),
                    WHITE,
                );
            } else if it.attached_slot > -1 {
                row.value(
                    &format!(
                        "{}    ({} {})",
                        clean_string(hotbar()),
                        clean_string(unequipped()),
                        tr("Tooltip_item_Unattached")
                    ),
                    WHITE,
                );
            } else {
                row.value(
                    &format!(
                        "{}    ({} {})",
                        clean_string(unequipped()),
                        clean_string(equipped_weight()),
                        tr("Tooltip_item_Equipped")
                    ),
                    WHITE,
                );
            }
        } else if it.template.class == ItemClass::Animal {
            row.set_value_right_no_plus_float(it.get_weight_with(ctx));
        } else {
            let mut weight = unequipped();
            if weight > 0.0 && weight < 0.01 {
                weight = 0.01;
            }
            if it.attached_slot > -1 {
                row.value(
                    &format!(
                        "{}    ({} {})",
                        clean_string(hotbar()),
                        clean_string(unequipped()),
                        tr("Tooltip_item_Unattached")
                    ),
                    WHITE,
                );
            } else {
                row.set_value_right_no_plus_float(weight);
            }
        }
    }
    if tip.tt.weight_of_stack() > 0.0 {
        let row = layout.add_item();
        row.label(&trc("Tooltip_item_StackWeight"), LABEL);
        let mut weight = tip.tt.weight_of_stack();
        if weight > 0.0 && weight < 0.01 {
            weight = 0.01;
        }
        row.set_value_right_no_plus_float(weight);
    }
    let (max_ammo, ammo_item, gun_type) = {
        let it = item.borrow();
        let s = script(&it);
        let s = s.as_ref().map(|s| s.borrow());
        (
            s.as_ref().map_or(0, |s| s.max_ammo),
            s.as_ref()
                .and_then(|s| s.ammo_type.as_ref().map(|k| k.item_key.clone())),
            s.as_ref().and_then(|s| s.gun_type.clone()),
        )
    };
    let weapon = is_weapon(&item.borrow());
    if max_ammo > 0 && !weapon {
        let it = item.borrow();
        layout
            .add_item()
            .label(&trc("Tooltip_weapon_AmmoCount"), LABEL)
            .value(&format!("{} / {}", it.current_ammo_count, max_ammo), WHITE);
    }
    if !weapon {
        if let Some(key) = ammo_item {
            let row = layout.add_item();
            row.label(&trc("ContextMenu_AmmoType"), LABEL);
            let name = key
                .as_deref()
                .and_then(created_display_name)
                .ok_or_else(npe)?;
            row.value(&tr(&name), WHITE);
        }
    }
    if let Some(guns) = gun_type {
        let module = item.borrow().get_module().to_string();
        let names: Vec<String> = guns
            .iter()
            .filter_map(|g| {
                let full = if g.contains('.') {
                    g.clone()
                } else {
                    format!("{module}.{g}")
                };
                let s = crate::bind_registry::shared_script_manager()
                    .get_script(pz_scripts::ScriptType::Item, &full)?
                    .downcast::<pz_scripts::objects::items::Item>()?;
                let s = s.borrow();
                Some(script_display_name(&s))
            })
            .collect();
        layout
            .add_item()
            .label(&trc("ContextMenu_GunType"), LABEL)
            .value(&names.join(", ").replace(['[', ']'], ""), WHITE);
    }
    if cx.tooltip_info() {
        let it = item.borrow();
        let mut dbg = |l: &str, v: f32| {
            layout
                .add_item()
                .label(l, LABEL)
                .set_value_right_no_plus_float(v);
        };
        dbg("getActualWeight()", it.get_actual_weight_with(ctx));
        dbg("getWeight()", it.get_weight_with(ctx));
        dbg(
            "getEquippedWeight()",
            it.get_equipped_weight(ctx, owner.as_ref()),
        );
        dbg(
            "getUnequippedWeight()",
            it.get_unequipped_weight(ctx, owner.as_ref()),
        );
        dbg(
            "getContentsWeight()",
            it.get_contents_weight_with(ctx, owner.as_ref()),
        );
        if matches!(it.kind, ItemKind::Key { .. }) || it.type_ == "Doorknob" {
            let key = match &it.kind {
                ItemKind::Key { key_id, .. } => *key_id,
                _ => it.key_id,
            };
            layout
                .add_item()
                .label("DBG: keyId", LABEL)
                .set_value_right_no_plus_int(key);
        }
        layout
            .add_item()
            .label("ID", LABEL)
            .set_value_right_no_plus_int(it.id);
        layout
            .add_item()
            .label("DictionaryID", LABEL)
            .set_value_right_no_plus_int(it.registry_id as i32);
        if let Some(c) = it.get_clothing_item(ctx.assets(), owner.as_ref()) {
            layout
                .add_item()
                .label("ClothingItem", WHITE)
                .value(&c.name, WHITE);
        }
    }
    if cx.tooltip_info() || cx.admin {
        // TODO(ItemPickerJava): getLootType(scriptItem) for the
        // "Sandbox_<type>LootNew" category; debug and MP admins only.
        let lt = "Other";
        layout
            .add_item()
            .label("Loot Category:", LABEL)
            .value(&tr(&format!("Sandbox_{lt}LootNew")), WHITE);
    }
    {
        let it = item.borrow();
        if is_drainable(&it) && !it.has_tag("base:hideremaining") {
            let f = it.get_current_uses_float();
            let c = cx.bad_to_good(f);
            layout
                .add_item()
                .label(&format!("{}: ", tr("IGUI_invpanel_Remaining")), LABEL)
                .progress(f, [c[0], c[1], c[2], 1.0]);
        }
        if it.is_tainted() && cx.tainted_text {
            let has_metal = sv(&it, |s| s.metal_value > 0.0) || it.has_tag("base:hasmetal");
            let key = if !has_metal {
                "Tooltip_item_TaintedWater"
            } else {
                "Tooltip_item_TaintedWater_Plastic"
            };
            layout.add_item().label(&tr(key), [1.0, 0.5, 0.5, 1.0]);
        }
        let cats = sv(&it, |s| s.forage_focus_categories.clone());
        for name in crate::item_tooltip::java_hash_set_order(&cats) {
            layout
                .add_item()
                .label(&trc("UI_search_mode_item_focus"), LABEL)
                .value(&tr(&format!("IGUI_SearchMode_Categories_{name}")), WHITE);
        }
        let fatigue = it.get_fatigue_change();
        if fatigue != 0.0 {
            let row = layout.add_item();
            row.label(&format!("{}: ", tr("Tooltip_item_Fatigue")), LABEL);
            if fatigue < 0.0 {
                row.progress(-fatigue, [cx.good[0], cx.good[1], cx.good[2], 1.0]);
            } else {
                row.progress(fatigue, [cx.bad[0], cx.bad[1], cx.bad[2], 1.0]);
            }
        }
    }
    kinds::do_tooltip(cx, item, tip, layout)?;
    tail_rows(cx, item, tip, layout)
}

/// The rows after the subclass's `DoTooltip`.
fn tail_rows(cx: &mut Cx<'_>, item: &ItemRef, tip: &Tip<'_>, layout: &mut Layout) -> LuaResult<()> {
    let e = item_env();
    let owner = e.owner(item);
    {
        let it = item.borrow();
        if it.remote_control_id != -1 {
            layout
                .add_item()
                .label(&tr("Tooltip_TrapControllerID"), LABEL)
                .value(&int_str(it.remote_control_id), LABEL);
        }
        let head_repaired = components::has_times_head_repaired(&it);
        if it.have_been_repaired > 0 {
            let row = layout.add_item();
            row.label(&trc("Tooltip_weapon_Repaired"), LABEL);
            if head_repaired {
                row.label(&trc("Tooltip_handle_Repaired"), LABEL);
            }
            row.value(&format!("{}x", it.have_been_repaired), WHITE);
        }
        if head_repaired {
            let n = components::times_head_repaired(&it);
            if n > 0 {
                layout
                    .add_item()
                    .label(&trc("Tooltip_head_Repaired"), LABEL)
                    .value(&format!("{n}x"), WHITE);
            }
        }
        if it.is_equipped_no_sprint() {
            layout
                .add_item()
                .label(&tr("Tooltip_CantSprintEquipped"), [1.0, 0.1, 0.1, 1.0]);
        }
        if sv(&it, |s| s.is_wet) {
            let f = it.wet_cooldown / 10000.0;
            let c = cx.good_to_bad(f);
            layout
                .add_item()
                .label(&format!("{}: ", tr("Tooltip_Wetness")), LABEL)
                .progress(f, [c[0], c[1], c[2], 1.0]);
        }
        if it.max_capacity > 0 {
            let mut capacity = it.max_capacity as f32;
            if it.template.condition_affects_capacity {
                capacity =
                    vehicle_number_by_condition(it.max_capacity as f32, it.condition as f32, 5.0);
            }
            let v = if it.item_capacity > -1.0 {
                format!("{} / {}", fstr(it.item_capacity), fstr(capacity))
            } else {
                format!("0 / {}", fstr(capacity))
            };
            layout
                .add_item()
                .label(&trc("Tooltip_container_Capacity"), LABEL)
                .value(&v, LABEL);
        }
    }
    if !is_weapon(&item.borrow())
        && components::has_sharpness_ref(item)
            .map_err(|error| LuaError::runtime(format!("{error:?}")))?
    {
        let f = components::sharpness_mut(item)
            .map_err(|error| LuaError::runtime(format!("{error:?}")))?;
        let c = cx.bad_to_good(f);
        layout
            .add_item()
            .label(&trc("Tooltip_weapon_Sharpness"), LABEL)
            .progress(f, [c[0], c[1], c[2], 1.0]);
    }
    {
        let it = item.borrow();
        let mech = sv(&it, |s| s.vehicle_type);
        let show = !(is_weapon(&it)
            || is_clothing(&it)
            || it.condition_max <= 0
            || mech <= 0 && !it.has_tag("base:showcondition") && it.condition_max <= it.condition);
        if show {
            let key = if components::has_head_condition(&it) {
                "Tooltip_weapon_HandleCondition"
            } else {
                "Tooltip_weapon_Condition"
            };
            let f = it.condition as f32 / it.condition_max as f32;
            let c = cx.bad_to_good(f);
            layout
                .add_item()
                .label(&trc(key), LABEL)
                .progress(f, [c[0], c[1], c[2], 1.0]);
        }
    }
    if has_player_value(cx) && chr::has_read_map(cx, item)? {
        layout
            .add_item()
            .label(&tr("Tooltip_literature_HasBeenRead"), LABEL);
    }
    let media = item.borrow().recorded_media_index;
    if media >= 0 {
        chr::recorded_media_rows(cx, item, layout)?;
    }
    if item.borrow().has_tag("base:compass") && owner.is_in_player_inventory(&item.borrow()) {
        if let Some(key) = chr::compass_key(cx, item)? {
            layout.add_item().label(&tr(&key), LABEL);
        }
    }
    {
        let it = item.borrow();
        if sv(&it, |s| s.fishing_lure) {
            layout.add_item().label(&tr("Tooltip_IsFishingLure"), LABEL);
        }
    }
    components::do_tooltip(cx, item, tip, layout)?;
    let it = item.borrow();
    let vision = sv(&it, |s| s.vision_modifier);
    let vision = if script(&it).is_some() { vision } else { 1.0 };
    if vision != 1.0 {
        let row = layout.add_item();
        row.label(&format!("{}: ", tr("Tooltip_item_VisionImpariment")), LABEL);
        if vision < 1.0 {
            row.progress(1.0 - vision, [cx.bad[0], cx.bad[1], cx.bad[2], 1.0]);
        } else {
            row.progress(vision - 1.0, [cx.good[0], cx.good[1], cx.good[2], 1.0]);
        }
    }
    let hearing = if script(&it).is_some() {
        sv(&it, |s| s.hearing_modifier)
    } else {
        1.0
    };
    if hearing != 1.0 {
        let row = layout.add_item();
        row.label(
            &format!("{}: ", tr("Tooltip_item_HearingImpariment")),
            LABEL,
        );
        if hearing < 1.0 {
            row.progress(1.0 - hearing, [cx.bad[0], cx.bad[1], cx.bad[2], 1.0]);
        } else {
            row.progress(hearing - 1.0, [cx.good[0], cx.good[1], cx.good[2], 1.0]);
        }
    }
    // Item.getDiscomfortModifier(): scaled by the sandbox factor.
    let factor = crate::bind_sandbox::current_double("DiscomfortFactor")
        .unwrap_or_else(|| sandbox_fallback().discomfort_factor);
    let discomfort = if factor <= 0.0 {
        0.0
    } else {
        (sv(&it, |s| s.discomfort_modifier) as f64 * factor) as f32
    };
    if discomfort != 0.0 {
        let row = layout.add_item();
        row.label(&format!("{}: ", tr("Tooltip_item_Discomfort")), LABEL);
        if discomfort > 0.0 {
            row.progress(discomfort, [cx.bad[0], cx.bad[1], cx.bad[2], 1.0]);
        } else {
            row.progress(-discomfort, [cx.good[0], cx.good[1], cx.good[2], 1.0]);
        }
    }
    if cx.show_mod_info {
        let mod_id = script(&it).and_then(|s| s.borrow().entity.mod_id().map(str::to_string));
        if mod_id.as_deref().is_some_and(|m| m != "pz-vanilla") {
            // Colors.CornFlowerBlue. TODO(WorldDictionary): the mod's name
            // and the mods this item overrides (`getModOverrides`).
            let c = cornflower_blue(cx.t);
            layout.add_item().label(
                &format!("Mod: {}", mod_id.unwrap_or_default()),
                [c[0], c[1], c[2], 1.0],
            );
        }
    }
    let text = mod_str(&it, "Tooltip").or_else(|| sv(&it, |s| s.tooltip.clone()));
    if let Some(text) = text {
        layout.add_item().label(&tr(&text), LABEL);
    }
    Ok(())
}

fn has_player_value(cx: &Cx<'_>) -> bool {
    !cx.player.is_nil()
}

/// `Colors.CornFlowerBlue`, from the bound `Colors` class (read from the
/// installed jar); white when `Colors` is not bound.
pub(crate) fn cornflower_blue(t: &Thread) -> Col {
    let v = match t.globals().get_str("Colors") {
        Value::Table(tb) => tb.get_str("CornFlowerBlue"),
        _ => Value::Nil,
    };
    crate::bind_misc_classes::color_of(&v).unwrap_or([1.0; 4])
}

/// `VehiclePart.getNumberByCondition(number, cond, min)`.
pub fn vehicle_number_by_condition(number: f32, cond: f32, min: f32) -> f32 {
    let cond = cond + 20.0 * (100.0 - cond) / 100.0;
    let d = cond / 100.0;
    ((min.max(number * d) * 100.0) as f64).round() as f32 / 100.0
}

/// A `HashSet<String>`'s iteration order in Java: buckets by
/// `String.hashCode()` spread (`h ^ h >>> 16`) in a table sized for the
/// elements, then insertion order within a bucket (sorted by name here,
/// as the script parser's order is not kept).
pub(crate) fn java_hash_set_order(set: &std::collections::HashSet<String>) -> Vec<String> {
    let mut v: Vec<String> = set.iter().cloned().collect();
    v.sort();
    let mut cap = 16usize;
    while v.len() as f32 > cap as f32 * 0.75 {
        cap *= 2;
    }
    let bucket = |s: &str| {
        let h = s
            .encode_utf16()
            .fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(c as i32));
        let h = h ^ ((h as u32) >> 16) as i32;
        (h as u32 as usize) & (cap - 1)
    };
    v.sort_by_key(|s| bucket(s));
    v
}

/// Binds `InventoryItem:DoTooltip(tooltip)`, `DoTooltip(tooltip,
/// layout)` (the subclass rows only) and `DoTooltipEmbedded(tooltip,
/// layout, offsetY)` (on every item class, as the subclasses inherit
/// them).
pub fn register(thread: &mut Thread) {
    use crate::hud::tooltip::LayoutObject;
    let b = crate::expose::expose_class(
        thread,
        crate::bind_items::INVENTORY_ITEM_CLASS,
        Some(crate::bind_items::GAME_ENTITY_CLASS),
    );
    fn tooltip_of(c: &pz_kahlua::NativeCall<'_>, i: usize) -> LuaResult<Rc<UIElement>> {
        crate::bind_ui::element_of(&c.arg(i)).ok_or_else(npe)
    }
    fn layout_of(c: &pz_kahlua::NativeCall<'_>, i: usize) -> Option<Rc<RefCell<Layout>>> {
        match c.arg(i) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<LayoutObject>()
                .map(|l| l.0.clone()),
            _ => None,
        }
    }
    b.method("DoTooltip", |c| {
        let item = crate::bind_items::item_arg(c, 0)?;
        let el = tooltip_of(c, 1)?;
        if c.nargs() >= 3 {
            // DoTooltip(tooltipUI, layout): the subclass's rows.
            let Some(layout) = layout_of(c, 2) else {
                return Ok(0);
            };
            let tip = Tip::of(&el)?;
            let mut cx = Cx::new(c.thread, tip.tt.character());
            let mut l = layout.borrow_mut();
            kinds::do_tooltip(&mut cx, &item, &tip, &mut l)?;
            return Ok(0);
        }
        do_tooltip(c.thread, &item, &el)?;
        Ok(0)
    });
    b.method("DoTooltipEmbedded", |c| {
        let item = crate::bind_items::item_arg(c, 0)?;
        let el = tooltip_of(c, 1)?;
        let offset = crate::expose::int_arg(c, 3);
        match layout_of(c, 2) {
            Some(layout) => {
                let mut l = layout.borrow_mut();
                do_tooltip_embedded(c.thread, &item, &el, Some(&mut l), offset)?;
            }
            None => do_tooltip_embedded(c.thread, &item, &el, None, offset)?,
        }
        Ok(0)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bind_items::item_value;
    use pz_inventory::ItemTemplate;

    fn thread() -> Thread {
        let mut t = Thread::new(pz_kahlua::TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        crate::bind_java_util::register(&mut t);
        crate::bind_globals::register(&mut t);
        crate::bind_ui::register(&mut t);
        crate::hud::register(&mut t);
        crate::bind_items::register(&mut t);
        t
    }

    fn rows(l: &Layout) -> Vec<(String, Option<String>, f32)> {
        l.items
            .iter()
            .map(|i| {
                (
                    i.label.clone().unwrap_or_default(),
                    i.value.clone(),
                    i.progress_fraction,
                )
            })
            .collect()
    }

    #[test]
    fn weapon_and_food_rows() {
        let mut t = thread();
        let el = ObjectTooltip::create("Small");
        let mut tpl = ItemTemplate::new(ItemClass::Weapon, "Base", "Knife");
        tpl.actual_weight = 0.75;
        tpl.min_damage = 0.5;
        tpl.max_damage = 1.5;
        let item: ItemRef = Rc::new(RefCell::new(InventoryItem::create(Rc::new(tpl), 1)));
        item.borrow_mut().condition = 5;
        let mut l = Layout::default();
        do_tooltip_embedded(&mut t, &item, &el, Some(&mut l), 3).unwrap();
        // No translations loaded: keys come back as they are.
        let r = rows(&l);
        assert_eq!(r[0].0, "Tooltip_item_Weight:");
        assert_eq!(
            r[0].1.as_deref(),
            Some("0.75    (0.75 Tooltip_item_Equipped)")
        );
        assert_eq!(r[1], ("Tooltip_weapon_Condition:".into(), None, 0.5));
        assert_eq!(r[2], ("Tooltip_weapon_Damage:".into(), None, 0.4));
        assert_eq!(l.offset_y, ObjectTooltip::line_spacing() + 5 + 5 + 3);
        // The name: half condition is above a third, so no prefix.
        assert_eq!(item_name(&mut item.borrow_mut()), "Knife");
        item.borrow_mut().condition = 0;
        assert_eq!(item_name(&mut item.borrow_mut()), "IGUI_ClothingNaming");

        // Food without a character: the original's NullPointerException
        // after the hunger row.
        let mut tpl = ItemTemplate::new(ItemClass::Food, "Base", "Apple");
        tpl.hunger_change = -16.0;
        let food: ItemRef = Rc::new(RefCell::new(InventoryItem::create(Rc::new(tpl), 2)));
        let mut l = Layout::default();
        let e = do_tooltip_embedded(&mut t, &food, &el, Some(&mut l), 0).unwrap_err();
        assert!(matches!(
            e,
            LuaError::Runtime {
                class: "java.lang.NullPointerException",
                ..
            }
        ));
        let r = rows(&l);
        assert_eq!(r[1].0, "Tooltip_food_Hunger:");
        assert_eq!(r[1].1.as_deref(), Some("-16"));
    }

    #[test]
    fn lua_do_tooltip_lays_out_and_sizes_the_tooltip() {
        let mut t = thread();
        let tpl = ItemTemplate::new(ItemClass::AlarmClock, "Base", "AlarmClock2");
        let item: ItemRef = Rc::new(RefCell::new(InventoryItem::create(Rc::new(tpl), 3)));
        if let ItemKind::AlarmClock(a) = &mut item.borrow_mut().kind {
            a.alarm_set = true;
            a.alarm_hour = 6;
            a.alarm_minutes = 0;
        }
        crate::bind_climate::shared_game_time()
            .borrow_mut()
            .time_of_day = 9.25;
        t.globals().set_str("item", item_value(&item));
        let f = pz_kahlua::stdlib::load_string(
            "local tip = ObjectTooltip.new()\n\
             tip:setMeasureOnly(true)\n\
             item:DoTooltip(tip)\n\
             return tip:getHeight(), tip:getWidth()",
            Some("test"),
            t.globals().clone(),
        )
        .unwrap();
        let r = t
            .pcall(&f, &[])
            .map_err(|(m, _)| pz_kahlua::exec::java_value_of(&m))
            .unwrap();
        let h = r[0].as_num().unwrap();
        // The tooltip's own pad top (beginLayout then sets the static
        // pads, 0 at the bottom), the name line + 5, three rows (weight,
        // time, alarm).
        let ls = ObjectTooltip::line_spacing() as f64;
        assert_eq!(h, 5.0 + ls + 5.0 + 3.0 * ls);
        assert!(r[1].as_num().unwrap() >= 150.0);
    }
}
