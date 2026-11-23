//! `IsoObject`'s outline highlight fields as a character carries them:
//! `isOutlineHighlight`, `isOutlineHlAttached`, `isOutlineHlBlink` (one bit
//! per player) and `outlineHighlightCol` (an ABGR colour per player). The
//! renderer turns them into the character's model outline.

/// `IsoPlayer.MAX`.
const PLAYERS: usize = 4;

/// `Color.colorToABGR`.
pub fn color_to_abgr(r: f32, g: f32, b: f32, a: f32) -> u32 {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0) as i32 as u32 & 0xFF;
    byte(a) << 24 | byte(b) << 16 | byte(g) << 8 | byte(r)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutlineState {
    pub highlight: u8,
    pub attached: u8,
    pub blink: u8,
    pub col: [u32; PLAYERS],
    /// `FBORenderObjectOutline`'s `duringUiRenderTime` and
    /// `duringUiUpdateTime` for this object: the `UIManager` render and
    /// update times when the highlight was set while the UI rendered or
    /// updated (0 otherwise). Such a highlight survives `renderlast` in
    /// that same UI frame.
    pub ui_times: [UiTimes; PLAYERS],
}

/// When the highlight was set during the UI: `UIManager.uiRenderTimeMS`
/// while `isRendering()`, `uiUpdateTimeMS` while `isUpdating()`, else 0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UiTimes {
    pub render: i64,
    pub update: i64,
}

impl Default for OutlineState {
    fn default() -> Self {
        // The constructor fills `outlineHighlightCol` with -1 (white).
        OutlineState {
            highlight: 0,
            attached: 0,
            blink: 0,
            col: [u32::MAX; PLAYERS],
            ui_times: [UiTimes::default(); PLAYERS],
        }
    }
}

/// `(byte)(1 << playerIndex)`: Java masks the shift to 5 bits and the
/// byte cast drops bits above 7.
fn bit(player: usize) -> u8 {
    (1u32 << (player & 31)) as u8
}

fn set_bit(bits: &mut u8, player: usize, on: bool) {
    if on {
        *bits |= bit(player);
    } else {
        *bits &= !bit(player);
    }
}

impl OutlineState {
    /// `isOutlineHighlight()`: any player.
    pub fn is_any(&self) -> bool {
        self.highlight != 0
    }

    /// `isOutlineHighlight(playerIndex)`. `render_container_highlight` is
    /// `DebugOptions.terrain.renderTiles.renderContainerHighlight` (on by
    /// default).
    pub fn is_on(&self, player: usize, render_container_highlight: bool) -> bool {
        render_container_highlight && self.highlight & bit(player) != 0
    }

    /// `setOutlineHighlight(boolean)`: every player (`(byte)-1` or 0).
    /// `ui` is when it was set during the UI (zeros outside it; turning
    /// the highlight off clears the times).
    pub fn set_all(&mut self, on: bool, ui: UiTimes) {
        self.highlight = if on { 0xFF } else { 0 };
        self.ui_times = [if on { ui } else { UiTimes::default() }; PLAYERS];
    }

    /// `setOutlineHighlight(playerIndex, boolean)`.
    pub fn set(&mut self, player: usize, on: bool, ui: UiTimes) {
        set_bit(&mut self.highlight, player, on);
        if let Some(t) = self.ui_times.get_mut(player) {
            *t = if on { ui } else { UiTimes::default() };
        }
    }

    /// `IsoMovingObject.renderlast()` for `player`: a highlight set during
    /// this UI frame's render or update (`now`) stays, any other one is
    /// turned off after the object has drawn.
    pub fn render_last(&mut self, player: usize, render_container_highlight: bool, now: UiTimes) {
        if !self.is_on(player, render_container_highlight) {
            return;
        }
        let t = self.ui_times.get(player).copied().unwrap_or_default();
        if (t.render != 0 && t.render == now.render) || (t.update != 0 && t.update == now.update) {
            return;
        }
        self.set(player, false, UiTimes::default());
    }

