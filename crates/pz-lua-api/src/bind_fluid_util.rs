//! `zombie.entity.components.fluids.FluidUtil` static helpers used by Build
//! 42.22 fluid actions and transfer UIs (`ISFluidUtil.lua`, `ISFluidTransferUI.lua`).

use pz_kahlua::{Thread, Value};

use crate::expose::{expose_class, num_arg};

pub const FLUID_UTIL_CLASS: &str = "zombie.entity.components.fluids.FluidUtil";

const TRANSFER_ACTION_TIME_PER_LITER: f64 = 50.0;
const MIN_TRANSFER_ACTION_TIME: f64 = 20.0;
const MIN_CONTAINER_CAPACITY: f64 = 0.05;

fn trim_decimal(v: f64, places: usize) -> String {
    let s = format!("{v:.places$}");
    if places == 0 {
        return s;
    }
    let trimmed = s.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() || trimmed == "-0" {
        "0".into()
    } else {
        trimmed.into()
    }
}

/// `FluidUtil.getAmountFormatted(amount)`.
pub fn format_amount(amount: f64) -> String {
    let liter = crate::bind_text::translate("IGUI_Liter");
    let ml = crate::bind_text::translate("IGUI_Milliliter");
    let liter = if liter == "IGUI_Liter" { "L".into() } else { liter };
    let ml = if ml == "IGUI_Milliliter" { "mL".into() } else { ml };
    if amount >= 1000.0 {
        format!("{}{liter}", trim_decimal(amount, 0))
    } else if amount >= 10.0 {
        format!("{}{liter}", trim_decimal(amount, 1))
    } else if amount >= 1.0 {
        format!("{}{liter}", trim_decimal(amount, 2))
    } else {
        format!("{}{ml}", (amount * 1000.0).round() as i64)
    }
}

/// `FluidUtil.getFractionFormatted(numerator, denominator)`.
pub fn format_fraction(numerator: f64, denominator: f64) -> String {
    let max = numerator.max(denominator);
    let liter = crate::bind_text::translate("IGUI_Liter");
    let ml = crate::bind_text::translate("IGUI_Milliliter");
    let liter = if liter == "IGUI_Liter" { "L".into() } else { liter };
    let ml = if ml == "IGUI_Milliliter" { "mL".into() } else { ml };
    if max >= 1000.0 {
        format!(
            "{} / {}{liter}",
            trim_decimal(numerator, 0),
            trim_decimal(denominator, 0)
        )
    } else if max >= 10.0 {
        format!(
            "{} / {}{liter}",
            trim_decimal(numerator, 1),
            trim_decimal(denominator, 1)
        )
    } else if max >= 1.0 {
        format!(
            "{} / {}{liter}",
            trim_decimal(numerator, 2),
            trim_decimal(denominator, 2)
        )
    } else {
        format!(
            "{} / {}{ml}",
            (numerator * 1000.0).round() as i64,
            (denominator * 1000.0).round() as i64
        )
    }
}

/// `FluidUtil.roundTransfer(amount)`: rounds to two decimal places (0.01 L).
pub fn round_transfer(amount: f64) -> f64 {
    (amount * 100.0).round() / 100.0
}

/// Expose `zombie.entity.components.fluids.FluidUtil`.
pub fn register(thread: &mut Thread) {
    let b = expose_class(thread, FLUID_UTIL_CLASS, Some("java.lang.Object"));
    b.static_fn("getTransferActionTimePerLiter", |c| {
        Ok(c.push(TRANSFER_ACTION_TIME_PER_LITER))
    });
    b.static_fn("getMinTransferActionTime", |c| {
        Ok(c.push(MIN_TRANSFER_ACTION_TIME))
    });
    b.static_fn("getMinContainerCapacity", |c| {
        Ok(c.push(MIN_CONTAINER_CAPACITY))
    });
    b.static_fn("getAmountFormatted", |c| {
        let s = format_amount(num_arg(c, 0));
        Ok(c.push(Value::str(&s)))
    });
    b.static_fn("getFractionFormatted", |c| {
        let s = format_fraction(num_arg(c, 0), num_arg(c, 1));
        Ok(c.push(Value::str(&s)))
    });
    b.static_fn("roundTransfer", |c| Ok(c.push(round_transfer(num_arg(c, 0)))));
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::stdlib::load_string;
    use pz_kahlua::TableRef;

    #[test]
    fn fluid_util_formatting_and_timing_from_lua() {
        let mut t = Thread::new(TableRef::new());
        register(&mut t);
        let f = load_string(
            r#"
            return FluidUtil.getTransferActionTimePerLiter(),
                   FluidUtil.getMinTransferActionTime(),
                   FluidUtil.getAmountFormatted(0.5),
                   FluidUtil.getAmountFormatted(1.25),
                   FluidUtil.roundTransfer(1.236)
            "#,
            Some("fluid_util_test.lua"),
            t.globals().clone(),
        )
        .unwrap();
        let res = t.pcall(&f, &[]).unwrap();
        assert_eq!(res[0].as_num(), Some(50.0));
        assert_eq!(res[1].as_num(), Some(20.0));
        assert_eq!(res[2].as_str(), Some("500mL"));
        assert_eq!(res[3].as_str(), Some("1.25L"));
        assert!((res[4].as_num().unwrap() - 1.24).abs() < 1e-6);
    }
}
