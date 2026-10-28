use super::*;
use crate::env::ItemEnv;
use crate::io::{Reader, Writer};
use crate::item::{EggData, InventoryItem, ItemKind};
use crate::{ItemClass, ItemTemplate};
use std::rc::Rc;

struct Env;
impl ItemEnv for Env {}

fn item() -> InventoryItem {
    let mut template = ItemTemplate::new(ItemClass::Food, "Base", "Apple");
    template.days_fresh = 2;
    template.days_totally_rotten = 4;
    InventoryItem::create(Rc::new(template), 1)
}

fn food() -> FoodData {
    let ItemKind::Food(food) = item().kind else {
        panic!("food factory returned another kind")
    };
    *food
}

#[test]
fn can_be_frozen_is_constructor_state_and_never_a_wire_field_or_load_reset() {
    let mut item = item();
    assert!(
        UPDATE_RATE.get().is_some(),
        "Food construction initializes its class timer"
    );
    let ItemKind::Food(f) = &mut item.kind else {
        panic!()
    };
    assert!(f.can_be_frozen);
    f.age = 1.25;
    f.last_aged = 12.5;
    let mut first = Writer::new();
    item.save(&mut first, &Env).unwrap();
    let ItemKind::Food(f) = &mut item.kind else {
        panic!()
    };
    f.can_be_frozen = false;
    let mut second = Writer::new();
    item.save(&mut second, &Env).unwrap();
    assert_eq!(first.buf, second.buf);

    // InventoryItem.load receives the bytes after registry ID/save type.
    let mut reader = Reader::new(&first.buf[3..]);
    item.load(&mut reader, 249, &Env).unwrap();
    assert_eq!(reader.remaining(), 0);
    let ItemKind::Food(f) = &item.kind else {
        panic!()
    };
    assert!(!f.can_be_frozen, "load must retain existing runtime state");
    assert_eq!((f.age, f.last_aged), (1.25, 12.5));
}

#[test]
fn checked_time_prefixes_initialize_rewind_and_preserve_unordered_values() {
    let mut f = food();
    assert_eq!(f.begin_age_update(12.0), None);
    assert_eq!(f.last_aged, 12.0);
    assert_eq!(f.begin_age_update(10.0), None);
    assert_eq!(f.last_aged, 10.0);
    assert_eq!(f.begin_age_update(10.25), Some(0.25));
    assert_eq!(f.last_aged, 10.0, "forward prefix is not a commit");
    f.last_frozen_update = -1.0;
    assert_eq!(f.begin_freezing_update(8.0), None);
    f.last_frozen_update = 20.0;
    assert_eq!(f.begin_freezing_update(8.0), None);
    assert_eq!(f.last_frozen_update, 8.0);
    assert_eq!(f.begin_freezing_update(8.5), Some(0.5));
    assert_eq!(f.last_frozen_update, 8.0);

    let nan = f32::from_bits(0x7fc0_0123);
    f.last_aged = nan;
    assert_eq!(f.begin_age_update(20.0), None);
    assert_eq!(f.last_aged.to_bits(), nan.to_bits());
    f.last_frozen_update = -0.0;
    assert_eq!(f.begin_freezing_update(0.0), None);
    assert_eq!(f.last_frozen_update.to_bits(), (-0.0f32).to_bits());
    f.last_aged = 7.0;
    assert_eq!(f.begin_age_update(nan), None);
    assert_eq!(f.last_aged, 7.0);
}

#[test]
fn age_delta_commits_float_fields_and_reports_both_transitions() {
    let mut f = food();
    f.age = 1.5;
    f.last_aged = 12.0;
    let elapsed = f.begin_age_update(72.0).unwrap();
    let changed = f.apply_age_delta(72.0, elapsed, 1.0);
    assert_eq!(f.age, 4.0);
    assert_eq!(f.last_aged, 72.0);
    assert_eq!(
        changed.before,
        AgeState {
            fresh: true,
            rotten: false
        }
    );
    assert_eq!(
        changed.after,
        AgeState {
            fresh: false,
            rotten: true
        }
    );
    assert!(changed.changed());
    assert!(!f.apply_age_delta(72.0, 0.0, 1.0).changed());
}

#[test]
fn age_math_preserves_float_subtraction_then_double_integration() {
    let mut f = food();
    f.age = 0.0;
    f.last_aged = 0.0;
    let elapsed = f.begin_age_update(0.1).unwrap();
    assert_eq!(elapsed, 0.10000000149011612f64);
    f.apply_age_delta(0.1, elapsed, food_rot_speed(1));
    assert_eq!(f.age.to_bits(), 0x3be8_1b4f);
}

