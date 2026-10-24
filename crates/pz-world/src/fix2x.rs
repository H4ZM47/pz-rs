//! `IsoChunk.Fix2x(String)`: renames tiles in maps not marked `fixed2x`.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::java::GameRand;

fn fix2x_map() -> &'static HashMap<String, String> {
    static MAP: OnceLock<HashMap<String, String>> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut m = HashMap::new();
        for i in 48..=51 {
            m.insert(format!("blends_streetoverlays_01_{}", i), "".to_string());
        }
        m.insert("fencing_01_14".to_string(), "".to_string());
        m.insert("fencing_01_15".to_string(), "".to_string());
        m.insert("fencing_01_22".to_string(), "".to_string());
        m.insert("fencing_01_23".to_string(), "".to_string());
        m.insert("fencing_01_30".to_string(), "".to_string());
        m.insert("fencing_01_31".to_string(), "".to_string());
        m.insert("fencing_01_38".to_string(), "".to_string());
        m.insert("fencing_01_39".to_string(), "".to_string());
        m.insert("fencing_01_46".to_string(), "".to_string());
        m.insert("fencing_01_47".to_string(), "".to_string());
        m.insert("fencing_01_62".to_string(), "".to_string());
        m.insert("fencing_01_63".to_string(), "".to_string());
        m.insert("fencing_01_70".to_string(), "".to_string());
        m.insert("fencing_01_71".to_string(), "".to_string());
        m.insert(
            "fixtures_bathroom_02_2".to_string(),
            "fixtures_bathroom_02_22".to_string(),
        );
        m.insert(
            "fixtures_bathroom_02_20".to_string(),
            "fixtures_bathroom_02_21".to_string(),
        );
        m.insert(
            "fixtures_bathroom_02_21".to_string(),
            "fixtures_bathroom_02_20".to_string(),
        );
        for i in 26..=29 {
            m.insert(
                format!("fixtures_bathroom_02_{}", i),
                format!("fixtures_bathroom_02_{}", i + 6),
            );
        }
        m.insert(
            "fixtures_counters_01_16".to_string(),
            "fixtures_counters_01_45".to_string(),
        );
        m.insert(
            "fixtures_counters_01_17".to_string(),
            "fixtures_counters_01_43".to_string(),
        );
        m.insert(
            "fixtures_counters_01_18".to_string(),
            "fixtures_counters_01_41".to_string(),
        );
        m.insert(
            "fixtures_counters_01_19".to_string(),
            "fixtures_counters_01_47".to_string(),
        );
        m.insert(
            "fixtures_counters_01_24".to_string(),
            "fixtures_counters_01_26".to_string(),
        );
        m.insert(
            "fixtures_counters_01_25".to_string(),
            "fixtures_counters_01_27".to_string(),
        );
        for i in 0..=7 {
            m.insert(format!("fixtures_railings_01_{}", i), "".to_string());
        }
        for i in 8..=12 {
            m.insert(
                format!("fixtures_railings_01_{}", i),
                format!("fixtures_railings_01_{}", i + 72),
            );
        }
        m.insert(
            "fixtures_railings_01_13".to_string(),
            "fixtures_railings_01_84".to_string(),
        );
        for i in 16..=17 {
            m.insert(
                format!("fixtures_railings_01_{}", i),
                format!("fixtures_railings_01_{}", i + 72),
            );
        }
        m.insert(
            "fixtures_railings_01_18".to_string(),
            "fixtures_railings_01_91".to_string(),
        );
        m.insert(
            "fixtures_railings_01_19".to_string(),
            "fixtures_railings_01_85".to_string(),
        );
        m.insert("fixtures_railings_01_20".to_string(), "".to_string());
        m.insert(
            "fixtures_railings_01_21".to_string(),
            "fixtures_railings_01_89".to_string(),
        );
        m.insert(
            "floors_exterior_natural_01_0".to_string(),
            "blends_natural_01_16".to_string(),
        );
        m.insert(
            "floors_exterior_natural_01_1".to_string(),
            "blends_natural_01_32".to_string(),
        );
        m.insert(
            "floors_exterior_natural_01_2".to_string(),
            "blends_natural_01_48".to_string(),
        );
        m.insert(
            "floors_rugs_01_0".to_string(),
            "floors_rugs_01_6".to_string(),
        );
        m.insert(
            "floors_rugs_01_6".to_string(),
            "floors_rugs_01_0".to_string(),
        );
        m.insert(
            "floors_rugs_01_1".to_string(),
            "floors_rugs_01_7".to_string(),
        );
        m.insert(
            "floors_rugs_01_7".to_string(),
            "floors_rugs_01_1".to_string(),
        );
        m.insert(
            "floors_rugs_01_8".to_string(),
            "floors_rugs_01_14".to_string(),
        );
        m.insert(
            "floors_rugs_01_14".to_string(),
            "floors_rugs_01_8".to_string(),
        );
        m.insert(
            "floors_rugs_01_9".to_string(),
            "floors_rugs_01_15".to_string(),
        );
        m.insert(
            "floors_rugs_01_15".to_string(),
            "floors_rugs_01_9".to_string(),
        );
        m.insert(
            "floors_rugs_01_16".to_string(),
            "floors_rugs_01_22".to_string(),
        );
        m.insert(
            "floors_rugs_01_22".to_string(),
            "floors_rugs_01_16".to_string(),
        );
        m.insert(
            "floors_rugs_01_17".to_string(),
            "floors_rugs_01_23".to_string(),
        );
        m.insert(
            "floors_rugs_01_23".to_string(),
            "floors_rugs_01_17".to_string(),
        );
        m.insert(
            "furniture_bedding_01_42".to_string(),
            "furniture_bedding_01_43".to_string(),
        );
        m.insert(
            "furniture_bedding_01_43".to_string(),
            "furniture_bedding_01_42".to_string(),
        );
        m.insert(
            "furniture_bedding_01_44".to_string(),
            "furniture_bedding_01_47".to_string(),
        );
        m.insert(
            "furniture_bedding_01_47".to_string(),
            "furniture_bedding_01_45".to_string(),
        );
        m.insert(
            "furniture_bedding_01_45".to_string(),
            "furniture_bedding_01_46".to_string(),
        );
        m.insert(
            "furniture_bedding_01_46".to_string(),
            "furniture_bedding_01_44".to_string(),
        );
        m.insert(
            "furniture_tables_low_01_4".to_string(),
            "furniture_tables_low_01_5".to_string(),
        );
        m.insert(
            "furniture_tables_low_01_5".to_string(),
            "furniture_tables_low_01_4".to_string(),
        );
        for i in 0..=5 {
            m.insert(
                format!("location_business_machinery_{}", i),
                format!("location_business_machinery_01_{}", i),
            );
            m.insert(
                format!("location_business_machinery_{}", i + 8),
                format!("location_business_machinery_01_{}", i + 8),
            );
            m.insert(
                format!("location_ business_machinery_{}", i),
                format!("location_business_machinery_01_{}", i),
            );
            m.insert(
                format!("location_ business_machinery_{}", i + 8),
                format!("location_business_machinery_01_{}", i + 8),
            );
        }
        for i in 44..=47 {
            m.insert(
                format!("location_hospitality_sunstarmotel_01_{}", i),
                "".to_string(),
            );
        }
        for i in 52..=55 {
            m.insert(
                format!("location_hospitality_sunstarmotel_01_{}", i),
                "".to_string(),
            );
        }
        m.insert(
            "location_hospitality_sunstarmotel_02_24".to_string(),
            "location_hospitality_sunstarmotel_02_28".to_string(),
        );
        m.insert(
            "location_hospitality_sunstarmotel_02_26".to_string(),
            "location_hospitality_sunstarmotel_02_28".to_string(),
        );
        m.insert(
            "location_hospitality_sunstarmotel_02_33".to_string(),
            "location_hospitality_sunstarmotel_02_29".to_string(),
        );
        m.insert(
            "location_restaurant_bar_01_0".to_string(),
            "location_restaurant_bar_01_1".to_string(),
        );
        m.insert(
            "location_restaurant_bar_01_1".to_string(),
            "location_restaurant_bar_01_0".to_string(),
        );
        m.insert(
            "location_restaurant_bar_01_2".to_string(),
            "location_restaurant_bar_01_7".to_string(),
        );
        m.insert(
            "location_restaurant_bar_01_3".to_string(),
            "location_restaurant_bar_01_6".to_string(),
        );
        m.insert(
            "location_restaurant_bar_01_4".to_string(),
            "location_restaurant_bar_01_5".to_string(),
        );
        m.insert(
            "location_restaurant_bar_01_5".to_string(),
            "location_restaurant_bar_01_4".to_string(),
        );
        m.insert(
            "location_restaurant_bar_01_6".to_string(),
            "location_restaurant_bar_01_3".to_string(),
        );
        m.insert(
            "location_restaurant_bar_01_7".to_string(),
            "location_restaurant_bar_01_2".to_string(),
        );
        m.insert(
            "location_restaurant_bar_01_16".to_string(),
            "location_restaurant_bar_01_45".to_string(),
        );
        m.insert(
            "location_restaurant_bar_01_17".to_string(),
            "location_restaurant_bar_01_44".to_string(),
        );
        m.insert(
            "location_restaurant_bar_01_18".to_string(),
            "location_restaurant_bar_01_46".to_string(),
        );
        for i in 19..=22 {
            m.insert(
                format!("location_restaurant_bar_01_{}", i),
                format!("location_restaurant_bar_01_{}", i + 33),
            );
        }
        m.insert(
            "location_restaurant_bar_01_23".to_string(),
            "location_restaurant_bar_01_47".to_string(),
        );
        m.insert(
            "location_restaurant_pie_01_8".to_string(),
            "location_restaurant_pie_01_5".to_string(),
        );
        m.insert(
            "location_restaurant_pie_01_14".to_string(),
            "location_restaurant_pie_01_10".to_string(),
        );
        m.insert(
            "location_restaurant_pie_01_15".to_string(),
            "location_restaurant_pie_01_11".to_string(),
        );
        m.insert(
            "location_restaurant_pie_01_22".to_string(),
            "location_restaurant_pie_01_14".to_string(),
        );
        m.insert(
            "location_restaurant_pie_01_23".to_string(),
            "location_restaurant_pie_01_15".to_string(),
        );
        m.insert(
            "location_restaurant_pie_01_54".to_string(),
            "location_restaurant_pie_01_55".to_string(),
        );
        m.insert(
            "location_restaurant_pie_01_55".to_string(),
            "location_restaurant_pie_01_54".to_string(),
        );
        m.insert(
            "location_pizzawhirled_01_32".to_string(),
            "location_pizzawhirled_01_34".to_string(),
        );
        m.insert(
            "location_pizzawhirled_01_34".to_string(),
            "location_pizzawhirled_01_32".to_string(),
        );
        m.insert(
            "location_restaurant_seahorse_01_0".to_string(),
            "location_restaurant_seahorse_01_1".to_string(),
        );
        m.insert(
            "location_restaurant_seahorse_01_1".to_string(),
            "location_restaurant_seahorse_01_0".to_string(),
        );
        m.insert(
            "location_restaurant_seahorse_01_4".to_string(),
            "location_restaurant_seahorse_01_5".to_string(),
        );
        m.insert(
            "location_restaurant_seahorse_01_5".to_string(),
            "location_restaurant_seahorse_01_4".to_string(),
        );
        for i in 8..=13 {
            m.insert(
                format!("location_restaurant_seahorse_01_{}", i),
                format!("location_restaurant_seahorse_01_{}", i + 8),
            );
        }
        for i in 22..=23 {
            m.insert(
                format!("location_restaurant_seahorse_01_{}", i),
                format!("location_restaurant_seahorse_01_{}", i - 12),
            );
        }
        for i in 44..=47 {
            m.insert(
                format!("location_restaurant_seahorse_01_{}", i),
                format!("location_restaurant_seahorse_01_{}", i + 4),
            );
        }
        for i in 48..=51 {
            m.insert(
                format!("location_restaurant_seahorse_01_{}", i),
                format!("location_restaurant_seahorse_01_{}", i - 4),
            );
        }
        m.insert(
            "location_restaurant_spiffos_01_56".to_string(),
            "location_restaurant_spiffos_01_58".to_string(),
        );
        m.insert(
            "location_restaurant_spiffos_01_58".to_string(),
            "location_restaurant_spiffos_01_56".to_string(),
        );
        m.insert(
            "location_shop_fossoil_01_45".to_string(),
            "location_shop_fossoil_01_44".to_string(),
        );
        m.insert(
            "location_shop_fossoil_01_46".to_string(),
            "location_shop_fossoil_01_45".to_string(),
        );
        m.insert(
            "location_shop_fossoil_01_57".to_string(),
            "location_shop_fossoil_01_58".to_string(),
        );
        m.insert(
            "location_shop_fossoil_01_58".to_string(),
            "location_shop_fossoil_01_59".to_string(),
        );
        m.insert(
            "location_shop_greenes_01_13".to_string(),
            "location_shop_greenes_01_18".to_string(),
        );
        m.insert(
            "location_shop_greenes_01_15".to_string(),
            "location_shop_greenes_01_19".to_string(),
        );
        m.insert(
            "location_shop_greenes_01_21".to_string(),
            "location_shop_greenes_01_16".to_string(),
        );
        m.insert(
            "location_shop_greenes_01_22".to_string(),
            "location_shop_greenes_01_13".to_string(),
        );
        m.insert(
            "location_shop_greenes_01_23".to_string(),
            "location_shop_greenes_01_17".to_string(),
        );
        m.insert(
            "location_shop_greenes_01_67".to_string(),
            "location_shop_greenes_01_70".to_string(),
        );
        m.insert(
            "location_shop_greenes_01_68".to_string(),
            "location_shop_greenes_01_67".to_string(),
        );
        m.insert(
            "location_shop_greenes_01_70".to_string(),
            "location_shop_greenes_01_71".to_string(),
        );
        m.insert(
            "location_shop_greenes_01_75".to_string(),
            "location_shop_greenes_01_78".to_string(),
        );
        m.insert(
            "location_shop_greenes_01_76".to_string(),
            "location_shop_greenes_01_75".to_string(),
        );
        m.insert(
            "location_shop_greenes_01_78".to_string(),
            "location_shop_greenes_01_79".to_string(),
        );
        for i in 0..=16 {
            m.insert(
                format!("vegetation_foliage_01_{}", i),
                "randBush".to_string(),
            );
        }
        m.insert(
            "vegetation_groundcover_01_0".to_string(),
            "blends_grassoverlays_01_16".to_string(),
        );
        m.insert(
            "vegetation_groundcover_01_1".to_string(),
            "blends_grassoverlays_01_8".to_string(),
        );
        m.insert(
            "vegetation_groundcover_01_2".to_string(),
            "blends_grassoverlays_01_0".to_string(),
        );
        m.insert(
            "vegetation_groundcover_01_3".to_string(),
            "blends_grassoverlays_01_64".to_string(),
        );
        m.insert(
            "vegetation_groundcover_01_4".to_string(),
            "blends_grassoverlays_01_56".to_string(),
        );
        m.insert(
            "vegetation_groundcover_01_5".to_string(),
            "blends_grassoverlays_01_48".to_string(),
        );
        m.insert("vegetation_groundcover_01_6".to_string(), "".to_string());
        m.insert(
            "vegetation_groundcover_01_44".to_string(),
            "blends_grassoverlays_01_40".to_string(),
        );
        m.insert(
            "vegetation_groundcover_01_45".to_string(),
            "blends_grassoverlays_01_32".to_string(),
        );
        m.insert(
            "vegetation_groundcover_01_46".to_string(),
            "blends_grassoverlays_01_24".to_string(),
        );
        m.insert(
            "vegetation_groundcover_01_16".to_string(),
            "d_plants_1_53".to_string(),
        );
        m.insert(
            "vegetation_groundcover_01_17".to_string(),
            "d_plants_1_53".to_string(),
        );
        for i in 18..=23 {
            m.insert(
                format!("vegetation_groundcover_01_{}", i),
                "randPlant".to_string(),
            );
        }
        for i in 20..=23 {
            m.insert(
                format!("walls_exterior_house_01_{}", i),
                format!("walls_exterior_house_01_{}", i + 12),
            );
            m.insert(
                format!("walls_exterior_house_01_{}", i + 8),
                format!("walls_exterior_house_01_{}", i + 8 + 12),
            );
        }
        for i in 24..=41 {
            m.insert(
                format!("walls_exterior_roofs_01_{}", i),
                format!("walls_exterior_roofs_03_{}", i),
            );
        }
        m
    })
}