    pub fn is_attached_any(&self) -> bool {
        self.attached != 0
    }

    pub fn is_attached(&self, player: usize) -> bool {
        self.attached & bit(player) != 0
    }

    pub fn set_attached_all(&mut self, on: bool) {
        self.attached = if on { 0xFF } else { 0 };
    }

    pub fn set_attached(&mut self, player: usize, on: bool) {
        set_bit(&mut self.attached, player, on);
    }

    pub fn is_blink_any(&self) -> bool {
        self.blink != 0
    }

    pub fn is_blink(&self, player: usize) -> bool {
        self.blink & bit(player) != 0
    }

    pub fn set_blink_all(&mut self, on: bool) {
        self.blink = if on { 0xFF } else { 0 };
    }

    pub fn set_blink(&mut self, player: usize, on: bool) {
        set_bit(&mut self.blink, player, on);
    }

    /// `setOutlineHighlightCol(r, g, b, a)`: every player.
    pub fn set_col_all(&mut self, r: f32, g: f32, b: f32, a: f32) {
        self.col = [color_to_abgr(r, g, b, a); PLAYERS];
    }

    /// `setOutlineHighlightCol(playerIndex, r, g, b, a)`.
    pub fn set_col(&mut self, player: usize, r: f32, g: f32, b: f32, a: f32) {
        self.col[player] = color_to_abgr(r, g, b, a);
    }

    /// `getOutlineHighlightCol(playerIndex)` (an int in the original).
    pub fn col(&self, player: usize) -> i32 {
        self.col[player] as i32
    }

    /// `unsetOutlineHighlight()`: clears the highlight, blink and attached
    /// bits (the colours stay).
    pub fn unset(&mut self) {
        self.highlight = 0;
        self.blink = 0;
        self.attached = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_and_colours() {
        let mut o = OutlineState::default();
        assert_eq!(o.col(2), -1);
        o.set(1, true, UiTimes::default());
        assert!(o.is_any() && o.is_on(1, true) && !o.is_on(0, true));
        assert!(!o.is_on(1, false));
        o.set_all(true, UiTimes::default());
        assert_eq!(o.highlight, 0xFF);
        o.set(3, false, UiTimes::default());
        assert_eq!(o.highlight, 0xF7);
        o.set_col(2, 1.0, 0.0, 0.0, 0.5);
        assert_eq!(o.col[2], 0x7F00_00FF);
        o.set_col_all(0.0, 1.0, 1.0, 0.65);
        assert_eq!(o.col, [0xA5FF_FF00; 4]);
        o.set_blink(0, true);
        o.set_attached_all(true);
        o.unset();
        assert_eq!((o.highlight, o.blink, o.attached), (0, 0, 0));
        assert_eq!(o.col[0], 0xA5FF_FF00);
    }

    #[test]
    fn render_last_keeps_highlights_set_during_this_ui_frame() {
        let mut o = OutlineState::default();
        // Set outside the UI (combat): cleared after the frame.
        o.set(0, true, UiTimes::default());
        o.render_last(0, true, UiTimes { render: 500, update: 400 });
        assert!(!o.is_on(0, true));
        // Set while the UI rendered at 500: kept in that UI frame only.
        o.set(0, true, UiTimes { render: 500, update: 0 });
        o.render_last(0, true, UiTimes { render: 500, update: 400 });
        assert!(o.is_on(0, true));
        o.render_last(0, true, UiTimes { render: 516, update: 400 });
        assert!(!o.is_on(0, true));
        // Set during the UI update at 400 survives while it is current.
        o.set_all(true, UiTimes { render: 0, update: 400 });
        o.render_last(2, true, UiTimes { render: 516, update: 400 });
        assert!(o.is_on(2, true));
        // Other players' bits are untouched.
        o.render_last(1, true, UiTimes { render: 516, update: 500 });
        assert!(!o.is_on(1, true) && o.is_on(2, true));
    }
}
