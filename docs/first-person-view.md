# First-person view (port extension)

The port can draw the world in 3D from the player's eyes. This is not part of
the original game. It is off by default, and with it off every frame is the
original isometric one.

## Controls

| Input | Action |
|---|---|
| O | First person on or off |
| Shift+O | Third person (from isometric), or swap first and third person |
| Mouse | Look (the cursor is captured, a crosshair shows) |
| Clicks while looking | Act on what the crosshair is on (right click opens its context menu) |
| Middle mouse | Free the cursor for the UI, press again to capture it |
| WASD | Move relative to where you look |

## How it works

- `pz-world3d` builds the scene. Floors, north and west walls, doors and
  windows become quads textured from their own tile sprites. The isometric
  view is an affine projection, so projecting each 3D corner into the sprite
  frame gives exact texture coordinates. Stairs are ramps rising a third of a
  level per piece. Roofs are planes whose corner heights are fitted, at load,
  from each roof sprite's depth map (tile data has no roof slope). Other
  objects (furniture, trees) are vertical cards on the square's diagonal.
- The first-person eye is kept 0.22 units off walls, so standing against
  one never puts the near plane inside it.
- Picking: under mouse-look, a ray through the crosshair hits the nearest
  face; the hit's isometric screen position stands in for the mouse
  (`pz_input::world_mouse`), so the existing pickers and context menus act on
  it. With the cursor freed for the UI, the mouse is the real cursor.
- Lighting is the game's own per-square vertex light, so night, lamps and
  interiors match the isometric view. The weather screen pass still runs.
- Characters and vehicles are the existing 3D models drawn with a
  perspective `ModelCamera` against the scene's depth buffer.
- `pz-gpu` has a `Scene3DBegin` / `Quad3D` / `Scene3DEnd` pass. The model and
  vehicle shaders divide depth by `w`, which is identical for the isometric
  projection (`w == 1`).
- Player input: `pz_player_input::set_view_yaw` makes movement, facing and
  aiming follow the view.

## Not done yet

Picking with a freed cursor, item and corpse drawing, furniture as more than
cards, shadows, per-chunk mesh caching, split screen
(player 0 only).
