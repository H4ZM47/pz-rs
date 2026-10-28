use super::{InventoryItemIcon, InventoryItemIconFluid};
use crate::{with_ui, UIElement, UIManager, UiDrawCmd};
use pz_kahlua::{TableRef, Thread};
use pz_render::{color_to_abgr, DrawCmd, DrawList, LuaTexture, Rgba, Texture};
use std::rc::Rc;

fn texture(name: &str, id: u32, width: u32, height: u32) -> Texture {
    let mut texture = Texture::empty(name);
    texture.id = Some(id);
    texture.width = width;
    texture.height = height;
    texture.ready = true;
    texture
}

fn cropped_texture(name: &str, id: u32) -> Texture {
    let mut texture = texture(name, id, 8, 4);
    texture.width_orig = 16;
    texture.height_orig = 8;
    texture.offset_x = 2.0;
    texture.offset_y = 3.0;
    texture.x_start = 0.25;
    texture.x_end = 0.75;
    texture.y_start = 0.125;
    texture.y_end = 0.625;
    texture
}

fn element() -> Rc<UIElement> {
    UIManager::reset();
    UIElement::new(None)
}

fn positioned_element() -> Rc<UIElement> {
    let element = element();
    element.set_x(10.75);
    element.set_y(20.5);
    let mut thread = Thread::new(TableRef::new());
    element.set_x_scroll(&mut thread, 1.25);
    element.set_y_scroll(&mut thread, -2.5);
    element
}

fn icon(texture: &Texture) -> InventoryItemIcon<'_> {
    InventoryItemIcon {
        texture: Some(texture),
        color: [0.25, 0.5, 0.75],
        color_mask: None,
        fluid_mask: None,
        fluid: None,
    }
}

fn commands() -> Vec<UiDrawCmd> {
    with_ui(|manager| manager.sprite.cmds.clone())
}

fn assert_quad(
    command: &UiDrawCmd,
    texture_id: u32,
    bounds: [f32; 4],
    rgba: Rgba,
    expected_uv: Option<[f32; 4]>,
) {
    let UiDrawCmd::Quad {
        texture,
        x,
        y,
        w,
        h,
        color,
        uv,
    } = command
    else {
        panic!("expected an actual texture quad, got {command:?}");
    };
    assert_eq!(texture.as_ref().unwrap().id, Some(texture_id));
    assert_eq!([*x, *y, *w, *h], bounds);
    assert_eq!(*color, rgba);
    assert_eq!(*uv, expected_uv);
}

#[test]
fn unmasked_icon_centers_cropped_extent_and_preserves_tint_alpha_scroll() {
    let element = positioned_element();
    let texture = cropped_texture("base", 1);
    element.draw_inventory_item_icon(&icon(&texture), 3.75, 4.25, 0.375, 24.0, 20.0);
    let commands = commands();
    assert_eq!(commands.len(), 1);
    // Unlike DrawTextureIcon, the unmasked path centers the scaled extent
    // and ignores the cropped texture's offsets.
    assert_quad(
        &commands[0],
        1,
        [21.0, 28.0, 12.0, 6.0],
        Rgba(0.25, 0.5, 0.75, 0.375),
        None,
    );
}

#[test]
fn fluid_and_color_masks_record_native_order_fit_uv_and_lowered_colors() {
    let element = positioned_element();
    let base = cropped_texture("base", 1);
    let fluid_mask = cropped_texture("fluid", 2);
    let color_mask = cropped_texture("color", 3);
    let mut icon = icon(&base);
    icon.color_mask = Some(&color_mask);
    icon.fluid_mask = Some(&fluid_mask);
    icon.fluid = Some(InventoryItemIconFluid {
        color: [0.125, 0.25, 0.5],
        amount: 1.0,
        capacity: 2.0,
    });
    element.draw_inventory_item_icon(&icon, 3.75, 4.25, 0.375, 24.0, 20.0);
    let commands = commands();
    assert_eq!(commands.len(), 3);
    assert_quad(
        &commands[0],
        1,
        [18.0, 25.0, 12.0, 6.0],
        Rgba(1.0, 1.0, 1.0, 0.375),
        None,
    );
    assert_quad(
        &commands[1],
        2,
        [18.0, 28.0, 12.0, 3.0],
        Rgba(0.125, 0.25, 0.5, 0.375),
        Some([0.0, 0.5, 1.0, 1.0]),
    );
    assert_quad(
        &commands[2],
        3,
        [18.0, 25.0, 12.0, 6.0],
        Rgba(0.25, 0.5, 0.75, 0.375),
        Some([0.0, 0.0, 1.0, 1.0]),
    );
    let mut draw_list = DrawList::default();
    with_ui(|manager| manager.sprite.lower_into(&mut draw_list));
    let sprites: Vec<_> = draw_list
        .cmds
        .iter()
        .map(|command| match command {
            DrawCmd::Sprite(sprite) => sprite,
            other => panic!("expected a lowered sprite, got {other:?}"),
        })
        .collect();
    assert_eq!(sprites.len(), 3);
    assert_eq!(sprites[1].u, [0.25, 0.75, 0.75, 0.25]);
    assert_eq!(sprites[1].v, [0.375, 0.375, 0.625, 0.625]);
    assert_eq!(
        sprites[1].color,
        [color_to_abgr(0.125, 0.25, 0.5, 0.375); 4]
    );
    assert_eq!(sprites[2].texture.as_ref().unwrap().id, Some(3));
}

