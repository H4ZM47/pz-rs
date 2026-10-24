//! `zombie.ui`: `UIElement`, `UIManager` and the Java side of the Lua UI.
//!
//! Lua UI classes (`ISUIElement` and subclasses in `media/lua/client/ISUI`)
//! wrap a Java `UIElement` created with `UIElement.new(luaTable)`; the Java
//! element calls back into its Lua table (`prerender`, `render`,
//! `onMouseDown`, ...). This crate owns that tree and its input dispatch.

/// Index into [`UiTree::elements`].
pub type ElementId = usize;

#[derive(Default, Debug, Clone)]
pub struct UIElement {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub visible: bool,
    pub parent: Option<ElementId>,
    pub children: Vec<ElementId>,
    pub always_on_top: bool,
    pub back_most: bool,
}

/// `UIManager.UI`: top-level elements in draw order.
#[derive(Default)]
pub struct UiTree {
    pub elements: Vec<UIElement>,
    pub roots: Vec<ElementId>,
}
