//! `zombie.core.textures.VideoTexture`, `LuaManager.GlobalObject.getVideo`
//! and `LuaManager.videoTextures` / `releaseAllVideoTextures`.
//!
//! A video texture is a [`LuaTexture`] of class `VideoTexture` (so every
//! `Texture` method and every UI draw call accepts it) whose image the
//! video's frames replace. The natives behind it (`openVideo`,
//! `isReadyForNewFrame`, ...) are [`pz_video::VideoManager`]'s; until the
//! Bink video decoder is ported every `openVideo` fails, and the callers
//! see what the original does when a video cannot be opened (`getVideo`
//! returns nil, the title screen draws its still background).

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use pz_fs::GameFs;
use pz_kahlua::{LuaResult, NativeCall, Value};
use pz_render::lua::VIDEO_TEXTURE_CLASS;
use pz_render::{LuaTexture, Texture};

use crate::bind_render::{textures, ASSET, TEXTURE};
use crate::expose::{bool_arg, expose_class, int_arg, str_arg, this};
use crate::global_objects::debug_log;
use crate::LuaManager;

pub const VIDEO_TEXTURE: &str = VIDEO_TEXTURE_CLASS;

/// One `VideoTexture`.
pub struct VideoTexture {
    /// The Lua object (a [`LuaTexture`] of class `VideoTexture`).
    object: Value,
    texture_id: u32,
    width: i32,
    height: i32,
    /// `videoFilename`: the absolute media path.
    video_filename: String,
    bink_id: Cell<i32>,
    use_async: bool,
    /// `Texture.destroyed`.
    destroyed: Cell<bool>,
}

pub type VideoRef = Rc<VideoTexture>;

#[derive(Default)]
struct Statics {
    fs: Option<Rc<RefCell<GameFs>>>,
    /// `VideoTexture.successfullyLoaded`, by absolute path.
    successfully_loaded: HashMap<String, VideoRef>,
    /// `VideoTexture.failedToLoad`.
    failed_to_load: HashSet<String>,
    /// `LuaManager.videoTextures`, by the name Lua passed.
    lua_textures: HashMap<String, VideoRef>,
    /// Every live video texture by texture id, to find `self`.
    by_texture: HashMap<u32, VideoRef>,
    created: u32,
}

thread_local! {
    static STATICS: RefCell<Statics> = RefCell::new(Statics::default());
}

fn statics<R>(f: impl FnOnce(&mut Statics) -> R) -> R {
    STATICS.with(|s| f(&mut s.borrow_mut()))
}

fn manager<R>(f: impl FnOnce(&mut pz_video::VideoManager) -> R) -> R {
    pz_video::with_manager(f)
}

impl VideoTexture {
    /// `new VideoTexture(filename, width, height, useAsync)`.
    fn new(filename: &str, width: i32, height: i32, use_async: bool) -> VideoRef {
        let n = statics(|s| {
            s.created += 1;
            s.created
        });
        let tex = textures().borrow_mut().video_texture(&format!("@video@/{n}/{filename}"), width, height);
        let texture_id = tex.id.expect("video texture has an image");
        let object = Value::Object(Rc::new(LuaTexture::with_class(tex, VIDEO_TEXTURE)));
        let v = Rc::new(VideoTexture {
            object,
            texture_id,
            width,
            height,
            video_filename: filename.to_string(),
            bink_id: Cell::new(-1),
            use_async,
            destroyed: Cell::new(false),
        });
        statics(|s| s.by_texture.insert(texture_id, v.clone()));
        v
    }

