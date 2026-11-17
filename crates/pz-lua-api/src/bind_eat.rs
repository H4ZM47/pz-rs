//! `IsoGameCharacter.Eat(item, percentage, useUtensil)` and the character's
//! sound emitter (`getEmitter`, `playSound`, `stopOrTriggerSound`) that
//! `ISEatFoodAction` plays its eating sound on.
//!
//! `Eat` splits the original in two: the character's stat, nutrition and
//! sickness changes are [`GameCharacter::eat_food`]; this file then calls
//! the food's `OnEat` function and uses the food up (or scales its values by
//! what is left), as the original does after `JustAteFood`.
//!
//! The emitter is a pooled world emitter per character, moved to the
//! character's square when it plays. Nothing ticks character emitters every
//! frame yet, so each call here ticks the emitter itself; a voice therefore
//! starts when the sound is played and its state is read on the next call.

use crate::bind_character_sim::{character_value, chr_of, with_ctx, LuaGameCharacterRef, ISO_GAME_CHARACTER_CLASS};
use crate::bind_characters::shared_characters;
use crate::bind_items::{item_arg, item_env, ItemOutcome};
use crate::expose::{bool_arg, expose_class, num_arg, str_arg, with_this};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaResult, NativeCall, Thread, Value};
use pz_sound::world_emitters::EmitterRef;
use std::any::Any;
use std::cell::RefCell;
use std::rc::{Rc, Weak};

const BASE_EMITTER_CLASS: &str = "zombie.characters.BaseCharacterSoundEmitter";
const EMITTER_CLASS: &str = "zombie.characters.CharacterSoundEmitter";

/// A character's sound emitter.
struct CharacterEmitterObject(LuaGameCharacterRef);

