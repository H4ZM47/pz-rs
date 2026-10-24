//! `zombie.ui`: `UIElement`, `UIManager` and the Java side of the Lua UI.
//!
//! Lua UI classes (`ISUIElement` and subclasses in `media/lua/client/ISUI`)
//! wrap a Java `UIElement` created with `UIElement.new(luaTable)`; the Java
//! element calls back into its Lua table (`prerender`, `render`,
//! `onMouseDown`, ...). This crate owns that tree and its input dispatch.
//!
//! Layout:
//! * [`element`]: `UIElement` (tree, geometry, event dispatch into Lua);
//! * [`draw`]: the `UIElement.Draw*` family and stencil clipping;
//! * [`manager`]: `UIManager` statics (`UI` list, per-frame update/render);
//! * [`sprite`]: the UI draw list (`SpriteRenderer` equivalent);
//! * [`text`]: the `TextManager` interface the engine implements;
//! * [`texture`]: Lua-facing `Texture` / `Color` wrappers.
//!
//! Java statics live in a thread-local [`UIManager`] (Lua is single
//! threaded); nothing holds a `RefCell` borrow while calling into Lua, so
//! callbacks may freely re-enter (`AddUI`, `bringToTop`, `setX`, ...).

pub mod draw;
pub mod element;
pub mod lua;
pub mod manager;
pub mod sprite;
pub mod text;
pub mod texture;

pub use element::UIElement;
pub use manager::{with_ui, MouseState, UIManager};
pub use sprite::{SpriteRenderer, TextAlign, UiDrawCmd};
pub use text::TextRenderer;
pub use texture::ColorObject;
