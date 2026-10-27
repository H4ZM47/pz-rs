//! Gameplay methods that need the item's holder or the game state: the
//! visual from the `ClothingItem` asset, weights, condition, clothing
//! wetness and defense, gas masks and SCBAs, and `Use`.
//!
//! Each function ports the original method named in its doc. Where the
//! original reads its container's parent, the caller passes an
//! [`ItemOwner`]; game state comes through an [`ItemContext`].

use crate::assets::{ClothingItem, ItemAssets};
use crate::container::ItemContainer;
use crate::context::{wet_dry_state, ItemContext, ItemOwner, WetDryState};
use crate::item::{ClothingData, InventoryItem, ItemKind};
use crate::moddata::{ModDataError, ModDataResult};
use crate::saved::SavedItem;
use crate::template::ItemClass;
use crate::visual::{ItemVisual, VisualRng};
use pz_kahlua::Value;
use pz_scripts::objects::items::ItemReplacement;

/// `ItemTag` registry ids these methods test.
pub mod tags {
    pub const GAS_MASK: &str = "base:gasmask";
    pub const RESPIRATOR: &str = "base:respirator";
    pub const IMPROVISED_GAS_MASK: &str = "base:improvisedgasmask";
    pub const SCBA: &str = "base:scba";
    pub const BREAK_WHEN_WET: &str = "base:breakwhenwet";
    pub const LIGHT_WHEN_ATTACHED: &str = "base:lightwhenattached";
    pub const KEY_RING: &str = "base:keyring";
    pub const SPIKED: &str = "base:spiked";
    pub const SPIKED_BEHIND: &str = "base:spikedbehind";
}

/// An exception the original throws where these methods return a value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ItemError {
    /// `NullPointerException` (what was null).
    NullPointer(&'static str),
    ModData(ModDataError),
}

/// `getAlternateModelName()` values.
pub mod alternate_model {
    pub const RIGHT_HAND: &str = "RightHand";
    pub const LEFT_HAND: &str = "LeftHand";
    pub const WITHOUT_DRAINABLE: &str = "WithoutDrainable";
}

/// What `setCondition` did besides changing the item.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConditionOutcome {
    /// Sounds the holding player plays (`doBreakSound`/`doDamagedSound`).
    pub sounds: Vec<String>,
    /// `onBreak()` calls: the script's `OnBreak` Lua function, called with
    /// the item and its character (once per `setBroken(true)`).
    pub on_break_calls: Vec<String>,
    /// `Clothing.setCondition`: the worn item is taken off and dropped
    /// (`Unwear(true)`).
    pub unwear_and_drop: bool,
}

impl ConditionOutcome {
    fn merge(&mut self, o: ConditionOutcome) {
        self.sounds.extend(o.sounds);
        self.on_break_calls.extend(o.on_break_calls);
        self.unwear_and_drop |= o.unwear_and_drop;
    }
}

/// An item `Use` adds to the item's container (`container.AddItem(type)`).
#[derive(Clone, Debug, PartialEq)]
pub struct AddedItem {
    pub full_type: String,
    /// The created item (`None` when it cannot be created: `AddItem`
    /// returns null and nothing else happens).
    pub item: Option<InventoryItem>,
    /// The new item takes the used item's hand slots
    /// (`DrainableComboItem` `ReplaceOnDeplete`).
    pub takes_hands: bool,
}

/// What `Use` asks of the item's container and character.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UseOutcome {
    /// Nothing happened (`isDisappearOnUse()` false and not crafting).
    pub ignored: bool,
    /// `ReplaceOnUse` / `ReplaceOnDeplete` item, already set up from the
    /// used one.
    pub added: Option<AddedItem>,
    /// `chr.removeFromHands(item)`.
    pub remove_from_hands: bool,
    /// The item leaves its container.
    pub remove: bool,
    /// `syncItemFields()` (network).
    pub sync: bool,
    /// `Clothing.Use`: `Unwear()` first.
    pub unwear: bool,
    /// A drainable used as a world item goes through `ItemUser.UseItem`
    /// (not ported: nothing else changed).
    pub world_item_use: bool,
}

fn is_clothing(i: &InventoryItem) -> bool {
    matches!(
        i.kind,
        ItemKind::Clothing(_) | ItemKind::AlarmClockClothing(..)
    )
}

fn clothing_mut(i: &mut InventoryItem) -> Option<&mut ClothingData> {
    match &mut i.kind {
        ItemKind::Clothing(c) | ItemKind::AlarmClockClothing(c, _) => Some(c),
        _ => None,
    }
}

fn clothing(i: &InventoryItem) -> Option<&ClothingData> {
    match &i.kind {
        ItemKind::Clothing(c) | ItemKind::AlarmClockClothing(c, _) => Some(c),
        _ => None,
    }
}

fn blank(s: Option<&str>) -> bool {
    s.is_none_or(|s| s.trim().is_empty())
}

/// `StringUtils.moduleDotType(module, type)`.
pub fn module_dot_type(module: &str, t: &str) -> String {
    if t.contains('.') {
        t.to_string()
    } else {
        format!("{module}.{t}")
    }
}

impl InventoryItem {
    // ----- Visual -----

