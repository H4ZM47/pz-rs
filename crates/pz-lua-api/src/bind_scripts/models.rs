//! `ModelScript` and `ModelAttachment`, `MannequinScript`, `SpriteModel`,
//! `SoundTimelineScript`, `TimedActionScript`, `FluidDefinitionScript`,
//! `EnergyDefinitionScript`, `StringListScript`, `ItemConfig`,
//! `ItemFilterScript`, `FluidFilterScript`, and the value objects they
//! return: live `org.joml.Vector3f`/`Vector2f` views (`x`, `y`, `z`, `set`,
//! `length`) and `zombie.core.Color` copies (`getR`..`getA`).
//!
//! Not exposed yet: texture getters (`getIconTexture`, bar textures;
//! TODO(Texture)), `TimedActionScript.applyMuscleStrain`
//! (TODO(IsoGameCharacter)), `SpriteModel.parseRuntimeString` (tile
//! definitions), `ItemConfig`'s entity configuration (`BuildBuckets`,
//! `ConfigureEntity*`), filters' item and fluid tests and
//! `FluidFilterScript.getFilter`/`copy`/`createFilter` (TODO(inventory,
//! FluidFilter)), `GameEntityTemplate`/`VehicleTemplate.getScript`, the Xui
//! script classes (TODO(XuiManager)), the rest of the joml vector API.

use super::{
    get, get_obj, method, opt_str, req_str, set, string_list, void, IntoLua, ScriptHandle,
};
use crate::bind_registry::java_error;
use crate::expose::{
    bool_arg, enum_value, expose_class, expose_enum, int_arg, num_arg, str_arg, this, ClassBuilder,
};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use pz_scripts::math::{Vector2, Vector3};
use pz_scripts::objects::definitions::{
    ActionSoundTime, Color, EnergyDefinitionScript, EnergyType, FluidCategory,
    FluidDefinitionScript, FluidFilterScript, FluidType, PoisonEffect, TimedActionScript,
};
use pz_scripts::objects::entity::item_config::ItemConfig;
use pz_scripts::objects::items::ItemFilterScript;
use pz_scripts::objects::models::{
    MannequinScript, ModelAttachment, ModelScript, SoundTimelineScript, SpriteModel,
};
use pz_scripts::objects::StringListScript;
use pz_scripts::ScriptManager;
use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub const MODEL_ATTACHMENT_CLASS: &str = "zombie.scripting.objects.ModelAttachment";
pub const VECTOR3F_CLASS: &str = "org.joml.Vector3f";
pub const VECTOR2F_CLASS: &str = "org.joml.Vector2f";
pub const COLOR_CLASS: &str = "zombie.core.Color";
pub const ACTION_SOUND_TIME_CLASS: &str = "zombie.scripting.objects.ActionSoundTime";
pub const METABOLICS_CLASS: &str = "zombie.characters.BodyDamage.Metabolics";
pub const FLUID_TYPE_CLASS: &str = "zombie.entity.components.fluids.FluidType";
pub const FLUID_CATEGORY_CLASS: &str = "zombie.entity.components.fluids.FluidCategory";
pub const POISON_EFFECT_CLASS: &str = "zombie.entity.components.fluids.PoisonEffect";
pub const ENERGY_TYPE_CLASS: &str = "zombie.entity.energy.EnergyType";
pub const FLUID_FILTER_CLASS: &str = "zombie.scripting.objects.FluidFilterScript";

type AttachmentRef = Rc<RefCell<ModelAttachment>>;

thread_local! {
    static ATTACHMENTS: RefCell<HashMap<usize, Value>> = RefCell::new(HashMap::new());
}

// ---- vectors and colours ----

type VecGet = Box<dyn Fn() -> LuaResult<[f32; 3]>>;
type VecSet = Box<dyn Fn([f32; 3]) -> LuaResult<()>>;

/// A live view of a vector field (`org.joml.Vector3f`/`Vector2f`).
pub struct VecView {
    dims: usize,
    get: VecGet,
    set: Option<VecSet>,
}