impl LuaObject for CharacterEmitterObject {
    fn class_name(&self) -> &str {
        EMITTER_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

thread_local! {
    /// The pooled emitter of each character that asked for one.
    static EMITTERS: RefCell<Vec<(Weak<RefCell<pz_characters::character::GameCharacter<crate::bind_characters::LuaItem>>>, EmitterRef)>> =
        const { RefCell::new(Vec::new()) };
}

/// The character's emitter, taken from the world pool on first use.
fn emitter_of(chr: &LuaGameCharacterRef) -> EmitterRef {
    EMITTERS.with(|all| {
        let mut all = all.borrow_mut();
        all.retain(|(c, _)| c.strong_count() > 0);
        if let Some((_, e)) = all.iter().find(|(c, _)| std::ptr::eq(c.as_ptr(), Rc::as_ptr(chr))) {
            return e.clone();
        }
        let e = crate::bind_audio::sound_system().borrow_mut().with_world_emitters(|pool, _| {
            let e = pool.get_free();
            pool.take_ownership(&e);
            e
        });
        all.push((Rc::downgrade(chr), e.clone()));
        e
    })
}

fn emitter_chr(c: &NativeCall<'_>) -> LuaResult<LuaGameCharacterRef> {
    with_this::<CharacterEmitterObject, _>(c, |o| o.0.clone())
}

/// `f` on the character's emitter, ticked afterwards.
fn with_emitter<R>(
    chr: &LuaGameCharacterRef,
    f: impl FnOnce(&mut pz_sound::emitter::FmodSoundEmitter, &mut pz_sound::emitter::Ctx) -> R,
) -> R {
    let emitter = emitter_of(chr);
    crate::bind_audio::sound_system().borrow_mut().with_world_emitters(|_, ctx| {
        let mut e = emitter.borrow_mut();
        let r = f(&mut e, ctx);
        e.tick(ctx);
        r
    })
}

/// `getEmitter().playSound(name)`: a handle, 0 when the sound is unknown.
fn play(chr: &LuaGameCharacterRef, name: &str) -> i64 {
    let (x, y, z) = {
        let c = chr.borrow();
        (c.vars.x, c.vars.y, c.vars.z)
    };
    with_emitter(chr, |e, ctx| {
        e.set_pos(x, y, z);
        e.play_sound(name, ctx)
    })
}

fn handle_arg(c: &NativeCall<'_>, i: usize) -> i64 {
    num_arg(c, i) as i64
}

pub fn register_shared(thread: &mut Thread) {
    let chars = shared_characters();
    let sm = crate::bind_registry::shared_script_manager();
    let b = expose_class(thread, ISO_GAME_CHARACTER_CLASS, None);
    b.method("getEmitter", |c| {
        let chr = chr_of(c)?;
        let o: Rc<dyn LuaObject> = Rc::new(CharacterEmitterObject(chr));
        Ok(c.push(Value::Object(o)))
    });
    b.method("playSound", |c| {
        let chr = chr_of(c)?;
        let name = str_arg(c, 1).unwrap_or_else(|| Rc::from(""));
        let h = play(&chr, &name);
        Ok(c.push(h as f64))
    });
    b.method("stopOrTriggerSound", |c| {
        let chr = chr_of(c)?;
        let h = handle_arg(c, 1);
        with_emitter(&chr, |e, ctx| e.stop_or_trigger_sound(h, ctx));
        Ok(0)
    });
    expose_class(thread, BASE_EMITTER_CLASS, None);
    let e = expose_class(thread, EMITTER_CLASS, Some(BASE_EMITTER_CLASS));
    e.method("playSound", |c| {
        let chr = emitter_chr(c)?;
        let name = str_arg(c, 1).unwrap_or_else(|| Rc::from(""));
        let h = play(&chr, &name);
        Ok(c.push(h as f64))
    });
    e.method("isPlaying", |c| {
        let chr = emitter_chr(c)?;
        let v = match c.arg(1) {
            Value::Str(name) => with_emitter(&chr, |e, _| e.is_playing_name(&name)),
            _ => {
                let h = handle_arg(c, 1);
                with_emitter(&chr, |e, _| e.is_playing(h))
            }
        };
        Ok(c.push(v))
    });
    e.method("stopOrTriggerSound", |c| {
        let chr = emitter_chr(c)?;
        let h = handle_arg(c, 1);
        with_emitter(&chr, |e, ctx| e.stop_or_trigger_sound(h, ctx));
        Ok(0)
    });
    e.method("stopAll", |c| {
        let chr = emitter_chr(c)?;
        with_emitter(&chr, |e, ctx| e.stop_all(ctx));
        Ok(0)
    });

    b.method("Eat", move |c| {
        let chr = chr_of(c)?;
        let item = item_arg(c, 1)?;
        let percentage = if c.nargs() > 2 { num_arg(c, 2) as f32 } else { 1.0 };
        let utensil = c.nargs() > 3 && bool_arg(c, 3);
        let v = character_value(&chr);
        // Everything the character changes, with no Lua running.
        let pct = {
            let chars = chars.borrow();
            let food = item.borrow();
            with_ctx(c, &v, |ctx| chr.borrow_mut().eat_food(&sm, &chars, ctx, &food, percentage, utensil))?
        };
        let Some(pct) = pct else {
            return Ok(c.push(false));
        };
        // `LuaManager.caller.pcallvoid(onEat, food, chr, (double) pct)`.
        let on_eat = item.borrow().get_on_eat().map(str::to_owned);
        if let Some(name) = on_eat {
            let f = crate::bind_item_container::function_object(c.thread, &name);
            if !matches!(f, Value::Nil) {
                let item_value = c.arg(1);
                crate::global_objects::pcall_logged(c.thread, &f, &[item_value, v.clone(), Value::Num(f64::from(pct))]);
            }
        }
        let e = item_env();
        let used_up = pct == 1.0 || {
            let (hung, thirst) = {
                let i = item.borrow();
                (i.get_hung_change(), i.get_thirst_change())
            };
            item.borrow_mut().multiply_food_values(1.0 - pct);
            let i = item.borrow();
            hung == 0.0 && thirst < 0.0 && i.get_thirst_change() > -0.01
        };
        if used_up {
            item.borrow_mut().set_hung_change(0.0);
            // `UseAndSync()`: the character applies what the use asks of
            // its inventory (the used-up food leaves it).
            let id = item.borrow().id;
            let used = with_ctx(c, &v, |ctx| Ok(chr.borrow_mut().use_held_item(&sm, ctx, id, false, false, false)))?;
            if used.is_none() {
                let owner = e.owner(&item);
                let out = item.borrow_mut().use_with(false, false, false, owner.as_ref(), e.context());
                e.outcome(&item, ItemOutcome::Use(Box::new(out)));
            }
            return Ok(c.push(true));
        }
        // A part-eaten food with a custom weight loses weight by `pct`,
        // down to the weight of the item it leaves behind.
        let i = item.borrow();
        if i.custom_weight {
            let empty = i
                .get_replace_on_use_full_type()
                .and_then(|full| e.context().script(&full).map(|s| s.actual_weight))
                .unwrap_or(0.0);
            let weight = i.get_weight_with(e.context());
            drop(i);
            let rest = weight - empty;
            item.borrow_mut().weight = ((rest - percentage.clamp(0.0, 1.0) * rest) + empty).max(0.0);
        }
        Ok(c.push(true))
    });
}