    /// `getAlternateModelName()` (`Clothing` adds `WithoutDrainable` for a
    /// gas mask without a filter or an SCBA without a tank). `None` is the
    /// base field's null.
    pub fn try_get_alternate_model_name(
        &self,
        assets: Option<&ItemAssets>,
        owner: &dyn ItemOwner,
    ) -> ModDataResult<Option<&'static str>> {
        let t = &self.template;
        if is_clothing(self) {
            let without = || {
                !blank(t.without_drainable.as_deref())
                    && t.without_drainable_clothing_item
                        .as_deref()
                        .is_some_and(|n| assets.is_some_and(|a| a.clothing_items.get(n).is_some()))
            };
            if self.is_gas_mask() && !self.try_has_filter()? && without() {
                return Ok(Some(alternate_model::WITHOUT_DRAINABLE));
            }
            if self.has_tag(tags::SCBA) && !self.try_has_tank()? && without() {
                return Ok(Some(alternate_model::WITHOUT_DRAINABLE));
            }
        }
        if owner.has_container(self) {
            if owner.is_primary_hand_item(self) && t.replace_primary_hand.is_some() {
                return Ok(Some(alternate_model::RIGHT_HAND));
            }
            if owner.is_secondary_hand_item(self) && t.replace_second_hand.is_some() {
                return Ok(Some(alternate_model::LEFT_HAND));
            }
        }
        Ok(None)
    }

    pub fn get_alternate_model_name(
        &self,
        assets: Option<&ItemAssets>,
        owner: &dyn ItemOwner,
    ) -> Option<&'static str> {
        self.try_get_alternate_model_name(assets, owner)
            .expect("unborrowed clothing model ModData")
    }

    /// `getClothingItem()`: the asset of the script's `clothingItem`, or of
    /// the hand replacement / `withoutDrainable` script the alternate
    /// model name selects. `None` without a ready asset.
    /// `getItemReplacementPrimaryHand()`: the script item's
    /// `replacePrimaryHand` (clothing item name, mask variable name and
    /// value, attachment).
    pub fn get_item_replacement_primary_hand(&self) -> Option<&ItemReplacement> {
        self.template.replace_primary_hand.as_ref()
    }

    /// `getItemReplacementSecondHand()`.
    pub fn get_item_replacement_second_hand(&self) -> Option<&ItemReplacement> {
        self.template.replace_second_hand.as_ref()
    }

    pub fn try_get_clothing_item<'a>(
        &self,
        assets: Option<&'a ItemAssets>,
        owner: &dyn ItemOwner,
    ) -> ModDataResult<Option<&'a ClothingItem>> {
        let Some(a) = assets else {
            return Ok(None);
        };
        let t = &self.template;
        let get = |n: &Option<String>| n.as_deref().and_then(|n| a.clothing_items.get(n));
        match self.try_get_alternate_model_name(assets, owner)? {
            Some(alternate_model::RIGHT_HAND) => {
                return Ok(t
                    .replace_primary_hand
                    .as_ref()
                    .and_then(|r| get(&r.clothing_item_name)));
            }
            Some(alternate_model::LEFT_HAND) => {
                return Ok(t
                    .replace_second_hand
                    .as_ref()
                    .and_then(|r| get(&r.clothing_item_name)));
            }
            Some(alternate_model::WITHOUT_DRAINABLE) => {
                if let Some(c) = get(&t.without_drainable_clothing_item) {
                    return Ok(Some(c));
                }
            }
            _ => {}
        }
        Ok(get(&t.clothing_item))
    }

    pub fn get_clothing_item<'a>(
        &self,
        assets: Option<&'a ItemAssets>,
        owner: &dyn ItemOwner,
    ) -> Option<&'a ClothingItem> {
        self.try_get_clothing_item(assets, owner)
            .expect("unborrowed clothing model ModData")
    }

    /// `getVisual()`: drops the visual when there is no ready clothing
    /// item, else creates it (`pickUninitializedValues`) and sets its
    /// clothing item and alternate model names.
    pub fn try_get_visual(
        &mut self,
        assets: Option<&ItemAssets>,
        owner: &dyn ItemOwner,
        rng: &dyn VisualRng,
    ) -> ModDataResult<Option<&mut ItemVisual>> {
        let Some(c) = self.try_get_clothing_item(assets, owner)? else {
            self.visual = None;
            return Ok(None);
        };
        let alt = self.try_get_alternate_model_name(assets, owner)?;
        let full = self.get_full_type();
        let v = self.visual.get_or_insert_with(|| {
            let mut v = ItemVisual::new(&full);
            v.pick_uninitialized_values(c, false, rng);
            v
        });
        v.clothing_item_name = c.name.clone();
        v.alternate_model_name = alt.unwrap_or("").to_string();
        Ok(Some(v))
    }

    pub fn get_visual(
        &mut self,
        assets: Option<&ItemAssets>,
        owner: &dyn ItemOwner,
        rng: &dyn VisualRng,
    ) -> Option<&mut ItemVisual> {
        self.try_get_visual(assets, owner, rng)
            .expect("unborrowed visual ModData")
    }

    /// `synchWithVisual()` for the visual part: `Clothing` and
    /// `InventoryContainer` make their visual, clothing with a blood
    /// clothing type recomputes its blood and dirt levels, and the item
    /// colour follows the tint. (`modelIndex` and the icon texture, drawn
    /// from `Rand` and the texture manager, are not modelled.)
    pub fn try_synch_with_visual(
        &mut self,
        assets: Option<&ItemAssets>,
        owner: &dyn ItemOwner,
        rng: &dyn VisualRng,
    ) -> ModDataResult<()> {
        if !is_clothing(self) && !matches!(self.kind, ItemKind::Container(_)) {
            return Ok(());
        }
        if self.try_get_visual(assets, owner, rng)?.is_none() {
            return Ok(());
        }
        let assets = assets.expect("a visual needs assets");
        if is_clothing(self) && self.template.blood_clothing_type.is_some() {
            self.calc_total_blood_and_dirt(assets);
        }
        let c = self.try_get_clothing_item(Some(assets), owner)?.unwrap();
        let tint = self.visual.as_ref().and_then(|v| v.tint);
        if c.allow_random_tint && !self.custom_color {
            let t = tint.unwrap_or([1.0, 1.0, 1.0]);
            self.col = [t[0], t[1], t[2], 1.0];
        } else {
            self.col = [self.color_red, self.color_green, self.color_blue, 1.0];
        }
        Ok(())
    }

    pub fn synch_with_visual(
        &mut self,
        assets: Option<&ItemAssets>,
        owner: &dyn ItemOwner,
        rng: &dyn VisualRng,
    ) {
        self.try_synch_with_visual(assets, owner, rng)
            .expect("unborrowed visual ModData")
    }

    /// `BloodClothingType.calcTotalBloodLevel` and `calcTotalDirtLevel`.
    pub fn calc_total_blood_and_dirt(&mut self, assets: &ItemAssets) {
        let parts = self.get_covered_parts(assets);
        let (blood, dirt) = match (&self.visual, parts) {
            (Some(v), Some(p)) if !p.is_empty() => {
                let b: f32 = p.iter().fold(0.0, |a, &i| a + v.get_blood(i) * 100.0);
                let d: f32 = p.iter().fold(0.0, |a, &i| a + v.get_dirt(i) * 100.0);
                (b / p.len() as f32, d / p.len() as f32)
            }
            _ => (0.0, 0.0),
        };
        if let Some(c) = clothing_mut(self) {
            c.blood_level = blood;
            c.dirtyness = dirt;
        }
    }

    /// `getBloodClothingType()` (the script's names).
    pub fn get_blood_clothing_type(&self) -> Option<&[String]> {
        self.template.blood_clothing_type.as_deref()
    }

    /// `Clothing.getCoveredParts()`: `BloodBodyPartType` indices.
    pub fn get_covered_parts(&self, assets: &ItemAssets) -> Option<Vec<usize>> {
        let names = self.template.blood_clothing_type.as_ref()?;
        Some(assets.blood_clothing_types.covered_parts_of(names))
    }

    /// `getBodyLocation()` as its registry id.
    pub fn get_body_location(&self) -> Option<&str> {
        self.template.body_location.as_deref()
    }

    /// `isBodyLocation(location)` by registry id.
    pub fn is_body_location(&self, id: &str) -> bool {
        self.template
            .body_location
            .as_deref()
            .is_some_and(|l| l.eq_ignore_ascii_case(id))
    }

    /// `isEquippedNoSprint()`.
    pub fn is_equipped_no_sprint(&self) -> bool {
        self.template.equipped_no_sprint
    }

    /// `isProtectFromRainWhileEquipped()`.
    pub fn is_protect_from_rain_while_equipped(&self) -> bool {
        self.template.protect_from_rain_when_equipped
    }

    /// `getClothingItemName()`.
    pub fn get_clothing_item_name(&self) -> Option<&str> {
        self.template.clothing_item.as_deref()
    }

    /// `isTwoHandWeapon()`.
    pub fn is_two_hand_weapon(&self) -> bool {
        self.template.two_hand_weapon
    }

    /// `getSwingAnim()`.
    pub fn get_swing_anim(&self) -> Option<&str> {
        self.template.swing_anim.as_deref()
    }

    /// `isDisappearOnUse()`.
    pub fn is_disappear_on_use(&self) -> bool {
        self.template.disappear_on_use
    }

    /// `isKeepOnDeplete()`.
    pub fn is_keep_on_deplete(&self) -> bool {
        !self.template.disappear_on_use
    }

    /// `getReplaceOnUse()`.
    pub fn get_replace_on_use(&self) -> Option<&str> {
        self.template.replace_on_use.as_deref()
    }

    /// `getReplaceOnUseFullType()`.
    pub fn get_replace_on_use_full_type(&self) -> Option<String> {
        self.template
            .replace_on_use
            .as_deref()
            .map(|r| module_dot_type(self.get_module(), r))
    }

    /// `DrainableComboItem.getReplaceOnDeplete()`.
    pub fn get_replace_on_deplete(&self) -> Option<&str> {
        match self.template.class {
            ItemClass::Drainable => self.template.replace_on_deplete.as_deref(),
            _ => None,
        }
    }

    /// `DrainableComboItem.getReplaceOnDepleteFullType()`.
    pub fn get_replace_on_deplete_full_type(&self) -> Option<String> {
        self.get_replace_on_deplete()
            .map(|r| module_dot_type(self.get_module(), r))
    }

    /// `isKeyRing()`.
    pub fn is_key_ring(&self) -> bool {
        self.template.key_ring_type
            || self.has_tag(tags::KEY_RING) && matches!(self.kind, ItemKind::Container(_))
    }

    /// `isBroken()`.
    pub fn is_broken(&self) -> bool {
        self.condition <= 0
    }

    // ----- Weight -----

    /// `getActualWeightUnmodded()`: 0 when the display name equals the
    /// full type, else the non-negative `actualWeight`.
    /// (`Moveable` and `Radio` that read their world sprite compare their
    /// translated moveable name, which is never the full type.)
    pub fn get_actual_weight_unmodded(&self) -> f32 {
        // Moveable.getDisplayName: a moveable that read its world sprite
        // shows its (translated) moveable name instead.
        let sprite_named = matches!(&self.kind,
            ItemKind::Moveable(m) | ItemKind::Radio(m, _) if m.has_read_world_sprite);
        if !sprite_named && self.name == self.get_full_type() {
            return 0.0;
        }
        self.actual_weight.max(0.0)
    }

    /// `getActualWeight()` with the subclass rules (`HandWeapon`: script
    /// weight plus the parts' weight modifiers; `Food`: scaled by the
    /// hunger or thirst left, and its empty item and extra items).
    pub fn get_actual_weight_with(&self, ctx: &dyn ItemContext) -> f32 {
        self.actual_weight_impl(Some(ctx))
    }

    pub(crate) fn actual_weight_impl(&self, ctx: Option<&dyn ItemContext>) -> f32 {
        let t = &self.template;
        match &self.kind {
            ItemKind::Weapon(w) => {
                let mut weight = t.actual_weight;
                for p in &w.attachments {
                    weight += p.template.weight_modifier;
                }
                weight
            }
            ItemKind::Food(f) => {
                let empty_weight = |ctx: Option<&dyn ItemContext>| {
                    let full = self.get_replace_on_use_full_type()?;
                    ctx?.script(&full).map(|s| s.actual_weight)
                };
                if self.extra_items.as_ref().is_some_and(|x| !x.is_empty()) {
                    let used = if f.base_hunger == 0.0 {
                        0.0
                    } else {
                        f.hung_change / f.base_hunger
                    };
                    let empty = empty_weight(ctx).unwrap_or(0.0);
                    let actual =
                        self.get_actual_weight_unmodded() + self.get_extra_items_weight(ctx);
                    return (actual - empty) * used + empty;
                }
                let used = || {
                    if t.hunger_change < 0.0 {
                        f.hung_change * 100.0 / t.hunger_change
                    } else if t.thirst_change < 0.0 {
                        f.thirst_change * 100.0 / t.thirst_change
                    } else {
                        1.0
                    }
                };
                if t.replace_on_use.is_some() && !self.custom_weight {
                    if let Some(e) = empty_weight(ctx) {
                        return (t.actual_weight - e) * used() + e;
                    }
                } else if !self.custom_weight {
                    return t.actual_weight * used();
                }
                self.get_actual_weight_unmodded()
            }
            _ => self.get_actual_weight_unmodded(),
        }
    }

    /// `getExtraItemsWeight()`: 0.6 of the positive weights of the items
    /// `CreateItem` makes from `extraItems`.
    pub fn get_extra_items_weight(&self, ctx: Option<&dyn ItemContext>) -> f32 {
        let (Some(x), Some(ctx)) = (self.extra_items.as_ref(), ctx) else {
            return 0.0;
        };
        let mut w = 0.0f32;
        for &id in x {
            let Some(item) = ctx
                .item_type_from_id(id)
                .and_then(|full| ctx.create_item(&full))
            else {
                continue;
            };
            let a = item.get_actual_weight_with(ctx);
            if a > 0.0 {
                w += a;
            }
        }
        w * 0.6
    }

    /// `Clothing.getWeight()` and the other `getWeight()` overrides.
    pub fn get_weight_with(&self, ctx: &dyn ItemContext) -> f32 {
        match &self.kind {
            ItemKind::Weapon(_) => self.get_actual_weight_with(ctx),
            ItemKind::Food(_) if self.template.replace_on_use.is_some() => {
                self.get_actual_weight_with(ctx)
            }
            ItemKind::Clothing(c) | ItemKind::AlarmClockClothing(c, _) => {
                let weight = self.get_actual_weight_with(ctx);
                let mut wet = self.template.weight_wet;
                if wet <= 0.0 {
                    wet = weight * 1.25;
                }
                weight + c.wetness / 100.0 * (wet - weight)
            }
            _ => self.weight.max(0.0),
        }
    }

    /// `getContentsWeight()`: the chambered round and magazine of a gun,
    /// the ammo count, the fluid, and an `InventoryContainer`'s items.
    pub fn get_contents_weight_with(&self, ctx: &dyn ItemContext, owner: &dyn ItemOwner) -> f32 {
        let t = &self.template;
        let base = || {
            if let Some(ammo) = t.ammo_item.as_deref() {
                if let Some(s) = ctx.script(ammo) {
                    return s.actual_weight * self.current_ammo_count as f32;
                }
            }
            if let Some(a) = ctx.fluid_amount(self) {
                return a;
            }
            owner.world_item_fluid_amount(self).unwrap_or(0.0)
        };
        match &self.kind {
            ItemKind::Weapon(w) => {
                let mut c = 0.0f32;
                if t.have_chamber && w.round_chambered {
                    if let Some(s) = t.ammo_item.as_deref().and_then(|a| ctx.script(a)) {
                        c += s.actual_weight;
                    }
                }
                if w.contains_clip && !blank(t.magazine_type.as_deref()) {
                    if let Some(s) = ctx.script(t.magazine_type.as_deref().unwrap()) {
                        c += s.actual_weight;
                    }
                }
                c + base()
            }
            ItemKind::Container(c) => {
                let mut w = c.container.borrow().get_contents_weight_with(ctx);
                if let Some(a) = ctx.fluid_amount(self) {
                    w += a;
                }
                w
            }
            _ => base(),
        }
    }

    /// `getUnequippedWeight()`.
    pub fn get_unequipped_weight(&self, ctx: &dyn ItemContext, owner: &dyn ItemOwner) -> f32 {
        self.get_actual_weight_with(ctx) + self.get_contents_weight_with(ctx, owner)
    }

    /// `getEquippedWeight()` (`InventoryContainer`: the contents count
    /// with the weight reduction instead of the multiplier).
    pub fn get_equipped_weight(&self, ctx: &dyn ItemContext, owner: &dyn ItemOwner) -> f32 {
        let m = ctx.equipped_or_worn_encumbrance_multiplier() as f32;
        if let ItemKind::Container(c) = &self.kind {
            let reduction = if c.weight_reduction > 0 {
                1.0 - c.weight_reduction as f32 / 100.0
            } else {
                1.0
            };
            return self.get_actual_weight_with(ctx) * m
                + self.get_contents_weight_with(ctx, owner) * reduction;
        }
        (self.get_actual_weight_with(ctx) + self.get_contents_weight_with(ctx, owner)) * m
    }

    /// `getHotbarEquippedWeight()`.
    pub fn get_hotbar_equipped_weight(&self, ctx: &dyn ItemContext, owner: &dyn ItemOwner) -> f32 {
        let w = self.get_actual_weight_with(ctx) + self.get_contents_weight_with(ctx, owner);
        if self.has_tag(tags::LIGHT_WHEN_ATTACHED) {
            w * ctx.equipped_or_worn_encumbrance_multiplier() as f32
        } else {
            w * 0.7
        }
    }

    /// `isFakeEquipped()`: a key ring directly in a player's inventory.
    pub fn is_fake_equipped(&self, owner: &dyn ItemOwner) -> bool {
        owner.is_in_player_inventory(self)
            && owner.is_in_outermost_container(self)
            && self.is_key_ring()
    }

    /// `isEquipped()`.
    pub fn is_equipped(&self, owner: &dyn ItemOwner) -> bool {
        owner.has_container(self) && owner.is_equipped(self)
    }

    // ----- Condition -----

    /// `getCurrentCondition()`.
    pub fn get_current_condition(&self) -> f32 {
        self.condition as f32 / self.condition_max as f32 * 100.0
    }

    /// `setCondition(condition, doSound)`: clamped to `[0, conditionMax]`;
    /// the break or damaged sound for an item a player holds; `setBroken`
    /// (and its `OnBreak` call off clients). The debug and tutorial
    /// unlimited-condition cheat is not modelled.
    pub fn set_condition_sound(
        &mut self,
        condition: i32,
        do_sound: bool,
        owner: &dyn ItemOwner,
        ctx: &dyn ItemContext,
    ) -> ConditionOutcome {
        let mut out = ConditionOutcome::default();
        let c = condition.max(0).min(self.condition_max);
        if do_sound && owner.is_in_player_inventory(self) {
            let t = &self.template;
            let damaged = t.damaged_sound.clone().filter(|s| !s.trim().is_empty());
            if self.condition > 0 && c <= 0 {
                match t.break_sound.clone().filter(|s| !s.trim().is_empty()) {
                    Some(s) => out.sounds.push(s),
                    None => out.sounds.extend(damaged),
                }
            } else if self.condition > 0 && c < self.condition {
                out.sounds.extend(damaged);
            }
        }
        self.condition = c;
        out.merge(self.set_broken(c <= 0, ctx));
        out
    }

    /// `setBroken(broken)`: the `OnBreak` call (the broken flag is
    /// `condition <= 0` here).
    pub fn set_broken(&mut self, broken: bool, ctx: &dyn ItemContext) -> ConditionOutcome {
        let mut out = ConditionOutcome::default();
        if !ctx.is_client() && broken {
            if let Some(f) = self.template.on_break.clone() {
                out.on_break_calls.push(f);
            }
        }
        out
    }

    /// `setCondition(condition)` (`Clothing` also re-breaks at 0 and takes
    /// off a worn item that is removed when broken).
    pub fn set_condition(
        &mut self,
        condition: i32,
        owner: &dyn ItemOwner,
        ctx: &dyn ItemContext,
    ) -> ConditionOutcome {
        let mut out = self.set_condition_sound(condition, true, owner, ctx);
        if is_clothing(self) && condition <= 0 {
            out.merge(self.set_broken(true, ctx));
            if owner.is_worn(self) && self.template.remove_on_broken {
                out.unwear_and_drop = true;
            }
        }
        out
    }

    /// `setConditionNoSound(condition)`.
    pub fn set_condition_no_sound(
        &mut self,
        condition: i32,
        owner: &dyn ItemOwner,
        ctx: &dyn ItemContext,
    ) -> ConditionOutcome {
        self.set_condition_sound(condition, false, owner, ctx)
    }

    // ----- Clothing defense -----

    /// `Clothing.getDefForPart(part, bite, bullet)`; the original throws
    /// without a visual.
    pub fn get_def_for_part(
        &self,
        part: usize,
        bite: bool,
        bullet: bool,
        assets: &ItemAssets,
    ) -> Result<f32, ItemError> {
        let v = self
            .visual
            .as_ref()
            .ok_or(ItemError::NullPointer("getVisual()"))?;
        if v.get_hole(part) > 0.0 {
            return Ok(0.0);
        }
        let patch = self.get_patch_type(part);
        let mut defense = self.get_scratch_defense();
        if bite {
            defense = self.get_bite_defense();
        }
        if bullet {
            defense = self.get_bullet_defense();
        }
        let neck = assets.blood_clothing_types.part_index("Neck");
        if Some(part) == neck && self.template.neck_protection_modifier < 1.0 {
            defense *= self.template.neck_protection_modifier;
        }
        if let Some(p) = patch {
            let mut pd = p.scratch_defense as i32;
            if bite || bullet {
                pd = p.bite_defense as i32;
            }
            defense = if !p.has_hole {
                defense + pd as f32
            } else {
                pd as f32
            };
        }
        Ok(defense)
    }

    /// `Clothing.getPatchType(part)`.
    pub fn get_patch_type(&self, part: usize) -> Option<&crate::item::ClothingPatch> {
        clothing(self)?
            .patches
            .as_ref()?
            .iter()
            .find(|(i, _)| *i as i32 == part as i32)
            .map(|(_, p)| p)
    }

    /// `Clothing.getCorpseSicknessDefense()`: 0 when broken; a mask with a
    /// filter gives at least the basic mask defense, and the improvised or
    /// standard gas mask defense while the filter lasts.
    pub fn try_get_corpse_sickness_defense_with(
        &self,
        ctx: &dyn ItemContext,
    ) -> ModDataResult<f32> {
        if !is_clothing(self) {
            return Ok(self.template.corpse_sickness_defense);
        }
        if self.condition <= 0 {
            return Ok(0.0);
        }
        let mut defense = self.template.corpse_sickness_defense;
        if self.try_has_filter()? {
            let mut value = ctx.basic_mask_defense();
            if self.try_used_delta()? > 0.0 {
                value = if self.has_tag(tags::IMPROVISED_GAS_MASK) {
                    ctx.improvised_gas_mask_defense()
                } else {
                    ctx.standard_gas_mask_defense()
                };
            }
            if value > defense {
                defense = value;
            }
        }
        if self.try_has_filter()? && ctx.basic_mask_defense() > defense {
            defense = ctx.basic_mask_defense();
        }
        Ok(defense)
    }

    pub fn get_corpse_sickness_defense_with(&self, ctx: &dyn ItemContext) -> f32 {
        self.try_get_corpse_sickness_defense_with(ctx)
            .expect("unborrowed defense ModData")
    }

    // ----- Wetness -----

    /// `Clothing.setWetness(percent)`.
    pub fn set_wetness(&mut self, percent: f32) {
        if let Some(c) = clothing_mut(self) {
            c.wetness = percent.clamp(0.0, 100.0);
        }
    }

    /// `Clothing.getWaterResistance()`.
    pub fn get_water_resistance(&self) -> f32 {
        if is_clothing(self) {
            self.template.waterresist
        } else {
            0.0
        }
    }

    /// `Clothing.getWeightWet()`.
    pub fn get_weight_wet(&self) -> f32 {
        self.template.weight_wet
    }

    /// `Clothing.updateWetness(ignoreEquipped)`: no change while equipped
    /// (unless ignored); without a blood clothing type the item dries at
    /// once; else once a minute it breaks a wet `BreakWhenWet` item, dries
    /// (twice as fast when equipped) or soaks in the rain.
    pub fn update_wetness_with(
        &mut self,
        ignore_equipped: bool,
        owner: &dyn ItemOwner,
        ctx: &dyn ItemContext,
    ) -> ConditionOutcome {
        let mut out = ConditionOutcome::default();
        if !is_clothing(self) {
            return out;
        }
        let equipped = self.is_equipped(owner);
        if !ignore_equipped && equipped {
            return out;
        }
        if self.template.blood_clothing_type.is_none() {
            self.set_wetness(0.0);
            return out;
        }
        let now = ctx.world_age_hours() as f32;
        let last = clothing(self).unwrap().last_wetness_update;
        let last = check_hours(last, now);
        clothing_mut(self).unwrap().last_wetness_update = last;
        let elapsed = now - last;
        if elapsed < 0.016_666_668 {
            return out;
        }
        clothing_mut(self).unwrap().last_wetness_update = now;
        if self.has_tag(tags::BREAK_WHEN_WET) && self.get_wetness() >= 100.0 {
            out.merge(self.set_condition(0, owner, ctx));
        }
        match wet_dry_state(owner.place(self), ctx.is_raining(), equipped) {
            WetDryState::Invalid => {}
            WetDryState::Dryer => {
                if self.get_wetness() > 0.0 {
                    let mut dry = elapsed * 20.0;
                    if equipped {
                        dry *= 2.0;
                    }
                    self.set_wetness(self.get_wetness() - dry);
                }
            }
            WetDryState::Wetter => {
                if self.get_wetness() < 100.0 {
                    let mut intensity = ctx.rain_intensity();
                    if intensity < 0.1 {
                        intensity = 0.0;
                    }
                    let wet = intensity * elapsed * 100.0;
                    self.set_wetness(self.get_wetness() + wet);
                }
            }
        }
        out
    }

    /// `Clothing.flushWetness()`.
    pub fn flush_wetness(
        &mut self,
        owner: &dyn ItemOwner,
        ctx: &dyn ItemContext,
    ) -> ConditionOutcome {
        let Some(c) = clothing(self) else {
            return ConditionOutcome::default();
        };
        if c.last_wetness_update < 0.0 {
            return ConditionOutcome::default();
        }
        let out = self.update_wetness_with(true, owner, ctx);
        clothing_mut(self).unwrap().last_wetness_update = -1.0;
        out
    }

    // ----- Gas masks and SCBAs -----

    /// Has one of the `GasMask`, `Respirator`, `ImprovisedGasMask` tags.
    pub fn is_gas_mask(&self) -> bool {
        self.has_tag(tags::GAS_MASK)
            || self.has_tag(tags::RESPIRATOR)
            || self.has_tag(tags::IMPROVISED_GAS_MASK)
    }
    pub fn try_get_filter_type(&self) -> ModDataResult<Option<String>> {
        self.mod_data_string("filterType")
    }
    pub fn try_get_tank_type(&self) -> ModDataResult<Option<String>> {
        self.mod_data_string("tankType")
    }
    pub fn try_has_filter(&self) -> ModDataResult<bool> {
        Ok(is_clothing(self) && self.is_gas_mask() && self.try_get_filter_type()?.is_some())
    }
    pub fn try_has_tank(&self) -> ModDataResult<bool> {
        Ok(is_clothing(self) && self.has_tag(tags::SCBA) && self.try_get_tank_type()?.is_some())
    }
    pub fn try_set_filter_type(&mut self, filter: &str) -> ModDataResult<()> {
        self.set_mod_data_value("filterType", Some(Value::str(filter)))
    }
    pub fn try_set_tank_type(&mut self, tank: &str) -> ModDataResult<()> {
        self.set_mod_data_value("tankType", Some(Value::str(tank)))
    }
    pub fn try_used_delta(&self) -> ModDataResult<f32> {
        match self.mod_data_value("usedDelta")? {
            None => Ok(0.0),
            Some(Value::Num(value)) => Ok(value as f32),
            Some(_) => Err(ModDataError::cast("java.lang.Double")),
        }
    }
    pub fn try_get_used_delta(&mut self) -> ModDataResult<f32> {
        if self.mod_data_value("usedDelta")?.is_none() {
            self.try_set_used_delta(0.0)?;
        }
        self.try_used_delta()
    }
    pub fn try_set_used_delta(&mut self, delta: f32) -> ModDataResult<()> {
        self.set_mod_data_value(
            "usedDelta",
            Some(Value::Num((if delta < 0.0 { 0.0 } else { delta }) as f64)),
        )
    }
    // Legacy synchronous callers require unborrowed actual tables. Production
    // Lua and tooltip adapters use the fallible methods above.
    pub fn get_filter_type(&self) -> Option<String> {
        self.try_get_filter_type()
            .expect("unborrowed ModData string")
    }
    pub fn get_tank_type(&self) -> Option<String> {
        self.try_get_tank_type().expect("unborrowed ModData string")
    }
    pub fn has_filter(&self) -> bool {
        self.try_has_filter().expect("unborrowed filter ModData")
    }
    pub fn has_tank(&self) -> bool {
        self.try_has_tank().expect("unborrowed tank ModData")
    }
    pub fn set_filter_type(&mut self, filter: &str) {
        self.try_set_filter_type(filter)
            .expect("unborrowed filter ModData")
    }
    pub fn set_tank_type(&mut self, tank: &str) {
        self.try_set_tank_type(tank)
            .expect("unborrowed tank ModData")
    }
    pub fn used_delta(&self) -> f32 {
        self.try_used_delta().expect("unborrowed usedDelta ModData")
    }
    pub fn get_used_delta(&mut self) -> f32 {
        self.try_get_used_delta()
            .expect("unborrowed usedDelta ModData")
    }
    pub fn set_used_delta(&mut self, delta: f32) {
        self.try_set_used_delta(delta)
            .expect("unborrowed usedDelta ModData")
    }

    /// `Clothing.drainGasMask(rate)`: the filter's `UseDelta` times the
    /// rate and the game time multiplier. Throws (`getItem` null) when the
    /// filter script does not exist. Network sync is not modelled.
    pub fn drain_gas_mask(&mut self, rate: f32, ctx: &dyn ItemContext) -> Result<(), ItemError> {
        if !self.is_gas_mask()
            || !self.try_has_filter().map_err(ItemError::ModData)?
            || self.try_get_used_delta().map_err(ItemError::ModData)? <= 0.0
        {
            return Ok(());
        }
        let filter = self
            .try_get_filter_type()
            .map_err(ItemError::ModData)?
            .unwrap()
            .to_string();
        let s = ctx
            .script(&filter)
            .ok_or(ItemError::NullPointer("ScriptManager.getItem(filterType)"))?;
        let rate2 = rate * s.use_delta * ctx.multiplier();
        let d = self.try_get_used_delta().map_err(ItemError::ModData)? - rate2;
        self.try_set_used_delta(d).map_err(ItemError::ModData)?;
        if self.try_get_used_delta().map_err(ItemError::ModData)? < 0.0 {
            self.try_set_used_delta(0.0).map_err(ItemError::ModData)?;
        }
        Ok(())
    }

    /// `Clothing.drainSCBA()`: an activated SCBA with a tank loses
    /// `0.001 * tank UseDelta * multiplier`, and turns off when empty.
    pub fn drain_scba(&mut self, ctx: &dyn ItemContext) -> Result<(), ItemError> {
        if self.try_get_used_delta().map_err(ItemError::ModData)? <= 0.0 {
            self.activated = false;
        }
        if !self.has_tag(tags::SCBA)
            || !self.try_has_tank().map_err(ItemError::ModData)?
            || !self.activated
            || self.try_get_used_delta().map_err(ItemError::ModData)? <= 0.0
        {
            return Ok(());
        }
        let tank = self
            .try_get_tank_type()
            .map_err(ItemError::ModData)?
            .unwrap()
            .to_string();
        let s = ctx
            .script(&tank)
            .ok_or(ItemError::NullPointer("ScriptManager.getItem(tankType)"))?;
        let rate2 = 0.001 * s.use_delta * ctx.multiplier();
        let d = self.try_get_used_delta().map_err(ItemError::ModData)? - rate2;
        self.try_set_used_delta(d).map_err(ItemError::ModData)?;
        if self.try_get_used_delta().map_err(ItemError::ModData)? < 0.0 {
            self.try_set_used_delta(0.0).map_err(ItemError::ModData)?;
        }
        if self.try_get_used_delta().map_err(ItemError::ModData)? <= 0.0 {
            self.activated = false;
        }
        Ok(())
    }

    // ----- Use -----

    /// `getCurrentUsesFloat()` and its overrides: `uses * useDelta` for
    /// `DrainableComboItem` and `WeaponPart`, the hunger left for `Food`
    /// with a base hunger, the device power for `Radio`, `uses /
    /// getMaxUses()` for the rest.
    pub fn get_current_uses_float(&self) -> f32 {
        match (&self.kind, self.template.class) {
            (_, ItemClass::Drainable | ItemClass::WeaponPart) => {
                self.uses as f32 * self.instance_use_delta()
            }
            (ItemKind::Food(f), _) if f.base_hunger != 0.0 => f.hung_change.abs(),
            (ItemKind::Radio(_, d), _) => d.power_delta,
            _ => self.uses as f32 / self.get_max_uses() as f32,
        }
    }

    /// The `useDelta` field the uses methods read: `DrainableComboItem`'s
    /// and `WeaponPart`'s own (`Item.InstanceItem` sets them from the
    /// script), `InventoryItem.useDelta` (0.03125, which nothing sets) for
    /// the rest.
    pub fn instance_use_delta(&self) -> f32 {
        match self.template.class {
            ItemClass::Drainable | ItemClass::WeaponPart => self.template.use_delta,
            _ => DEFAULT_USE_DELTA,
        }
    }

    /// `getUseDelta()`: the script's for `Clothing` (its override) and
    /// `DrainableComboItem`, `InventoryItem.useDelta` for the rest
    /// (`WeaponPart` does not override it, so it reads the base field,
    /// not its own).
    pub fn get_use_delta(&self) -> f32 {
        match self.template.class {
            ItemClass::Drainable => self.template.use_delta,
            _ if is_clothing(self) => self.template.use_delta,
            _ => DEFAULT_USE_DELTA,
        }
    }

    /// `setCurrentUsesFloat(f)`: `uses = round(clamp(f, 0, 1) / useDelta)`
    /// ([`Self::instance_use_delta`]); `DrainableComboItem` also updates
    /// its weight. `DrainableComboItem.setUsedDelta` and
    /// `WeaponPart.setUsedDelta` call it.
    pub fn set_current_uses_float(&mut self, f: f32, ctx: Option<&dyn ItemContext>) {
        let f = if f.is_nan() { 0.0 } else { f.clamp(0.0, 1.0) };
        self.uses = java_round(f / self.instance_use_delta());
        if self.template.class == ItemClass::Drainable {
            self.update_weight(ctx);
        }
    }

    /// `DrainableComboItem.updateWeight()`.
    pub fn update_weight(&mut self, ctx: Option<&dyn ItemContext>) {
        self.update_weight_by(|f| ctx.and_then(|c| c.script(f)).map(|s| s.actual_weight));
    }

    /// [`Self::update_weight`] with the script weight lookup
    /// (`ScriptManager.getItem(name).getActualWeight()`).
    pub fn update_weight_by(&mut self, script_weight_of: impl Fn(&str) -> Option<f32>) {
        if self.template.class != ItemClass::Drainable {
            return;
        }
        let script_weight = self.template.actual_weight;
        if self.template.replace_on_deplete.is_some() {
            if self.get_current_uses_float() >= 1.0 {
                self.custom_weight = true;
                self.actual_weight = script_weight.max(0.0);
                self.weight = self.get_actual_weight_unmodded().max(0.0);
                return;
            }
            let empty = self
                .get_replace_on_deplete_full_type()
                .and_then(|f| script_weight_of(&f));
            if let Some(e) = empty {
                self.custom_weight = true;
                let w = (script_weight - e) * self.get_current_uses_float() + e;
                self.actual_weight = w.max(0.0);
                self.weight = self.get_actual_weight_unmodded().max(0.0);
            }
        }
        let empty = self.template.weight_empty;
        if empty != 0.0 {
            self.custom_weight = true;
            let w = (script_weight - empty) * self.get_current_uses_float() + empty;
            self.actual_weight = w.max(0.0);
        }
    }

    /// `setCurrentUses(uses)` with the drainable weight update.
    pub fn set_current_uses_with(&mut self, uses: i32, ctx: Option<&dyn ItemContext>) {
        if self.template.class == ItemClass::Drainable {
            self.uses = uses;
            self.update_weight(ctx);
        } else {
            self.set_current_uses(uses);
        }
    }

    /// `Use(bCrafting, bInContainer, bNeedSync)` and the subclass
    /// overrides (`DrainableComboItem`, `Clothing`): changes the item and
    /// says what the container and character must do. Apply it with
    /// [`ItemContainer::apply_use`] (or [`ItemContainer::use_item`]).
    pub fn use_with(
        &mut self,
        crafting: bool,
        in_container: bool,
        need_sync: bool,
        owner: &dyn ItemOwner,
        ctx: &dyn ItemContext,
    ) -> UseOutcome {
        let mut out = UseOutcome::default();
        let has_container = owner.has_container(self);
        if self.template.class == ItemClass::Drainable {
            if matches!(owner.place(self), crate::context::ItemPlace::World { .. }) {
                out.world_item_use = true;
                out.sync = need_sync;
                return out;
            }
            self.uses -= 1;
            if self.uses <= 0 {
                if let Some(full) = self.get_replace_on_deplete_full_type() {
                    if has_container {
                        let item = ctx.create_item(&full).map(|mut i| {
                            i.copy_condition_states_from(self, ctx);
                            i
                        });
                        if item.is_some() {
                            out.remove = true;
                        }
                        out.added = Some(AddedItem {
                            full_type: full,
                            item,
                            takes_hands: true,
                        });
                    }
                } else {
                    if self.is_keep_on_deplete() {
                        out.sync = need_sync;
                        return out;
                    }
                    if has_container && self.is_disappear_on_use() {
                        out.remove_from_hands = true;
                        out.remove = true;
                    }
                }
            }
            self.update_weight(Some(ctx));
            out.sync |= need_sync;
            return out;
        }
        if !self.is_disappear_on_use() && !crafting {
            out.ignored = true;
            return out;
        }
        let uses = self.get_current_uses();
        self.set_current_uses(uses - 1);
        if let Some(r) = self.template.replace_on_use.clone() {
            if !in_container && !crafting && has_container {
                let full = module_dot_type(self.get_module(), &r);
                let item = ctx.create_item(&full).map(|mut i| {
                    i.color_red = self.color_red;
                    i.color_green = self.color_green;
                    i.color_blue = self.color_blue;
                    i.col = [self.color_red, self.color_green, self.color_blue, 1.0];
                    i.custom_color = true;
                    i.model_index = self.model_index;
                    i.copy_condition_states_from(self, ctx);
                    i.favorite = self.favorite;
                    i
                });
                out.added = Some(AddedItem {
                    full_type: full,
                    item,
                    takes_hands: false,
                });
            }
        }
        if self.get_current_uses() <= 0 {
            if self.is_keep_on_deplete() {
                return out;
            }
            if has_container {
                out.remove_from_hands = !matches!(self.kind, ItemKind::Weapon(_));
                out.remove = true;
            }
        } else if need_sync {
            out.sync = true;
        }
        out
    }

    /// `Clothing.Use(bCrafting, bInContainer)`: `Unwear()` at one use
    /// left, then `Use(bCrafting, bInContainer, false)`.
    pub fn clothing_use(
        &mut self,
        crafting: bool,
        in_container: bool,
        owner: &dyn ItemOwner,
        ctx: &dyn ItemContext,
    ) -> UseOutcome {
        let unwear = is_clothing(self) && self.uses <= 1;
        let mut out = self.use_with(crafting, in_container, false, owner, ctx);
        out.unwear = unwear;
        out
    }

    /// `Use()`.
    pub fn use_in(&mut self, owner: &dyn ItemOwner, ctx: &dyn ItemContext) -> UseOutcome {
        self.use_with(false, false, false, owner, ctx)
    }

    /// `copyConditionStatesFrom(other)` without the clothing copy, head
    /// condition and sharpness (components not modelled here).
    pub fn copy_condition_states_from(&mut self, other: &InventoryItem, ctx: &dyn ItemContext) {
        // Legacy synchronous adapter. The actual Lua caller uses a fallible
        // source enumeration and preserves the actual values' identities.
        for (key, value) in other
            .mod_data_own_entries()
            .expect("unborrowed condition ModData")
        {
            if let Value::Str(key) = key {
                if key.starts_with("condition:") {
                    self.set_mod_data_value(&key, Some(value))
                        .expect("unborrowed condition ModData");
                }
            }
        }
        let c = if self.condition_max == other.condition_max {
            other.condition
        } else {
            let perc = other.condition as f32 / other.condition_max as f32;
            (self.condition_max as f32 * perc) as i32
        };
        self.set_condition_sound(c, false, &crate::context::Loose, ctx);
        self.have_been_repaired = other.have_been_repaired;
        self.favorite = other.favorite;
        if self.template.class == ItemClass::Drainable
            && other.template.class == ItemClass::Drainable
        {
            // setUsedDelta(other.getCurrentUsesFloat()) = setCurrentUsesFloat.
            self.set_current_uses_float(other.get_current_uses_float(), Some(ctx));
        }
    }
}

