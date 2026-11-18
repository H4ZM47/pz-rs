use pz_kahlua::{TableRef, Thread, Value};

use super::fetch::compare_type_for_test as compare_type;

#[test]
fn compare_type_matches_one_slash_separated_entry() {
    assert!(compare_type("Sledgehammer", "Sledgehammer"));
    assert!(!compare_type("Sledgehammer", "Sledgehammer2"));
    assert!(compare_type("Hammer/Sledgehammer", "Sledgehammer"));
    assert!(compare_type("Sledgehammer/Hammer", "Sledgehammer"));
    assert!(compare_type("A/Sledgehammer/B", "Sledgehammer"));
    assert!(!compare_type("A/Sledgehammer2", "Sledgehammer"));
    assert!(!compare_type("A/B", "C"));
}

#[test]
fn append_and_contains_value_use_identity() {
    let t = TableRef::new();
    let a = Value::Table(TableRef::new());
    let b = Value::Table(TableRef::new());
    super::append(&t, a.clone());
    assert!(super::contains_value(Some(&t), &a));
    assert!(!super::contains_value(Some(&t), &b));
    assert!(!super::contains_value(None, &a));
    super::append(&t, b.clone());
    assert_eq!(t.len(), 2);
    assert!(super::contains_key(&t, &Value::Num(2.0)));
}

#[test]
fn registers_the_static_class() {
    let mut t = Thread::new(TableRef::new());
    pz_kahlua::stdlib::install(&mut t, None).unwrap();
    super::register(&mut t);
    let class = t.globals().rawget(&Value::str("ISWorldObjectContextMenuLogic"));
    let class = class.as_table().expect("global class table");
    for name in ["fetch", "createMenuEntries", "callCustomFunction"] {
        assert!(matches!(class.rawget(&Value::str(name)), Value::Native(_)), "{name}");
    }
}

#[test]
fn fetch_with_a_null_fetch_table_throws_a_null_pointer() {
    let mut t = Thread::new(TableRef::new());
    pz_kahlua::stdlib::install(&mut t, None).unwrap();
    super::register(&mut t);
    let f = pz_kahlua::stdlib::load_string(
        "return pcall(ISWorldObjectContextMenuLogic.fetch, nil, nil, 0, true)",
        Some("test"),
        t.globals().clone(),
    )
    .unwrap();
    let r = t.call(&f, &[]).unwrap();
    assert!(matches!(r[0], Value::Bool(false)));
}

#[test]
fn pre_pickup_ground_cover_item_adds_the_remove_option() {
    let mut t = Thread::new(TableRef::new());
    pz_kahlua::stdlib::install(&mut t, None).unwrap();
    let f = pz_kahlua::stdlib::load_string(
        r#"
        GroundCoverItems = { ["4Stones"] = "Stone2", Gone = "Missing" }
        local items = { Stone2 = { getDisplayName = function() return "Stones" end } }
        function getScriptManager() return { getItem = function(_, n) return items[n] end } end
        function getText(key, a) if a then return key .. ":" .. a end return key end
        Texture = { trygetTexture = function(n) return { splitIcon = function() return "icon:" .. n end } end }
        ISWorldObjectContextMenu = { onPickupGroundCoverItem = function() end }
        local menu = { added = {} }
        function menu:addGetUpOption(name, target, f, player, object)
            local o = { name = name, target = target, f = f, player = player, object = object }
            self.added[#self.added + 1] = o
            return o
        end
        local function obj(custom)
            return {
                getProperty = function(_, k) if k == "CustomName" then return custom end end,
                getSpriteName = function() return "sprite_" .. tostring(custom) end,
            }
        end
        return menu, obj("4Stones"), obj("Gone"), obj("Bush"), obj(nil)
        "#,
        Some("test"),
        t.globals().clone(),
    )
    .unwrap();
    let r = t.call(&f, &[]).unwrap();
    let (menu, worldobjects) = (r[0].clone(), TableRef::new());
    let mut cx = super::Cx { t: &mut t };
    for object in &r[1..] {
        super::helpers::pre_pickup_ground_cover_item(&mut cx, &menu, &worldobjects, 2.0, object).unwrap();
    }
    let added = menu.as_table().unwrap().get_str("added");
    let added = added.as_table().unwrap();
    assert_eq!(added.len(), 1, "only the item that has a script");
    let option = added.rawget(&Value::Num(1.0));
    let option = option.as_table().unwrap();
    assert_eq!(option.get_str("name").as_str(), Some("ContextMenu_Remove_GroundItem:Stones"));
    assert!(matches!(option.get_str("target"), Value::Table(ref w) if w.ptr_eq(&worldobjects)));
    assert!(!option.get_str("f").is_nil());
    assert_eq!(option.get_str("player").as_num(), Some(2.0));
    assert_eq!(option.get_str("iconTexture").as_str(), Some("icon:sprite_4Stones"));
    // A CustomName with no GroundCoverItems table throws like the cast.
    t.globals().rawset(Value::str("GroundCoverItems"), Value::Nil);
    let mut cx = super::Cx { t: &mut t };
    assert!(super::helpers::pre_pickup_ground_cover_item(&mut cx, &menu, &worldobjects, 2.0, &r[1]).is_err());
    assert!(super::helpers::pre_pickup_ground_cover_item(&mut cx, &menu, &worldobjects, 2.0, &r[4]).is_ok());
}