#[test]
fn burnt_sentinel_nan_and_exact_boundaries_use_native_predicates() {
    let mut f = food();
    f.age = 2.0;
    assert_eq!(
        f.age_state(),
        AgeState {
            fresh: false,
            rotten: false
        }
    );
    f.age = 4.0;
    assert_eq!(
        f.age_state(),
        AgeState {
            fresh: false,
            rotten: true
        }
    );
    f.burnt = true;
    assert_eq!(
        f.age_state(),
        AgeState {
            fresh: false,
            rotten: false
        }
    );
    f.burnt = false;
    f.off_age = 1_000_000_000;
    f.off_age_max = 1_000_000_000;
    f.age = 2_000_000_000.0;
    assert_eq!(
        f.age_state(),
        AgeState {
            fresh: false,
            rotten: false
        }
    );
    f.off_age = 2;
    f.off_age_max = 4;
    f.age = f32::NAN;
    assert_eq!(
        f.age_state(),
        AgeState {
            fresh: false,
            rotten: false
        }
    );
}

#[test]
fn freezing_saturates_and_clears_only_fertilized_flag() {
    let mut f = food();
    let egg = EggData {
        time_to_hatch: 48,
        fertilized_time: 12,
        animal_hatch: "chicken".into(),
        animal_hatch_breed: "brown".into(),
        genome: vec![],
        mother_id: 91,
    };
    f.fertilized = true;
    f.egg = Some(egg.clone());
    f.apply_freezing(2.0);
    assert_eq!(f.freezing_time, 50.0);
    assert!(!f.frozen);
    assert!(!f.fertilized);
    assert_eq!(f.egg.as_ref(), Some(&egg));
    f.apply_freezing(2.0);
    assert_eq!(f.freezing_time, 100.0);
    assert!(f.frozen);
    f.apply_freezing(20.0);
    assert_eq!(f.freezing_time, 100.0);
}

#[test]
fn thawing_uses_fridge_and_hot_rates_and_preserves_partial_frozen_state() {
    for (fridge, hot, dt, expected) in [
        (false, false, 0.75, 50.0),
        (true, false, 0.75, 75.0),
        (false, true, 0.125, 50.0),
        (true, true, 0.125, 75.0),
    ] {
        let mut f = food();
        f.set_freezing_time(100.0);
        f.apply_thawing(dt, thaw_hours(fridge, hot));
        assert_eq!(f.freezing_time, expected);
        assert!(f.frozen, "partial thaw keeps the frozen flag");
        f.apply_thawing(10.0, thaw_hours(fridge, hot));
        assert_eq!(f.freezing_time.to_bits(), 0.0f32.to_bits());
        assert!(!f.frozen);
    }
}

#[test]
fn freezing_setter_preserves_nan_and_normalizes_endpoint_zero() {
    let mut f = food();
    f.frozen = true;
    let nan = f32::from_bits(0x7fc0_4321);
    f.set_freezing_time(nan);
    assert_eq!(f.freezing_time.to_bits(), nan.to_bits());
    assert!(f.frozen);
    f.set_freezing_time(-0.0);
    assert_eq!(f.freezing_time.to_bits(), 0.0f32.to_bits());
    assert!(!f.frozen);
    f.set_freezing_time(f32::INFINITY);
    assert_eq!(f.freezing_time, 100.0);
    assert!(f.frozen);
    f.set_freezing_time(f32::NEG_INFINITY);
    assert_eq!(f.freezing_time, 0.0);
    assert!(!f.frozen);
}

#[test]
fn frozen_age_is_zero_multiplication_including_nonfinite_values() {
    let mut f = food();
    f.age = 1.0;
    f.apply_age_delta(48.0, frozen_age_hours(24.0), 1.7);
    assert_eq!(f.age, 1.0);
    assert_eq!(f.last_aged, 48.0);
    assert!(frozen_age_hours(f64::INFINITY).is_nan());
    assert!(frozen_age_hours(f64::NAN).is_nan());
    assert_eq!(frozen_age_hours(-1.0).to_bits(), (-0.0f64).to_bits());
}

