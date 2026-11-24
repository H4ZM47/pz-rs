//! `zombie.iso.objects.IsoFireplace` and `zombie.iso.objects.IsoBarbecue`
//! Lua bindings (`ISBBQToggle`, `ISBBQAddFuel`, `ISBBQLightFromKindle`,
//! `ISBBQLightFromLiterature`, `ISBBQExtinguish`, `ISBBQInsertPropaneTank`,
//! `ISBBQRemovePropaneTank`, `ISFireplaceAddFuel`, `ISFireplaceExtinguish`).

use super::*;
use crate::bind_world::square_coords;
use crate::expose::{bool_arg, expose_class, int_arg, str_arg};
use pz_kahlua::NativeCall;
use pz_world::objects::{IsoBarbecue, IsoFireplace, SpriteName};
use pz_world::ObjectSlot;

fn new_fireplace(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    // IsoFireplace.new(cell, square, sprite)
    let sq_val = c.arg(1);
    let coords = square_coords(&sq_val);
    let sprite = match c.arg(2) {
        Value::Str(s) => Some(s.to_string()),
        _ => str_arg(c, 2).map(|s| s.to_string()),
    };
    let mut body = IsoObject::new(0, ObjectKind::Fireplace(IsoFireplace::default()), None);
    if let Some(name) = sprite {
        body.sprite = construction::named_sprite(&name)?;
        body.sprite_name = Some(SpriteName::Str(name));
    }
    let slot = ObjectSlot::new(body);
    let val = object_value(slot.owner_slot())?;
    if let Some((x, y, z)) = coords {
        remember_square(&val, [x, y, z]);
    }
    Ok(c.push(val))
}

fn new_barbecue(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    // IsoBarbecue.new(cell, square, sprite)
    let sq_val = c.arg(1);
    let coords = square_coords(&sq_val);
    let sprite = match c.arg(2) {
        Value::Str(s) => Some(s.to_string()),
        _ => str_arg(c, 2).map(|s| s.to_string()),
    };
    let mut body = IsoObject::new(0, ObjectKind::Barbecue(IsoBarbecue::default()), None);
    if let Some(name) = sprite {
        body.sprite = construction::named_sprite(&name)?;
        body.sprite_name = Some(SpriteName::Str(name));
    }
    let slot = ObjectSlot::new(body);
    let val = object_value(slot.owner_slot())?;
    if let Some((x, y, z)) = coords {
        remember_square(&val, [x, y, z]);
    }
    Ok(c.push(val))
}