    /// `VideoTexture.getOrCreate(filename, width, height, useAsync)`.
    pub fn get_or_create(filename: &str, width: i32, height: i32, use_async: bool) -> Option<VideoRef> {
        let abs_path = statics(|s| s.fs.as_ref().map(|fs| fs.borrow().media_file(&format!("videos/{filename}"))))
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("videos/{filename}"));
        let cached = statics(|s| {
            if s.failed_to_load.contains(&abs_path) {
                return Some(None);
            }
            let v = s.successfully_loaded.get(&abs_path)?.clone();
            if v.destroyed.get() {
                s.successfully_loaded.remove(&abs_path);
            }
            Some(Some(v))
        });
        if let Some(v) = cached {
            return v;
        }
        let v = VideoTexture::new(&abs_path, width, height, use_async);
        if v.load_video_file() {
            statics(|s| s.successfully_loaded.insert(abs_path, v.clone()));
            return Some(v);
        }
        debug_log(&format!("Unable to load video texture {abs_path}."));
        statics(|s| s.failed_to_load.insert(abs_path));
        // RenderThread.queueInvokeOnRenderContext(videoTexture::destroy).
        v.destroy();
        None
    }

    /// The Lua object.
    pub fn value(&self) -> Value {
        self.object.clone()
    }

    /// The texture to draw.
    pub fn texture(&self) -> Texture {
        match &self.object {
            Value::Object(o) => o.as_any().downcast_ref::<LuaTexture>().map(LuaTexture::get).unwrap(),
            _ => unreachable!(),
        }
    }

    /// `Texture.destroy()`.
    fn destroy(&self) {
        if self.destroyed.replace(true) {
            return;
        }
        textures().borrow_mut().destroy_data(self.texture_id);
        statics(|s| s.by_texture.remove(&self.texture_id));
    }

    /// `closeAndDestroy()`.
    pub fn close_and_destroy(&self) {
        statics(|s| {
            s.successfully_loaded.remove(&self.video_filename);
            s.failed_to_load.remove(&self.video_filename);
        });
        self.close();
        if self.destroyed.get() {
            return;
        }
        self.destroy();
    }

    /// `LoadVideoFile()`.
    pub fn load_video_file(&self) -> bool {
        if self.bink_id.get() > -1 {
            debug_log("VideoTexture warning - trying to load a video file which has already been loaded.");
        } else {
            let id = manager(|m| m.open_video(&self.video_filename));
            self.bink_id.set(id);
            if id > -1 && self.use_async {
                manager(|m| m.process_frame_async(id));
            }
        }
        debug_log(&format!("binkId: {}", self.bink_id.get()));
        self.bink_id.get() >= 0
    }

    /// `Close()`.
    pub fn close(&self) {
        let id = self.bink_id.get();
        if id > -1 {
            manager(|m| m.close_video(id));
            self.bink_id.set(-1);
        }
    }

    /// `isValid()`.
    pub fn is_valid(&self) -> bool {
        self.bink_id.get() >= 0
    }

    /// The render-thread upload: `glTexImage2D` of `getWidth()` x
    /// `getHeight()` RGBA pixels read from the start of the frame buffer.
    fn upload(&self) {
        let id = self.bink_id.get();
        let len = (self.width.max(0) as usize) * (self.height.max(0) as usize) * 4;
        let Some(mut px) = manager(|m| m.current_frame_data(id).map(|d| d[..len.min(d.len())].to_vec())) else {
            return;
        };
        px.resize(len, 0);
        textures().borrow_mut().tex_image(self.texture_id, self.width, self.height, px);
    }

    /// `RenderFrameAsync()`.
    fn render_frame_async(&self) {
        let id = self.bink_id.get();
        if !manager(|m| m.is_ready_for_new_frame(id)) {
            return;
        }
        if manager(|m| m.process_frame_async_wait(id, 1000)) {
            while manager(|m| m.should_skip_frame(id)) {
                manager(|m| {
                    m.next_frame(id);
                    m.process_frame_async(id);
                    m.process_frame_async_wait(id, -1);
                });
            }
            // `isEndOfVideo` is asked and ignored.
            manager(|m| {
                m.is_end_of_video(id);
                m.next_frame(id);
                m.process_frame_async(id);
            });
        }
        self.upload();
    }

    /// `RenderFrame()`: advances the video when its next frame is due and
    /// uploads the frame into the texture.
    pub fn render_frame(&self) {
        let id = self.bink_id.get();
        if id < 0 {
            return;
        }
        if self.use_async {
            self.render_frame_async();
            return;
        }
        if manager(|m| m.is_ready_for_new_frame(id)) {
            manager(|m| {
                m.process_frame(id);
                m.next_frame(id);
                while m.should_skip_frame(id) {
                    m.process_frame(id);
                    m.next_frame(id);
                }
            });
            self.upload();
        }
        manager(|m| m.is_end_of_video(id));
    }
}