impl LuaObject for VecView {
    fn class_name(&self) -> &str {
        if self.dims == 3 {
            VECTOR3F_CLASS
        } else {
            VECTOR2F_CLASS
        }
    }
    fn to_lua_string(&self) -> String {
        match (self.get)() {
            Ok(v) if self.dims == 3 => format!("({} {} {})", v[0], v[1], v[2]),
            Ok(v) => format!("({} {})", v[0], v[1]),
            Err(_) => String::new(),
        }
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A `Vector3f` field of `T` inside a script.
pub fn vec3<T: 'static>(
    h: &ScriptHandle,
    get: fn(&T) -> Vector3,
    set: Option<fn(&mut T, Vector3)>,
) -> Value {
    let (hg, hs) = (h.clone(), h.clone());
    Value::Object(Rc::new(VecView {
        dims: 3,
        get: Box::new(move || hg.read(|t: &T| get(t)).map(|v| [v.x, v.y, v.z])),
        set: set.map(|s| -> VecSet {
            Box::new(move |v| {
                hs.write(|t: &mut T| {
                    s(
                        t,
                        Vector3 {
                            x: v[0],
                            y: v[1],
                            z: v[2],
                        },
                    )
                })
            })
        }),
    }))
}

/// A `Vector2f` field of `T` inside a script (read-only).
pub fn vec2<T: 'static>(h: &ScriptHandle, get: fn(&T) -> Vector2) -> Value {
    let hg = h.clone();
    Value::Object(Rc::new(VecView {
        dims: 2,
        get: Box::new(move || hg.read(|t: &T| get(t)).map(|v| [v.x, v.y, 0.0])),
        set: None,
    }))
}

/// A `zombie.core.Color` (the getters return copies).
pub struct ColorObject(pub Color);

