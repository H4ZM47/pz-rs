use super::*;
use crate::character::tests::fixture;
use crate::population::zombie_defs::{ZoneView, ZzdOutfit, ZzdZone};
use crate::population::{OutfitContext, OutfitManager, Population};
use crate::CharacterKind;
use std::cell::RefCell;
use std::rc::Rc;

struct World;
impl ZoneWorld for World {
    fn square_room(&self, _x: i32, _y: i32, _z: i32) -> Option<Option<String>> {
        Some(None)
    }
    fn zones_at(&self, _x: i32, _y: i32, _z: i32) -> Vec<ZoneView> {
        Vec::new()
    }
}
impl ZombieDressWorld for World {
    fn check_dirty_zones(&mut self, zones: &mut ZombiesZoneDefinition) {
        zones.dirty = false;
    }
    fn check_dirty_underwear(&mut self, underwear: &mut UnderwearDefinition) {
        underwear.dirty = false;
    }
    fn check_dirty_weapons(&mut self, weapons: &mut AttachedWeaponDefinitions) {
        weapons.dirty = false;
    }
    fn create_item(&mut self, _item_type: &str) -> Option<InventoryItem> {
        None
    }
    fn loot_modifier(&self, _item_type: &str) -> f32 {
        1.0
    }
}

#[test]
fn dressing_in_persistent_outfits() {
    let (sm, mut chars) = fixture();
    let doc = b"<outfitManager><m_FemaleOutfits><m_Name>Nurse</m_Name></m_FemaleOutfits>\
        <m_MaleOutfits><m_Name>Generic01</m_Name></m_MaleOutfits></outfitManager>";
    let om = OutfitManager::from_xml(doc, "game").unwrap();
    let pop = Population::default();
    let clothing = pz_inventory::ClothingItems::default();
    let group = chars.body_locations.get_group("Human").unwrap();
    let octx = OutfitContext { clothing_items: &clothing, population: &pop, hair_common_colors: &[], world_age_days: 0.0, no_black_clothes: false };
    let dress = DressContext { outfit: &octx, outfits: &om, clothing: &clothing, scripts: &sm, group: &group };
    let mut outfits = ZombieOutfits::default();
    outfits.zones.set_zones(vec![ZzdZone::new(
        "Army".into(),
        -1,
        -1,
        -1,
        -1,
        vec![ZzdOutfit::new(Some("Nurse".into()), 10.0, None, -1, None, None, Some("Bald:100"), None, None).unwrap()],
    )]);
    rand::set_seed(9);
    let custom = outfits.zones.register_custom_outfits();
    outfits.persistent.init(&om, &custom, false, false, None);
    let mut z: GameCharacter<i32> = GameCharacter::new(&sm, CharacterKind::Zombie);
    let mut d = crate::SurvivorDesc::new(&mut chars, &sm).unwrap();
    d.set_female(true);
    z.descriptor = Some(Rc::new(RefCell::new(d)));
    let settings = DressSettings { clothing_degradation: 1, remove_zombie_loot: false, tutorial: false, world_age_days: 0.0, world_age_days_since_begin: 0.0 };
    let mut world = World;

    // No names: `SurvivorFactory.getRandomForename` throws.
    let id = outfits.persistent.pick_outfit_female("Nurse");
    {
        let mut cx = ZombieDressCx { chars: &chars, dress: &dress, outfits: &mut outfits, world: &mut world, settings };
        let e = z.dress_in_persistent_outfit_id(id, &mut cx).unwrap_err();
        assert_eq!(e.to_string(), "java.lang.IndexOutOfBoundsException: Index 0 out of bounds for length 0");
    }
    chars.add_female_forename("Ann");
    let mut cx = ZombieDressCx { chars: &chars, dress: &dress, outfits: &mut outfits, world: &mut world, settings };
    z.dress_in_persistent_outfit_id(id, &mut cx).unwrap();
    assert!(z.zombie.persistent_outfit_init);
    assert_eq!(z.human_visual.as_ref().unwrap().outfit.as_deref(), Some("Nurse"));
    assert_eq!(z.descriptor.as_ref().unwrap().borrow().forename.as_deref(), Some("SurvivorName_Ann"));
    assert_eq!((z.zombie.sprite_name.as_str(), z.zombie.hurt_sound.as_str()), ("KateZ", "FemaleZombieHurt"));
    // The zone outfit dresses in the outfit, then the zone's hair.
    let id = cx.outfits.persistent.pick_outfit_female("ZZD.Army.Nurse");
    z.dress_in_persistent_outfit_id(id, &mut cx).unwrap();
    assert_eq!(z.human_visual.as_ref().unwrap().hair_model.as_deref(), Some("Bald"));
    // The id 0: cleared, nothing dressed.
    z.dress_in_persistent_outfit_id(0, &mut cx).unwrap();
    assert_eq!((z.zombie.persistent_outfit_id, z.human_visual.as_ref().unwrap().hair_model.as_deref()), (0, None));
    // A loaded zombie dresses in its id when added to the model manager.
    z.zombie.persistent_outfit_init = false;
    z.zombie.persistent_outfit_id = id;
    z.model_manager_dress_in_random_outfit(&mut cx).unwrap();
    assert_eq!(z.zombie.persistent_outfit_id, id);
    assert!(!z.remove_fallen_hat(id | NO_HAT_BIT, &cx));
}