/// `LuaManager.GlobalObject.getVideo(filename, width, height)`.
pub fn get_video(filename: &str, width: i32, height: i32) -> Option<VideoRef> {
    if crate::savefile::contains_double_dot(filename) {
        pz_core::debug::DebugType::Lua.warn(None, "relative paths not allowed");
        return None;
    }
    let v = VideoTexture::get_or_create(filename, width, height, true)?;
    statics(|s| s.lua_textures.insert(filename.to_string(), v.clone()));
    Some(v)
}

/// `LuaManager.releaseAllVideoTextures()` (`GameLoadingState.enter`).
pub fn release_all_video_textures() {
    let all: Vec<VideoRef> = statics(|s| s.lua_textures.values().cloned().collect());
    for v in all {
        v.close_and_destroy();
    }
    statics(|s| s.lua_textures.clear());
}

fn self_video(c: &NativeCall<'_>) -> LuaResult<VideoRef> {
    let o = this::<LuaTexture>(c)?;
    let t = o.as_any().downcast_ref::<LuaTexture>().unwrap();
    let id = t.0.borrow().id;
    id.and_then(|id| statics(|s| s.by_texture.get(&id).cloned()))
        .filter(|_| t.1 == VIDEO_TEXTURE)
        .ok_or_else(|| pz_kahlua::LuaError::runtime("Expected a VideoTexture"))
}

fn opt(v: Option<VideoRef>) -> Value {
    v.map_or(Value::Nil, |v| v.value())
}

pub fn register(mgr: &mut LuaManager) {
    statics(|s| s.fs = Some(mgr.fs.clone()));
    let thread = &mut mgr.thread;
    let texture = expose_class(thread, TEXTURE, Some(ASSET));
    let b = expose_class(thread, VIDEO_TEXTURE, Some(TEXTURE));
    // Kahlua exposes the inherited static methods on the subclass too.
    for (k, v) in texture.statics.entries() {
        if matches!(v, Value::Native(_)) && b.statics.get_own(&k).is_nil() {
            b.statics.rawset(k, v);
        }
    }
    b.static_fn("getOrCreate", |c| {
        let name = str_arg(c, 0).map_or_else(|| "null".to_string(), |s| s.to_string());
        let use_async = if c.nargs() >= 4 { bool_arg(c, 3) } else { true };
        let v = VideoTexture::get_or_create(&name, int_arg(c, 1), int_arg(c, 2), use_async);
        Ok(c.push(opt(v)))
    });
    b.method("closeAndDestroy", |c| {
        self_video(c)?.close_and_destroy();
        Ok(0)
    });
    b.method("LoadVideoFile", |c| {
        let r = self_video(c)?.load_video_file();
        Ok(c.push(r))
    });
    b.method("Close", |c| {
        self_video(c)?.close();
        Ok(0)
    });
    b.method("RenderFrame", |c| {
        self_video(c)?.render_frame();
        Ok(0)
    });
    b.method("isValid", |c| {
        let r = self_video(c)?.is_valid();
        Ok(c.push(r))
    });
    crate::expose::global(thread, "getVideo", |c| {
        let name = str_arg(c, 0).map_or_else(|| "null".to_string(), |s| s.to_string());
        let v = get_video(&name, int_arg(c, 1), int_arg(c, 2));
        Ok(c.push(opt(v)))
    });
}

#[cfg(test)]
mod tests;