fn register_common_hearth(class: &crate::expose::ClassBuilder) {
    class.method("getFuelAmount", |c| {
        let obj = receiver_object(c)?;
        let body = obj.try_borrow().map_err(|_| borrowed("hearth"))?;
        let amt = match &body.kind {
            ObjectKind::Fireplace(f) => f.fuel_amount(),
            ObjectKind::Barbecue(b) => b.fuel_amount(),
            _ => 0,
        };
        Ok(c.push(amt))
    });
    class.method("setFuelAmount", |c| {
        let units = int_arg(c, 1);
        let obj = receiver_object(c)?;
        let mut body = obj.try_borrow_mut().map_err(|_| borrowed("hearth"))?;
        match &mut body.kind {
            ObjectKind::Fireplace(f) => f.set_fuel_amount(units),
            ObjectKind::Barbecue(b) => b.set_fuel_amount(units),
            _ => {}
        }
        Ok(0)
    });
    class.method("addFuel", |c| {
        let units = int_arg(c, 1);
        let obj = receiver_object(c)?;
        let mut body = obj.try_borrow_mut().map_err(|_| borrowed("hearth"))?;
        let new_amt = match &mut body.kind {
            ObjectKind::Fireplace(f) => {
                f.add_fuel(units);
                f.fuel_amount()
            }
            ObjectKind::Barbecue(b) => {
                b.add_fuel(units);
                b.fuel_amount()
            }
            _ => 0,
        };
        Ok(c.push(new_amt))
    });
    class.method("useFuel", |c| {
        let amount = int_arg(c, 1);
        let obj = receiver_object(c)?;
        let mut body = obj.try_borrow_mut().map_err(|_| borrowed("hearth"))?;
        let used = match &mut body.kind {
            ObjectKind::Fireplace(f) => f.use_fuel(amount),
            ObjectKind::Barbecue(b) => b.use_fuel(amount),
            _ => 0,
        };
        Ok(c.push(used))
    });
    class.method("setLit", |c| {
        let lit = bool_arg(c, 1);
        let obj = receiver_object(c)?;
        let mut body = obj.try_borrow_mut().map_err(|_| borrowed("hearth"))?;
        match &mut body.kind {
            ObjectKind::Fireplace(f) => f.lit = lit,
            ObjectKind::Barbecue(b) => b.lit = lit,
            _ => {}
        }
        Ok(0)
    });
    class.method("turnOn", |c| {
        let obj = receiver_object(c)?;
        let mut body = obj.try_borrow_mut().map_err(|_| borrowed("hearth"))?;
        match &mut body.kind {
            ObjectKind::Fireplace(f) => {
                f.turn_on();
            }
            ObjectKind::Barbecue(b) => {
                b.turn_on();
            }
            _ => {}
        }
        Ok(0)
    });
    class.method("turnOff", |c| {
        let obj = receiver_object(c)?;
        let mut body = obj.try_borrow_mut().map_err(|_| borrowed("hearth"))?;
        match &mut body.kind {
            ObjectKind::Fireplace(f) => {
                f.lit = false;
            }
            ObjectKind::Barbecue(b) => {
                b.turn_off();
            }
            _ => {}
        }
        Ok(0)
    });
    class.method("toggle", |c| {
        let obj = receiver_object(c)?;
        let mut body = obj.try_borrow_mut().map_err(|_| borrowed("hearth"))?;
        match &mut body.kind {
            ObjectKind::Fireplace(f) => {
                if f.lit {
                    f.lit = false;
                } else {
                    f.turn_on();
                }
            }
            ObjectKind::Barbecue(b) => {
                if b.lit {
                    b.turn_off();
                } else {
                    b.turn_on();
                }
            }
            _ => {}
        }
        Ok(0)
    });
    class.method("extinguish", |c| {
        let obj = receiver_object(c)?;
        let propane = is_propane_bbq(&obj)?;
        let mut body = obj.try_borrow_mut().map_err(|_| borrowed("hearth"))?;
        match &mut body.kind {
            ObjectKind::Fireplace(f) => f.extinguish(),
            ObjectKind::Barbecue(b) => b.extinguish(propane),
            _ => {}
        }
        Ok(0)
    });
    class.method("fadeOut", |c| {
        let obj = receiver_object(c)?;
        let propane = is_propane_bbq(&obj)?;
        let mut body = obj.try_borrow_mut().map_err(|_| borrowed("hearth"))?;
        match &mut body.kind {
            ObjectKind::Fireplace(f) => {
                f.extinguish();
                f.smouldering = false;
            }
            ObjectKind::Barbecue(b) => {
                b.extinguish(propane);
                b.smouldering = false;
            }
            _ => {}
        }
        Ok(0)
    });
    class.method("isSmouldering", |c| {
        let obj = receiver_object(c)?;
        let body = obj.try_borrow().map_err(|_| borrowed("hearth"))?;
        let smouldering = match &body.kind {
            ObjectKind::Fireplace(f) => f.smouldering,
            ObjectKind::Barbecue(b) => b.smouldering,
            _ => false,
        };
        Ok(c.push(smouldering))
    });
    class.method("getTemperature", |c| {
        let obj = receiver_object(c)?;
        let body = obj.try_borrow().map_err(|_| borrowed("hearth"))?;
        let temp = match &body.kind {
            ObjectKind::Fireplace(f) => f.temperature(),
            ObjectKind::Barbecue(b) => b.temperature(),
            _ => 1.0,
        };
        Ok(c.push(temp as f64))
    });
}