#[test]
fn sandbox_factor_mappings_include_native_default_branches() {
    assert_eq!(
        (1..=6).map(fridge_factor).collect::<Vec<_>>(),
        [0.4, 0.3, 0.2, 0.1, 0.03, 0.0]
    );
    assert_eq!(
        (1..=5).map(food_rot_speed).collect::<Vec<_>>(),
        [1.7, 1.4, 1.0, 0.7, 0.4]
    );
    for option in [i32::MIN, -1, 0, 7, i32::MAX] {
        assert_eq!(fridge_factor(option), 0.2);
        assert_eq!(food_rot_speed(option), 1.0);
    }
}

#[test]
fn heat_interpolation_clamps_factor_but_preserves_nonfinite_arithmetic() {
    assert_eq!(lerp_heat(1.0, 0.2, -1.0), 1.0);
    assert_eq!(lerp_heat(1.0, 0.2, 2.0), 0.19999999);
    assert_eq!(lerp_heat(1.0, 0.2, 0.5), 0.6);
    assert!(lerp_heat(1.0, 0.2, f32::NAN).is_nan());
    assert!(lerp_heat(f32::INFINITY, 1.0, 0.0).is_nan());
}

#[test]
fn outage_splits_at_shutoff_with_float_cooled_time_before_double_promotion() {
    let hours = outage_hours(36.0, 60.0, 2, fridge_factor(3));
    assert_eq!(hours.to_bits(), 0x402c_cccc_d000_0000);
    let mut f = food();
    f.age = 1.0;
    f.apply_age_delta(60.0, hours, 1.0);
    assert_eq!(f.age.to_bits(), 0x3fcc_cccd);
    assert_eq!(outage_hours(36.0, 48.0, 2, 0.25), 3.0);
    assert_eq!(outage_hours(48.0, 72.0, 2, 0.25), 24.0);
    assert_eq!(outage_hours(36.0, 60.0, -1, 0.25), 24.0);
    // The positive modifier's Java int product is -24 after wrapping.
    assert_eq!(outage_hours(0.0, 24.0, i32::MAX, 0.25), 24.0);
}

#[test]
fn java_min_is_distinct_from_float_min_for_nan_and_signed_zero() {
    let a = f32::from_bits(0x7fc0_0123);
    let b = f32::from_bits(0x7fc0_4567);
    assert_eq!(java_min(a, b).to_bits(), a.to_bits());
    assert_eq!(java_min(1.0, b).to_bits(), b.to_bits());
    assert_eq!(java_min(0.0, -0.0).to_bits(), (-0.0f32).to_bits());
    assert_eq!(java_min(-0.0, 0.0).to_bits(), (-0.0f32).to_bits());
    assert_eq!(java_min(0.0, 0.0).to_bits(), 0.0f32.to_bits());
}

#[test]
fn shared_rate_strict_edges_catchup_and_clock_rewind() {
    let mut rate = FoodAgeRate::new_at(0);
    assert!(!rate.check_at(1000));
    assert!(rate.check_at(1001));
    assert_eq!(rate.last(), 1000);
    assert!(!rate.check_at(2000));
    assert!(rate.check_at(2001));
    assert_eq!(rate.last(), 2000);
    assert!(rate.check_at(5001));
    assert_eq!(rate.last(), 5001, "more than three periods restarts at now");
    assert!(!rate.check_at(4000));
    assert_eq!(rate.last(), 5001);

    let mut rate = FoodAgeRate::new_at(0);
    assert!(rate.check_at(3000));
    assert_eq!(
        rate.last(),
        1000,
        "exactly three periods advances one period"
    );
    assert!(rate.check_at(3000));
    assert_eq!(rate.last(), 2000);
    assert!(!rate.check_at(3000));
    let mut rate = FoodAgeRate::new_at(i64::MAX - 500);
    assert!(rate.check_at(i64::MIN + 600));
    assert_eq!(rate.last(), i64::MIN + 499);
}

#[test]
fn a_false_shared_rate_result_does_not_gate_age_phases() {
    let mut rate = FoodAgeRate::new_at(0);
    let mut foods = [food(), food()];
    let mut stats = Vec::new();
    for f in &mut foods {
        stats.push(rate.check_at(1001));
        f.age = 1.0;
        f.last_aged = 0.0;
        let dt = f.begin_age_update(24.0).unwrap();
        assert!(f.apply_age_delta(24.0, dt, 1.0).changed());
    }
    assert_eq!(stats, [true, false]);
    assert!(foods.iter().all(|f| f.age == 2.0 && f.last_aged == 24.0));
}
