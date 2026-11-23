//! SoundManager's positioned single-player sounds and the returned FMODAudio.
use super::*;
use pz_sound::world_emitters::EmitterRef;

const FMOD_AUDIO: &str = "fmod.fmod.FMODAudio";
struct WorldAudio(EmitterRef);
impl LuaObject for WorldAudio {
    fn class_name(&self) -> &str {
        FMOD_AUDIO
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn emitter(c: &NativeCall<'_>) -> LuaResult<EmitterRef> {
    Ok(down::<WorldAudio>(&obj::<WorldAudio>(c, 0)?).0.clone())
}

fn stop(emitter: EmitterRef) {
    sound_system().borrow_mut().with_world_emitters(|_, ctx| emitter.borrow_mut().stop_all(ctx));
}

pub(super) fn register_audio(thread: &mut Thread) {
    let b = expose_class(thread, FMOD_AUDIO, None);
    b.method("isPlaying", |c| {
        let playing = !emitter(c)?.borrow().is_empty();
        Ok(c.push(playing))
    });
    b.method("setVolume", |c| {
        emitter(c)?
            .borrow_mut()
            .set_volume_all(num_arg(c, 1) as f32);
        Ok(0)
    });
    b.method("stop", |c| {
        stop(emitter(c)?);
        Ok(0)
    });
    b.method("getName", |c| {
        emitter(c)?;
        Ok(c.push(Value::Nil))
    });
    for name in ["start", "pause", "setName"] {
        b.method(name, |c| {
            emitter(c)?;
            Ok(0)
        });
    }
}

pub(super) fn register_manager(b: &crate::expose::ClassBuilder) {
    b.method("StopSound", |c| {
        sm_this(c)?;
        stop(down::<WorldAudio>(&obj::<WorldAudio>(c, 1)?).0.clone());
        Ok(0)
    });
    b.method("PlaySound", |c| {
        sm_this(c)?;
        // The four-parameter overload returns null.
        if c.nargs() != 4 || sound_system().borrow().world.is_server() {
            return Ok(c.push(Value::Nil));
        }
        let name = name_arg(c, 1)?;
        let (emitter, handle) = sound_system()
            .borrow_mut()
            .play_world_clip(&name, [0.0; 3], false);
        Ok(push_obj(
            c,
            (handle != 0).then(|| Rc::new(WorldAudio(emitter))),
        ))
    });
    for method in [
        "PlayWorldSound",
        "PlayWorldSoundWav",
        "PlayWorldSoundWavImpl",
        "PlayWorldSoundImpl",
    ] {
        b.method(method, move |c| {
            sm_this(c)?;
            let mut name = name_arg(c, 1)?.to_string();
            let wav = matches!(method, "PlayWorldSoundWav" | "PlayWorldSoundWavImpl");
            let choices = method == "PlayWorldSoundWav"
                && c.nargs() == 8
                && matches!(c.arg(6), Value::Num(_));
            if sound_system().borrow().sounds.env.sound_disabled {
                return if choices { Ok(0) } else { Ok(c.push(Value::Nil)) };
            }
            if choices {
                let n = crate::expose::int_arg(c, 6);
                let choice = (sound_system().borrow_mut().rand_next)(n) + 1;
                name.push_str(&choice.to_string());
            }
            let pos = if method == "PlayWorldSoundImpl" {
                (
                    crate::expose::int_arg(c, 3),
                    crate::expose::int_arg(c, 4),
                    crate::expose::int_arg(c, 5),
                )
            } else {
                let i = if matches!(c.arg(2), Value::Bool(_)) {
                    3
                } else {
                    2
                };
                let Some(pos) = crate::bind_world::square_coords(&c.arg(i)) else {
                    if method == "PlayWorldSoundWavImpl" { return Err(npe("IsoGridSquare.getX()")); }
                    return if choices {
                        Ok(0)
                    } else {
                        Ok(c.push(Value::Nil))
                    };
                };
                if sound_system().borrow().world.is_server() && method != "PlayWorldSoundWavImpl" {
                    return if choices {
                        Ok(0)
                    } else {
                        Ok(c.push(Value::Nil))
                    };
                }
                pos
            };
            let (emitter, _) = sound_system().borrow_mut().play_world_clip(
                &name,
                [pos.0 as f32 + 0.5, pos.1 as f32 + 0.5, pos.2 as f32],
                !wav,
            );
            if choices {
                Ok(0)
            } else {
                Ok(push_obj(c, Some(Rc::new(WorldAudio(emitter)))))
            }
        });
    }
}