#[test]
fn fluid_requires_both_component_and_mask_while_color_mask_is_independent() {
    let element = positioned_element();
    let base = cropped_texture("base", 1);
    let mask = cropped_texture("mask", 2);
    for component in [false, true] {
        with_ui(|manager| manager.sprite.clear());
        let mut icon = icon(&base);
        icon.color_mask = Some(&mask);
        // Exercise each absent half of the fluid branch separately.
        if component {
            icon.fluid = Some(InventoryItemIconFluid {
                color: [1.0, 0.0, 0.0],
                amount: 1.0,
                capacity: 1.0,
            });
        } else {
            icon.fluid_mask = Some(&mask);
        }
        element.draw_inventory_item_icon(&icon, 3.75, 4.25, 0.375, 24.0, 20.0);
        let commands = commands();
        assert_eq!(commands.len(), 2);
        assert_quad(
            &commands[0],
            1,
            [21.0, 28.0, 12.0, 6.0],
            Rgba(1.0, 1.0, 1.0, 0.375),
            None,
        );
        assert_quad(
            &commands[1],
            2,
            [18.0, 25.0, 12.0, 6.0],
            Rgba(0.25, 0.5, 0.75, 0.375),
            Some([0.0, 0.0, 1.0, 1.0]),
        );
    }
}

#[test]
fn invisible_or_missing_base_suppresses_all_overlay_commands() {
    let element = element();
    let texture = texture("mask", 1, 4, 20);
    let mut icon = icon(&texture);
    icon.color_mask = Some(&texture);
    icon.fluid_mask = Some(&texture);
    icon.fluid = Some(InventoryItemIconFluid {
        color: [0.0, 1.0, 0.0],
        amount: 1.0,
        capacity: 1.0,
    });
    icon.texture = None;
    element.draw_inventory_item_icon(&icon, 0.0, 0.0, 1.0, 4.0, 20.0);
    assert!(commands().is_empty());
    icon.texture = Some(&texture);
    element.set_visible(false);
    element.draw_inventory_item_icon(&icon, 0.0, 0.0, 1.0, 4.0, 20.0);
    assert!(commands().is_empty());
}

#[test]
fn fluid_fill_uses_float_division_minimum_and_native_nan_branch() {
    let element = element();
    let texture = texture("fluid", 1, 4, 20);
    for (amount, capacity, height, expected_height, expected_uv_y) in [
        (0.0, 1.0, 20.0, 3.0, 0.85),
        (1.0, 2.0, 20.0, 10.0, 0.5),
        (1.0, 1.0, 20.0, 20.0, 0.0),
        // 5f / 6f widened to double is below 5d / 6d. The destination
        // height truncates to four, not five.
        (5.0, 6.0, 6.0, 4.0, 0.2),
        // 0f / 0f is NaN: PZMath.max(0.15f, NaN) keeps NaN, so the
        // Java float/double-to-int conversions produce zero extents.
        (0.0, 0.0, 20.0, 0.0, 1.0),
    ] {
        with_ui(|manager| manager.sprite.clear());
        let mut icon = icon(&texture);
        icon.fluid_mask = Some(&texture);
        icon.fluid = Some(InventoryItemIconFluid {
            color: [1.0; 3],
            amount,
            capacity,
        });
        element.draw_inventory_item_icon(&icon, 0.0, 0.0, 1.0, 4.0, height);
        let commands = commands();
        assert_eq!(commands.len(), 2);
        let UiDrawCmd::Quad { h, uv, .. } = &commands[1] else {
            panic!("expected fluid quad");
        };
        assert_eq!(*h, expected_height, "amount={amount} capacity={capacity}");
        assert_eq!(uv.unwrap()[1], expected_uv_y);
    }
}

#[test]
fn each_draw_uses_current_caller_supplied_shared_texture_offsets() {
    let element = element();
    let shared = LuaTexture::new(texture("shared", 1, 4, 4));
    for offset in [1.0, 7.0] {
        shared.0.borrow_mut().offset_x = offset;
        let current = shared.get();
        let mut icon = icon(&current);
        icon.fluid_mask = Some(&current);
        icon.fluid = Some(InventoryItemIconFluid {
            color: [1.0; 3],
            amount: 1.0,
            capacity: 1.0,
        });
        element.draw_inventory_item_icon(&icon, 0.0, 0.0, 1.0, 8.0, 8.0);
    }
    let commands = commands();
    assert_eq!(commands.len(), 4);
    for (index, expected_x) in [(0, 2.0), (2, 14.0)] {
        assert_quad(
            &commands[index],
            1,
            [expected_x, 0.0, 8.0, 8.0],
            Rgba(0.25, 0.5, 0.75, 1.0),
            None,
        );
    }
}