impl LuaObject for ColorObject {
    fn class_name(&self) -> &str {
        COLOR_CLASS
    }
    fn to_lua_string(&self) -> String {
        let c = &self.0;
        format!("Color ({},{},{},{})", c.r, c.g, c.b, c.a)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub fn color_value(c: Color) -> Value {
    Value::Object(Rc::new(ColorObject(c)))
}

fn view<R>(c: &NativeCall<'_>, f: impl FnOnce(&VecView) -> LuaResult<R>) -> LuaResult<R> {
    let o = this::<VecView>(c)?;
    f(o.as_any().downcast_ref::<VecView>().unwrap())
}

fn register_values(thread: &mut Thread) {
    for (class, dims) in [(VECTOR3F_CLASS, 3), (VECTOR2F_CLASS, 2)] {
        if !thread.class_metatables.get_str(class).is_nil() {
            continue;
        }
        let b = expose_class(thread, class, None);
        for (i, name) in ["x", "y", "z"].into_iter().enumerate().take(dims) {
            b.method(name, move |c| {
                let v = view(c, |v| (v.get)())?;
                Ok(c.push(v[i] as f64))
            });
        }
        b.method("length", |c| {
            let v = view(c, |v| (v.get)())?;
            Ok(c.push(((v[0] * v[0] + v[1] * v[1] + v[2] * v[2]) as f64).sqrt()))
        });
        b.method("set", |c| {
            let other = match c.arg(1) {
                Value::Object(o) => o.as_any().downcast_ref::<VecView>().map(|v| (v.get)()),
                _ => None,
            };
            let new = match other {
                Some(v) => v?,
                None => [
                    num_arg(c, 1) as f32,
                    num_arg(c, 2) as f32,
                    num_arg(c, 3) as f32,
                ],
            };
            view(c, |v| match &v.set {
                Some(s) => s(new),
                None => Err(LuaError::runtime("vector is read-only here")),
            })?;
            let this = c.arg(0);
            Ok(c.push(this))
        });
        b.method("toString", |c| {
            let s = view(c, |v| Ok(v.to_lua_string()))?;
            Ok(c.push(s))
        });
    }
    if thread.class_metatables.get_str(COLOR_CLASS).is_nil() {
        let b = expose_class(thread, COLOR_CLASS, None);
        type Channel = fn(&Color) -> f32;
        let channels: [(&str, Channel); 8] = [
            ("getR", |c| c.r),
            ("getG", |c| c.g),
            ("getB", |c| c.b),
            ("getA", |c| c.a),
            ("getRedFloat", |c| c.r),
            ("getGreenFloat", |c| c.g),
            ("getBlueFloat", |c| c.b),
            ("getAlphaFloat", |c| c.a),
        ];
        for (name, f) in channels {
            b.method(name, move |c| {
                let o = this::<ColorObject>(c)?;
                let v = f(&o.as_any().downcast_ref::<ColorObject>().unwrap().0);
                Ok(c.push(v as f64))
            });
        }
    }
}

/// Expose a Java enum unless another binding already did.
fn enum_once(
    thread: &mut Thread,
    class: &'static str,
    names: Vec<&'static str>,
) -> Option<ClassBuilder> {
    if !thread.class_metatables.get_str(class).is_nil() {
        return None;
    }
    Some(expose_enum(thread, class, &names))
}

fn action_sound_time_name(t: ActionSoundTime) -> &'static str {
    match t {
        ActionSoundTime::ActionStart => "ACTION_START",
        ActionSoundTime::AnimationEvent => "ANIMATION_EVENT",
        ActionSoundTime::AnimationStart => "ANIMATION_START",
    }
}

fn register_enums(thread: &mut Thread, sm: &Rc<ScriptManager>) {
    let names = ActionSoundTime::ALL
        .iter()
        .map(|t| action_sound_time_name(*t))
        .collect();
    if let Some(b) = enum_once(thread, ACTION_SOUND_TIME_CLASS, names) {
        b.method("toString", |c| {
            let id = match c.arg(0) {
                Value::Object(o) => ActionSoundTime::ALL
                    .iter()
                    .find(|t| action_sound_time_name(**t) == o.to_lua_string())
                    .map(|t| t.id()),
                _ => None,
            };
            Ok(c.push(id.into_lua()))
        });
    }
    // The constants are game data read from the jar.
    let mets: Vec<&'static str> = sm
        .builtin_tables
        .borrow()
        .metabolics
        .iter()
        .map(|(n, _)| &*Box::leak(n.clone().into_boxed_str()))
        .collect();
    enum_once(thread, METABOLICS_CLASS, mets);
    enum_once(
        thread,
        FLUID_TYPE_CLASS,
        FluidType::ALL.iter().map(|m| m.name()).collect(),
    );
    enum_once(
        thread,
        FLUID_CATEGORY_CLASS,
        FluidCategory::ALL.iter().map(|m| m.name()).collect(),
    );
    enum_once(
        thread,
        POISON_EFFECT_CLASS,
        PoisonEffect::ALL.iter().map(|m| m.name()).collect(),
    );
    enum_once(
        thread,
        ENERGY_TYPE_CLASS,
        EnergyType::ALL.iter().map(|m| m.name()).collect(),
    );
}

// ---- ModelAttachment ----

/// A `ModelAttachment` a `ModelScript` holds (or one Lua created), with the
/// owner `setOwner` gave it.
pub struct AttachmentObject {
    pub attach: AttachmentRef,
    owner: RefCell<Option<ScriptHandle>>,
}

impl LuaObject for AttachmentObject {
    fn class_name(&self) -> &str {
        MODEL_ATTACHMENT_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn attachment_value(a: Option<&AttachmentRef>, owner: Option<&ScriptHandle>) -> Value {
    let Some(a) = a else { return Value::Nil };
    ATTACHMENTS.with(|m| {
        m.borrow_mut()
            .entry(Rc::as_ptr(a) as usize)
            .or_insert_with(|| {
                Value::Object(Rc::new(AttachmentObject {
                    attach: a.clone(),
                    owner: RefCell::new(owner.cloned()),
                }))
            })
            .clone()
    })
}

fn with_attachment<R>(v: &Value, f: impl FnOnce(&AttachmentObject) -> R) -> Option<R> {
    match v {
        Value::Object(o) => o.as_any().downcast_ref::<AttachmentObject>().map(f),
        _ => None,
    }
}

pub(super) fn attachment_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<AttachmentRef> {
    with_attachment(&c.arg(i), |a| a.attach.clone()).ok_or_else(super::npe)
}

/// Run `f` on `self`: a shared attachment, or one inside a vehicle script.
fn read_attach<R>(c: &NativeCall<'_>, f: impl FnOnce(&ModelAttachment) -> R) -> LuaResult<R> {
    if let Some(a) = with_attachment(&c.arg(0), |a| a.attach.clone()) {
        return Ok(f(&a.borrow()));
    }
    super::this_handle(c)?.read(f)
}

fn write_attach<R>(c: &NativeCall<'_>, f: impl FnOnce(&mut ModelAttachment) -> R) -> LuaResult<R> {
    if let Some(a) = with_attachment(&c.arg(0), |a| a.attach.clone()) {
        return Ok(f(&mut a.borrow_mut()));
    }
    super::this_handle(c)?.write(f)
}

/// `getOffset` (`rotate` false) or `getRotate` of `self`.
fn attach_vec(c: &NativeCall<'_>, rotate: bool) -> LuaResult<Value> {
    if let Some(a) = with_attachment(&c.arg(0), |a| a.attach.clone()) {
        let (ag, as_) = (a.clone(), a);
        return Ok(Value::Object(Rc::new(VecView {
            dims: 3,
            get: Box::new(move || {
                let a = ag.borrow();
                let v = if rotate { *a.rotate() } else { *a.offset() };
                Ok([v.x, v.y, v.z])
            }),
            set: Some(Box::new(move |v| {
                let mut a = as_.borrow_mut();
                let t = if rotate {
                    a.rotate_mut()
                } else {
                    a.offset_mut()
                };
                *t = Vector3 {
                    x: v[0],
                    y: v[1],
                    z: v[2],
                };
                Ok(())
            })),
        })));
    }
    let h = super::this_handle(c)?;
    Ok(if rotate {
        vec3(
            &h,
            |a: &ModelAttachment| *a.rotate(),
            Some(|a: &mut ModelAttachment, v| *a.rotate_mut() = v),
        )
    } else {
        vec3(
            &h,
            |a: &ModelAttachment| *a.offset(),
            Some(|a: &mut ModelAttachment, v| *a.offset_mut() = v),
        )
    })
}

fn register_attachment(thread: &mut Thread) {
    let b = expose_class(thread, MODEL_ATTACHMENT_CLASS, None);
    b.ctor(|c| {
        let id = str_arg(c, 0).map(|s| s.to_string()).unwrap_or_default();
        let a = ModelAttachment::new(&id).map_err(java_error)?;
        Ok(c.push(attachment_value(Some(&Rc::new(RefCell::new(a))), None)))
    });
    macro_rules! attach_get {
        ($name:literal, |$a:ident| $e:expr) => {
            b.method($name, |c| {
                let v = read_attach(c, |$a| $e.into_lua())?;
                Ok(c.push(v))
            });
        };
    }
    attach_get!("getId", |a| a.id().to_string());
    attach_get!("getBone", |a| a.bone().map(str::to_string));
    attach_get!("getScale", |a| a.scale());
    attach_get!("getZOffset", |a| a.zoffset());
    attach_get!("isUpdateConstraint", |a| a.is_update_constraint());
    attach_get!("getCanAttach", |a| a.can_attach().map(string_list));
    b.method("getOffset", |c| {
        let v = attach_vec(c, false)?;
        Ok(c.push(v))
    });
    b.method("getRotate", |c| {
        let v = attach_vec(c, true)?;
        Ok(c.push(v))
    });
    b.method("setBone", |c| {
        let bone = str_arg(c, 1).ok_or_else(super::npe)?;
        write_attach(c, |a| a.set_bone(&bone))?;
        Ok(0)
    });
    b.method("setScale", |c| {
        let s = num_arg(c, 1) as f32;
        write_attach(c, |a| a.set_scale(s))?;
        Ok(0)
    });
    b.method("setZOffset", |c| {
        let s = num_arg(c, 1) as f32;
        write_attach(c, |a| a.set_zoffset(s))?;
        Ok(0)
    });
    b.method("setUpdateConstraint", |c| {
        let v = bool_arg(c, 1);
        write_attach(c, |a| a.set_update_constraint(v))?;
        Ok(0)
    });
    b.method("setCanAttach", |c| {
        let list = match c.arg(1) {
            Value::Nil => None,
            _ => Some(
                super::list_arg(c, 1)
                    .iter()
                    .map(pz_kahlua::exec::java_value_of)
                    .collect(),
            ),
        };
        write_attach(c, |a| a.set_can_attach(list))?;
        Ok(0)
    });
    b.method("setOwner", |c| {
        let owner = super::handle_arg(c, 1).ok().flatten();
        with_attachment(&c.arg(0), |a| *a.owner.borrow_mut() = owner);
        Ok(0)
    });
    b.method("setId", |c| {
        let id = opt_str(c, 1).unwrap_or_default();
        let shared = with_attachment(&c.arg(0), |a| (a.attach.clone(), a.owner.borrow().clone()));
        match shared {
            Some((a, Some(owner))) => {
                owner.write(|m: &mut ModelScript| m.rename_attachment(&a, &id))?
            }
            Some((a, None)) => a.borrow_mut().set_id(&id),
            None => write_attach(c, |a| a.set_id(&id))?,
        }
        .map_err(java_error)?;
        Ok(0)
    });
    b.method("equals", |c| {
        let r = c.arg(0).raw_equals(&c.arg(1));
        Ok(c.push(r))
    });
}

fn set_owner(a: &AttachmentRef, owner: Option<&ScriptHandle>) {
    with_attachment(&attachment_value(Some(a), owner), |o| {
        *o.owner.borrow_mut() = owner.cloned()
    });
}

fn index_error(i: i32, n: usize) -> LuaError {
    LuaError::Runtime {
        message: format!("Index {i} out of bounds for length {n}"),
        class: "java.lang.IndexOutOfBoundsException",
    }
}

fn register_model_script(thread: &mut Thread) {
    let b = expose_class(
        thread,
        "zombie.scripting.objects.ModelScript",
        Some(super::BASE_SCRIPT_OBJECT_CLASS),
    );
    get(&b, "getName", |m: &ModelScript| {
        m.name().map(str::to_string)
    });
    method(&b, "getFullType", |_, h| {
        h.read(|m: &ModelScript| {
            m.base
                .module_name()
                .map(|module| format!("{module}.{}", m.name().unwrap_or("null")))
        })?
        .ok_or_else(super::npe)
        .map(IntoLua::into_lua)
    });
    get(&b, "getFileName", |m: &ModelScript| {
        m.file_name().map(str::to_string)
    });
    get(&b, "getMeshName", |m: &ModelScript| {
        m.mesh_name().map(str::to_string)
    });
    get(&b, "getShaderName", |m: &ModelScript| {
        m.shader_name().to_string()
    });
    get(&b, "isStatic", |m: &ModelScript| m.is_static());
    get(&b, "getModelManagerKey", |m: &ModelScript| {
        m.model_manager_key().map(str::to_string)
    });
    set(
        &b,
        "setModelManagerKey",
        |m: &mut ModelScript, v: Option<String>| m.set_model_manager_key(v),
    );
    method(&b, "getTextureName", |c, h| {
        let allow_null = c.nargs() > 1 && bool_arg(c, 1);
        h.read(|m: &ModelScript| {
            m.texture_name_or_null(allow_null)
                .map(str::to_string)
                .into_lua()
        })
    });
    get(&b, "getAttachmentCount", |m: &ModelScript| {
        m.attachment_count()
    });
    method(&b, "getAttachment", |c, h| {
        let i = int_arg(c, 1);
        let (a, n) = h.read(|m: &ModelScript| {
            (
                usize::try_from(i).ok().and_then(|i| m.attachment(i)),
                m.attachment_count(),
            )
        })?;
        match a {
            Some(a) => Ok(attachment_value(Some(&a), Some(h))),
            None => Err(index_error(i, n)),
        }
    });
    method(&b, "getAttachmentById", |c, h| {
        let id = match c.arg(1) {
            Value::Nil => return Ok(Value::Nil),
            Value::Object(o) => o.to_lua_string(),
            _ => opt_str(c, 1).unwrap_or_default(),
        };
        let a = h.read(|m: &ModelScript| m.attachment_by_id(&id))?;
        Ok(attachment_value(a.as_ref(), Some(h)))
    });
    method(&b, "addAttachment", |c, h| {
        let a = attachment_arg(c, 1)?;
        h.write(|m: &mut ModelScript| m.add_attachment(a.clone()))?;
        set_owner(&a, Some(h));
        Ok(attachment_value(Some(&a), Some(h)))
    });
    method(&b, "addAttachmentAt", |c, h| {
        let i = int_arg(c, 1);
        let a = attachment_arg(c, 2)?;
        let n = h.read(|m: &ModelScript| m.attachment_count())?;
        if i < 0 || i as usize > n {
            return Err(LuaError::Runtime {
                message: format!("Index: {i}, Size: {n}"),
                class: "java.lang.IndexOutOfBoundsException",
            });
        }
        h.write(|m: &mut ModelScript| m.add_attachment_at(i as usize, a.clone()))?;
        set_owner(&a, Some(h));
        Ok(attachment_value(Some(&a), Some(h)))
    });
    method(&b, "removeAttachment", |c, h| {
        let a = match c.arg(1) {
            Value::Num(_) => {
                let i = int_arg(c, 1);
                let n = h.read(|m: &ModelScript| m.attachment_count())?;
                if i < 0 || i as usize >= n {
                    return Err(index_error(i, n));
                }
                h.write(|m: &mut ModelScript| m.remove_attachment_at(i as usize))?
            }
            _ => {
                let a = attachment_arg(c, 1)?;
                h.write(|m: &mut ModelScript| m.remove_attachment(&a))?
            }
        };
        set_owner(&a, None);
        Ok(attachment_value(Some(&a), None))
    });
    void(&b, "scaleAttachmentOffset", |c, h| {
        let s = num_arg(c, 1) as f32;
        h.write(|m: &mut ModelScript| m.scale_attachment_offset(s))
    });
    void(&b, "beforeRenameAttachment", |c, h| {
        let id = attachment_arg(c, 1)?.borrow().id().to_string();
        h.write(|m: &mut ModelScript| {
            m.attachment_by_id.remove(&id);
        })
    });
    void(&b, "afterRenameAttachment", |c, h| {
        let a = attachment_arg(c, 1)?;
        let id = a.borrow().id().to_string();
        h.write(|m: &mut ModelScript| {
            m.attachment_by_id.insert(id, a.clone());
        })
    });
}

pub fn register(thread: &mut Thread, sm: &Rc<ScriptManager>) {
    register_values(thread);
    register_enums(thread, sm);
    register_attachment(thread);
    register_model_script(thread);

    let b = expose_class(
        thread,
        "zombie.scripting.objects.MannequinScript",
        Some(super::BASE_SCRIPT_OBJECT_CLASS),
    );
    get(&b, "getName", |m: &MannequinScript| {
        m.name().map(str::to_string)
    });
    get(&b, "isFemale", |m: &MannequinScript| m.is_female());
    set(&b, "setFemale", |m: &mut MannequinScript, v: bool| {
        m.set_female(v)
    });
    type Text = (
        &'static str,
        &'static str,
        fn(&MannequinScript) -> Option<&str>,
        fn(&mut MannequinScript, &str),
    );
    let texts: [Text; 6] = [
        (
            "getModelScriptName",
            "setModelScriptName",
            MannequinScript::model_script_name,
            MannequinScript::set_model_script_name,
        ),
        (
            "getTexture",
            "setTexture",
            MannequinScript::texture,
            MannequinScript::set_texture,
        ),
        (
            "getAnimSet",
            "setAnimSet",
            MannequinScript::anim_set,
            MannequinScript::set_anim_set,
        ),
        (
            "getAnimState",
            "setAnimState",
            MannequinScript::anim_state,
            MannequinScript::set_anim_state,
        ),
        (
            "getPose",
            "setPose",
            MannequinScript::pose,
            MannequinScript::set_pose,
        ),
        (
            "getOutfit",
            "setOutfit",
            MannequinScript::outfit,
            MannequinScript::set_outfit,
        ),
    ];
    for (g, s, fg, fs) in texts {
        get(&b, g, move |m: &MannequinScript| fg(m).map(str::to_string));
        void(&b, s, move |c, h| {
            let v = opt_str(c, 1).unwrap_or_default();
            h.write(|m: &mut MannequinScript| fs(m, &v))
        });
    }

    let b = expose_class(
        thread,
        "zombie.iso.SpriteModel",
        Some(super::BASE_SCRIPT_OBJECT_CLASS),
    );
    get(&b, "getModelScriptName", |m: &SpriteModel| {
        m.model_script_name().map(str::to_string)
    });
    set(
        &b,
        "setModelScriptName",
        |m: &mut SpriteModel, v: Option<String>| m.set_model_script_name(v),
    );
    get(&b, "getTextureName", |m: &SpriteModel| {
        m.texture_name().map(str::to_string)
    });
    set(
        &b,
        "setTextureName",
        |m: &mut SpriteModel, v: Option<String>| m.set_texture_name(v.as_deref().unwrap_or("")),
    );
    get(&b, "getAnimationName", |m: &SpriteModel| {
        m.animation_name().map(str::to_string)
    });
    set(
        &b,
        "setAnimationName",
        |m: &mut SpriteModel, v: Option<String>| m.animation_name = v,
    );
    get(&b, "getAnimationTime", |m: &SpriteModel| m.animation_time);
    set(&b, "setAnimationTime", |m: &mut SpriteModel, v: f32| {
        m.animation_time = v
    });
    get(&b, "getRuntimeString", |m: &SpriteModel| {
        m.runtime_string().map(str::to_string)
    });
    set(
        &b,
        "setRuntimeString",
        |m: &mut SpriteModel, v: Option<String>| m.set_runtime_string(v.as_deref().unwrap_or("")),
    );
    get(&b, "getScale", |m: &SpriteModel| m.scale);
    set(&b, "setScale", |m: &mut SpriteModel, v: f32| m.scale = v);
    get_obj(&b, "getTranslate", |h, _: &SpriteModel| {
        vec3(
            h,
            |m: &SpriteModel| m.translate,
            Some(|m: &mut SpriteModel, v| m.translate = v),
        )
    });
    get_obj(&b, "getRotate", |h, _: &SpriteModel| {
        vec3(
            h,
            |m: &SpriteModel| m.rotate,
            Some(|m: &mut SpriteModel, v| m.rotate = v),
        )
    });
    method(&b, "set", |c, h| {
        let other = super::handle_arg(c, 1)?.ok_or_else(super::npe)?;
        if !other.root.ptr_eq(&h.root) {
            let copy = other.read(|o: &SpriteModel| {
                let mut s = SpriteModel::new();
                s.set(o);
                s
            })?;
            h.write(|m: &mut SpriteModel| {
                m.set(&copy);
            })?;
        }
        Ok(c.arg(0))
    });

    let b = expose_class(
        thread,
        "zombie.scripting.objects.SoundTimelineScript",
        Some(super::BASE_SCRIPT_OBJECT_CLASS),
    );
    get(&b, "getEventName", |s: &SoundTimelineScript| {
        s.event_name().map(str::to_string)
    });
    method(&b, "getPosition", |c, h| {
        let id = opt_str(c, 1).unwrap_or_default();
        h.read(|s: &SoundTimelineScript| s.position(&id).into_lua())
    });

    let b = expose_class(
        thread,
        "zombie.scripting.objects.TimedActionScript",
        Some(super::BASE_SCRIPT_OBJECT_CLASS),
    );
    get(&b, "getName", |t: &TimedActionScript| t.name().to_string());
    method(&b, "getFullType", |_, h| {
        h.read(|t: &TimedActionScript| t.full_type().map_err(java_error))?
            .map(IntoLua::into_lua)
    });
    get(&b, "getTime", |t: &TimedActionScript| t.time());
    get(&b, "isFaceObject", |t: &TimedActionScript| {
        t.is_face_object()
    });
    get(&b, "isCantSit", |t: &TimedActionScript| t.is_cant_sit());
    get(&b, "getProp1", |t: &TimedActionScript| {
        t.prop1().map(str::to_string)
    });
    get(&b, "getProp2", |t: &TimedActionScript| {
        t.prop2().map(str::to_string)
    });
    get(&b, "getActionAnim", |t: &TimedActionScript| {
        t.action_anim().map(str::to_string)
    });
    get(&b, "getAnimVarKey", |t: &TimedActionScript| {
        t.anim_var_key().map(str::to_string)
    });
    get(&b, "getAnimVarVal", |t: &TimedActionScript| {
        t.anim_var_val().map(str::to_string)
    });
    get(&b, "getSound", |t: &TimedActionScript| {
        t.sound().map(str::to_string)
    });
    get(&b, "getCompletionSound", |t: &TimedActionScript| {
        t.completion_sound().map(str::to_string)
    });
    get(&b, "hasMuscleStrain", |t: &TimedActionScript| {
        t.has_muscle_strain()
    });
    get(&b, "getSoundTime", |t: &TimedActionScript| {
        enum_value(
            ACTION_SOUND_TIME_CLASS,
            action_sound_time_name(t.sound_time()),
        )
    });
    get(&b, "getMetabolics", |t: &TimedActionScript| {
        enum_value(METABOLICS_CLASS, t.metabolics())
    });

    let b = expose_class(
        thread,
        "zombie.scripting.objects.FluidDefinitionScript",
        Some(super::BASE_SCRIPT_OBJECT_CLASS),
    );
    type FluidF32 = fn(&FluidDefinitionScript) -> f32;
    let floats: [(&str, FluidF32); 15] = [
        ("getStressChange", FluidDefinitionScript::stress_change),
        ("getThirstChange", FluidDefinitionScript::thirst_change),
        ("getHungerChange", FluidDefinitionScript::hunger_change),
        (
            "getEnduranceChange",
            FluidDefinitionScript::endurance_change,
        ),
        ("getFatigueChange", FluidDefinitionScript::fatigue_change),
        ("getCalories", FluidDefinitionScript::calories),
        ("getCarbohydrates", FluidDefinitionScript::carbohydrates),
        ("getLipids", FluidDefinitionScript::lipids),
        ("getProteins", FluidDefinitionScript::proteins),
        ("getPainReduction", FluidDefinitionScript::pain_reduction),
        ("getFluReduction", FluidDefinitionScript::flu_reduction),
        ("getUnhappyChange", FluidDefinitionScript::unhappy_change),
        ("getAlcohol", FluidDefinitionScript::alcohol),
        (
            "getPoisonMinAmount",
            FluidDefinitionScript::poison_min_amount,
        ),
        (
            "getPoisonDiluteRatio",
            FluidDefinitionScript::poison_dilute_ratio,
        ),
    ];
    for (name, f) in floats {
        get(&b, name, f);
    }
    get(&b, "getFoodSicknessChange", |f: &FluidDefinitionScript| {
        f.food_sickness_change()
    });
    get(&b, "getFluidType", |f: &FluidDefinitionScript| {
        enum_value(FLUID_TYPE_CLASS, f.fluid_type().name())
    });
    get(&b, "getFluidTypeString", |f: &FluidDefinitionScript| {
        f.fluid_type_string().map(str::to_string)
    });
    get(&b, "getModID", |f: &FluidDefinitionScript| {
        f.mod_id().map(str::to_string)
    });
    get(&b, "isVanilla", |f: &FluidDefinitionScript| f.is_vanilla());
    get(&b, "getExistsAsVanilla", |f: &FluidDefinitionScript| {
        f.exists_as_vanilla()
    });
    get(&b, "getDisplayName", |f: &FluidDefinitionScript| {
        f.display_name().to_string()
    });
    get(&b, "getColor", |f: &FluidDefinitionScript| {
        color_value(f.color())
    });
    get(&b, "hasPropertiesSet", |f: &FluidDefinitionScript| {
        f.has_properties_set()
    });
    get(&b, "getPoisonMaxEffect", |f: &FluidDefinitionScript| {
        enum_value(POISON_EFFECT_CLASS, f.poison_max_effect().name())
    });
    get(&b, "getCategories", |f: &FluidDefinitionScript| {
        let list = f
            .categories()
            .iter()
            .map(|c| enum_value(FLUID_CATEGORY_CLASS, c.name()))
            .collect();
        crate::bind_java_util::list_value(list)
    });
    get_obj(
        &b,
        "getBlendWhitelist",
        |h, f: &FluidDefinitionScript| match f.blend_whitelist() {
            Some(_) => h.child(
                FLUID_FILTER_CLASS,
                "blendWhitelist",
                |f: &FluidDefinitionScript| f.blend_whitelist(),
            ),
            None => Value::Nil,
        },
    );
    get_obj(
        &b,
        "getBlendBlackList",
        |h, f: &FluidDefinitionScript| match f.blend_blacklist() {
            Some(_) => h.child(
                FLUID_FILTER_CLASS,
                "blendBlacklist",
                |f: &FluidDefinitionScript| f.blend_blacklist(),
            ),
            None => Value::Nil,
        },
    );

    let b = expose_class(
        thread,
        "zombie.scripting.objects.EnergyDefinitionScript",
        Some(super::BASE_SCRIPT_OBJECT_CLASS),
    );
    get(&b, "getModID", |e: &EnergyDefinitionScript| {
        e.mod_id().map(str::to_string)
    });
    get(&b, "isVanilla", |e: &EnergyDefinitionScript| e.is_vanilla());
    get(&b, "getExistsAsVanilla", |e: &EnergyDefinitionScript| {
        e.exists_as_vanilla()
    });
    get(&b, "getEnergyType", |e: &EnergyDefinitionScript| {
        enum_value(ENERGY_TYPE_CLASS, e.energy_type().name())
    });
    get(&b, "getEnergyTypeString", |e: &EnergyDefinitionScript| {
        e.energy_type_string().map(str::to_string)
    });
    get(&b, "getDisplayName", |e: &EnergyDefinitionScript| {
        e.display_name().map(str::to_string)
    });
    get(&b, "getColor", |e: &EnergyDefinitionScript| {
        color_value(e.color())
    });

    let b = expose_class(
        thread,
        "zombie.scripting.objects.StringListScript",
        Some(super::BASE_SCRIPT_OBJECT_CLASS),
    );
    get(&b, "getValues", |s: &StringListScript| {
        string_list(s.values())
    });

    let b = expose_class(
        thread,
        "zombie.scripting.itemConfig.ItemConfig",
        Some(super::BASE_SCRIPT_OBJECT_CLASS),
    );
    get(&b, "getName", |i: &ItemConfig| i.name().map(str::to_string));
    get(&b, "isValid", |i: &ItemConfig| i.is_valid());

    let b = expose_class(
        thread,
        "zombie.scripting.objects.ItemFilterScript",
        Some(super::BASE_SCRIPT_OBJECT_CLASS),
    );
    get(&b, "getName", |i: &ItemFilterScript| {
        i.name().map(str::to_string)
    });

    let b = expose_class(
        thread,
        FLUID_FILTER_CLASS,
        Some(super::BASE_SCRIPT_OBJECT_CLASS),
    );
    get(&b, "isSingleFluid", |f: &FluidFilterScript| {
        f.is_single_fluid()
    });
    void(&b, "LoadAnonymousSingleFluid", |c, h| {
        let name = req_str(c, 1)?;
        h.write(|f: &mut FluidFilterScript| f.load_anonymous_single_fluid(&name))?
            .map_err(java_error)
    });
}