/// `IsoChunk.Fix2x(tileName)`. Returns the replacement name (possibly empty)
/// or the input unchanged; `randBush`/`randPlant` draw from `rand`.
pub fn fix2x(tile_name: &str, rand: &mut dyn GameRand) -> String {
    let Some(s) = fix2x_map().get(tile_name) else {
        return tile_name.to_string();
    };
    if s == "randBush" {
        let id = 64 + rand.next(16);
        return format!("f_bushes_1_{id}");
    }
    if s == "randPlant" {
        let id = rand.next(4) * 16 + rand.next(8);
        return format!("d_plants_1_{id}");
    }
    s.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Zero;
    impl GameRand for Zero {
        fn next(&mut self, _max: i32) -> i32 {
            0
        }
    }

    #[test]
    fn renames() {
        assert_eq!(fix2x("fencing_01_14", &mut Zero), "");
        assert_eq!(
            fix2x("fixtures_bathroom_02_27", &mut Zero),
            "fixtures_bathroom_02_33"
        );
        assert_eq!(fix2x("vegetation_foliage_01_3", &mut Zero), "f_bushes_1_64");
        assert_eq!(
            fix2x("vegetation_groundcover_01_20", &mut Zero),
            "d_plants_1_0"
        );
        assert_eq!(
            fix2x("walls_exterior_house_01_28", &mut Zero),
            "walls_exterior_house_01_40"
        );
        assert_eq!(fix2x("not_a_tile", &mut Zero), "not_a_tile");
    }
}