/// `InventoryItem.useDelta`'s initial value.
pub const DEFAULT_USE_DELTA: f32 = 0.03125;

/// `Math.round(float)`.
fn java_round(v: f32) -> i32 {
    if v.is_nan() {
        0
    } else {
        (v as f64 + 0.5).floor() as i32
    }
}

/// `GameTime.checkHours(last, now)`: an unset (negative) or future last
/// update restarts at now.
fn check_hours(last: f32, now: f32) -> f32 {
    if last < 0.0 || now < last {
        now
    } else {
        last
    }
}

impl ItemContainer {
    /// `getContentsWeight()`: the items' `getUnequippedWeight()` (raw
    /// records weigh nothing).
    pub fn get_contents_weight_with(&self, ctx: &dyn ItemContext) -> f32 {
        let owner = crate::context::NoOwner;
        self.items
            .iter()
            .filter_map(SavedItem::as_item)
            .map(|i| i.get_unequipped_weight(ctx, &owner))
            .fold(0.0, |a, w| a + w)
    }

    /// Applies a [`UseOutcome`] of the item `id`: adds the new item, and
    /// removes the used one. Returns the new item's id; the hand changes
    /// are the caller's.
    pub fn apply_use(&mut self, id: i32, out: &mut UseOutcome, temperature: f32) -> Option<i32> {
        let mut new_id = None;
        if let Some(a) = out.added.as_mut() {
            if let Some(mut item) = a.item.take() {
                if let ItemKind::Food(f) = &mut item.kind {
                    f.heat = temperature;
                }
                new_id = Some(item.id);
                self.items.push(SavedItem::Item(Box::new(item)));
            }
        }
        if out.remove {
            if let Some(i) = self.items.iter().position(|i| i.id() == id) {
                self.items.remove(i);
            }
        }
        new_id
    }

    /// `item.Use()` for the item `id` in this container.
    pub fn use_item(
        &mut self,
        id: i32,
        owner: &dyn ItemOwner,
        ctx: &dyn ItemContext,
    ) -> Option<UseOutcome> {
        let item = self
            .items
            .iter_mut()
            .filter_map(SavedItem::as_item_mut)
            .find(|i| i.id == id)?;
        let mut out = item.use_in(owner, ctx);
        self.apply_use(id, &mut out, owner.container_temperature());
        Some(out)
    }
}

/// `FluidContainer.getAmount()` of an item's decoded `FluidContainer`
/// component block (`None` without one or when it does not decode).
pub fn fluid_container_amount(
    item: &InventoryItem,
    entity: &dyn pz_entity::EntityContext,
    _world_version: i32,
) -> Option<f32> {
    item.get_fluid_container(entity).map(|f| f.get_amount())
}