pub(super) fn register(thread: &mut Thread) {
    let fp = expose_class(thread, ISO_FIREPLACE, Some(ISO_OBJECT));
    fp.static_fn("new", new_fireplace);
    register_common_hearth(&fp);

    let bbq = expose_class(thread, ISO_BARBECUE, Some(ISO_OBJECT));
    bbq.static_fn("new", new_barbecue);
    register_common_hearth(&bbq);

    bbq.method("setPropaneTank", |c| {
        let obj = receiver_object(c)?;
        let item = crate::bind_items::item_arg(c, 1)?;
        let (is_tank, uses) = {
            let it = item.borrow();
            (it.get_full_type() == "Base.PropaneTank", it.get_current_uses())
        };
        if is_tank {
            let mut body = obj.try_borrow_mut().map_err(|_| borrowed("IsoBarbecue"))?;
            if let ObjectKind::Barbecue(b) = &mut body.kind {
                b.set_propane_tank(uses);
            }
        }
        Ok(0)
    });

    bbq.method("removePropaneTank", |c| {
        let obj = receiver_object(c)?;
        let fuel_minutes = {
            let mut body = obj.try_borrow_mut().map_err(|_| borrowed("IsoBarbecue"))?;
            let ObjectKind::Barbecue(b) = &mut body.kind else {
                return Ok(c.push(Value::Nil));
            };
            if !b.has_propane_tank {
                return Ok(c.push(Value::Nil));
            }
            b.has_propane_tank = false;
            b.lit = false;
            let fuel = b.fuel_amount();
            b.set_fuel_amount(0);
            fuel
        };
        let Some(item_ref) = crate::bind_items::try_create_item("Base.PropaneTank", 1.0)? else {
            return Ok(c.push(Value::Nil));
        };
        {
            let mut it = item_ref
                .try_borrow_mut()
                .map_err(|_| borrowed("PropaneTank"))?;
            let max_uses = it.get_max_uses().max(0);
            let uses = ((fuel_minutes as f32 / 0.24).round() as i32).clamp(0, max_uses);
            it.set_current_uses(uses);
        }
        Ok(c.push(crate::bind_items::item_value(&item_ref)))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::stdlib::{self, load_string};
    use pz_kahlua::TableRef;
    use pz_world::{IsoCell, IsoChunk, IsoGridSquare, IsoSpriteManager};

    fn run(t: &mut Thread, code: &str) -> Vec<Value> {
        let f = load_string(code, Some("hearth_test"), t.globals().clone()).unwrap();
        t.pcall(&f, &[])
            .unwrap_or_else(|(e, trace)| panic!("{e:?}\n{trace}"))
    }

    #[test]
    fn fireplace_barbecue_and_generator_lua_lifecycle() {
        let prev_obj =
            pz_world::object_slot::install_object_observer(Some(crate::component_gc::observe_object));
        let prev_item =
            pz_inventory::shared::install_item_observer(Some(crate::component_gc::observe_item));
        let mut t = Thread::new(TableRef::new());
        stdlib::install(&mut t, None).unwrap();
        crate::bind_java_util::register(&mut t);
        crate::bind_globals::register(&mut t);
        crate::bind_misc_classes::register(&mut t, None);
        crate::bind_items::register(&mut t);
        crate::bind_world::register(&mut t);

        let sm = pz_scripts::ScriptManager::new();
        sm.enable_texture_capture();
        sm.registries
            .get("item_type")
            .unwrap()
            .borrow_mut()
            .register_id("base:drainable", true)
            .unwrap();
        sm.statics
            .items
            .borrow_mut()
            .constants
            .insert("item_type", "DRAINABLE", "base:drainable");
        sm.parse_script(
            pz_scripts::script_type::ScriptLoadMode::Init,
            r#"
            module Base {
                item PropaneTank {
                    ItemType = base:drainable,
                    DisplayName = Propane Tank,
                    UseDelta = 0.0625,
                }
            }
            "#,
        )
        .unwrap();
        sm.get_module("Base")
            .unwrap()
            .bucket(pz_scripts::script_type::ScriptType::Item)
            .load_scripts(&sm, pz_scripts::script_type::ScriptLoadMode::Init);
        crate::bind_registry::install_script_manager(Rc::new(sm));

        let sprites = Rc::new(RefCell::new(IsoSpriteManager::new(
            pz_world::sprite::test_tables(),
        )));
        crate::bind_sprites::install_sprite_manager(Some(sprites));

        let mut cell = IsoCell::default();
        let mut chunk = IsoChunk::new(1, 2);
        let mut sq = IsoGridSquare::new(10, 20, 0);
        let gen = IsoObject::new(1, ObjectKind::Generator(pz_world::objects::IsoGenerator {
            connected: false,
            activated: false,
            fuel: 5.0,
            condition: 90,
            last_hour: 0,
            total_power_using: 0.0,
        }), None);
        sq.objects.push(gen.into());
        chunk.set_square(2, 4, 0, Some(sq.into()));
        cell.chunks.insert((1, 2), chunk);
        crate::bind_world::set_cell(Some(cell));

        let out = run(
            &mut t,
            r#"
            local function assert(v, msg) if not v then error(msg or "assertion failed") end end
            local cell = getCell()
            local sq = cell:getGridSquare(10, 20, 0)

            -- IsoFireplace lifecycle
            local fp = IsoFireplace.new(cell, sq, "fixtures_fireplace_01_0")
            assert(instanceof(fp, "IsoFireplace"))
            assert(not fp:hasFuel() and not fp:isLit())
            assert(fp:getTemperature() == 1.0)
            fp:addFuel(90)
            assert(fp:getFuelAmount() == 90 and fp:hasFuel())
            assert(fp:useFuel(30) == 30 and fp:getFuelAmount() == 60)
            fp:turnOn()
            assert(fp:isLit() and fp:getTemperature() > 1.5)
            fp:extinguish()
            assert(not fp:isLit())

            -- IsoBarbecue + propane tank round-trip
            local bbq = IsoBarbecue.new(cell, sq, "appliances_cooking_01_35")
            assert(instanceof(bbq, "IsoBarbecue"))
            bbq:addFuel(40)
            bbq:toggle()
            assert(bbq:isLit())
            bbq:toggle()
            assert(not bbq:isLit())

            local tank = instanceItem("Base.PropaneTank")
            tank:setCurrentUses(10)
            bbq:setPropaneTank(tank)
            assert(bbq:getFuelAmount() == 2)
            local removed = bbq:removePropaneTank()
            assert(removed ~= nil and removed:getFullType() == "Base.PropaneTank")
            assert(bbq:getFuelAmount() == 0)

            -- IsoGenerator methods
            local g = sq:getObjects():get(0)
            assert(instanceof(g, "IsoGenerator"))
            g:setActivated(true)
            assert(not g:isActivated()) -- not connected yet
            g:setConnected(true)
            g:setActivated(true)
            assert(g:isActivated())
            g:setTotalPowerUsing(0.45)
            assert(g:getTotalPowerUsing() > 0.4)
            assert(not g:isTooCloseToOtherGenerators())
            assert(g:getItemsPowered():size() == 0)
            IsoGenerator.updateGenerator(sq)
            g:failToStart()
            assert(not g:isActivated())
            g:remove()
            assert(sq:getObjects():size() == 0)
            IsoGenerator.Reset()

            return fp:getFuelAmount(), removed:getCurrentUses()
            "#,
        );
        assert_eq!(out[0].as_num(), Some(60.0));
        assert_eq!(out[1].as_num(), Some(8.0));

        crate::bind_world::set_cell(None);
        crate::bind_sprites::install_sprite_manager(None);
        pz_world::object_slot::install_object_observer(prev_obj);
        pz_inventory::shared::install_item_observer(prev_item);
    }
}
