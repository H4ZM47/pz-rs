use super::*;
use crate::save::{
    io::{Reader, SaveError, Writer},
    items::AllItems,
    object_io, NoSaveHooks, SaveCtx, SaveSettings, SaveTables,
};
use std::cell::{Cell, RefCell};

thread_local! {
    static CONFIGURED: Cell<GeneratorPowerRange> = const { Cell::new(GeneratorPowerRange { radius: 5, vertical: 1 }) };
    static FAIL_RADIUS: Cell<bool> = const { Cell::new(false) };
    static READS: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
}

fn vertical() -> crate::save::io::SaveResult<i32> {
    READS.with(|reads| reads.borrow_mut().push("vertical"));
    Ok(CONFIGURED.with(|range| range.get().vertical))
}

fn radius() -> crate::save::io::SaveResult<i32> {
    READS.with(|reads| reads.borrow_mut().push("radius"));
    if FAIL_RADIUS.with(Cell::get) {
        return Err(SaveError("authored missing radius".into()));
    }
    Ok(CONFIGURED.with(|range| range.get().radius))
}

struct Restore {
    source: Option<GeneratorRangeSource>,
    range: GeneratorPowerRange,
}

impl Restore {
    fn install() -> Self {
        let range = generator_power_range();
        let source =
            install_generator_range_source(Some(GeneratorRangeSource { vertical, radius }));
        set_generator_power_range(GeneratorPowerRange::default());
        CONFIGURED.with(|range| {
            range.set(GeneratorPowerRange {
                radius: 5,
                vertical: 1,
            })
        });
        FAIL_RADIUS.with(|fail| fail.set(false));
        READS.with(|reads| reads.borrow_mut().clear());
        Self { source, range }
    }
}

impl Drop for Restore {
    fn drop(&mut self) {
        install_generator_range_source(self.source);
        set_generator_power_range(self.range);
    }
}

#[test]
fn native_generator_class_defaults_and_constructor_latch() {
    // A fresh native host thread begins with the class initializer values.
    std::thread::spawn(|| {
        assert_eq!(
            generator_power_range(),
            GeneratorPowerRange {
                radius: 20,
                vertical: 3
            }
        )
    })
    .join()
    .unwrap();
    let _restore = Restore::install();
    assert_eq!(generator_power_range(), GeneratorPowerRange::default());
    generator_constructed().unwrap();
    assert_eq!(
        generator_power_range(),
        GeneratorPowerRange {
            radius: 5,
            vertical: 1
        }
    );
    CONFIGURED.with(|range| {
        range.set(GeneratorPowerRange {
            radius: 2,
            vertical: 4,
        })
    });
    assert_eq!(
        generator_power_range(),
        GeneratorPowerRange {
            radius: 5,
            vertical: 1
        }
    );
    generator_constructed().unwrap();
    assert_eq!(
        generator_power_range(),
        GeneratorPowerRange {
            radius: 2,
            vertical: 4
        }
    );
    install_generator_range_source(None);
    assert_eq!(
        generator_power_range(),
        GeneratorPowerRange {
            radius: 2,
            vertical: 4
        }
    );
    assert!(generator_constructed()
        .unwrap_err()
        .0
        .contains("not installed"));
}

#[test]
fn constructor_failure_preserves_native_vertical_write_prefix() {
    let _restore = Restore::install();
    FAIL_RADIUS.with(|fail| fail.set(true));
    assert_eq!(
        generator_constructed().unwrap_err().0,
        "authored missing radius"
    );
    assert_eq!(
        READS.with(|reads| reads.borrow().clone()),
        ["vertical", "radius"]
    );
    assert_eq!(
        generator_power_range(),
        GeneratorPowerRange {
            radius: 20,
            vertical: 1
        }
    );
}

#[test]
fn saved_generator_latches_before_body_and_keeps_decode_failure_cursor() {
    let _restore = Restore::install();
    let sprites = IsoSpriteManager::new(crate::sprite::test_tables());
    let tables = SaveTables::from_text("0:IsoObject\n7:IsoGenerator\nbloodBodyParts=0").unwrap();
    let settings = SaveSettings::default();
    let ctx = SaveCtx {
        sprites: &sprites,
        tables: &tables,
        settings: &settings,
        items: &AllItems,
        hooks: &NoSaveHooks,
        meta: None,
    };
    let prefix = [1, 7];
    let mut input = Reader::new(&prefix);
    let error =
        object_io::load_object(&mut input, &ctx, crate::save::WORLD_VERSION, 8).unwrap_err();
    assert!(error.0.contains("BufferUnderflowException"));
    assert_eq!(input.pos, 2);
    assert_eq!(
        generator_power_range(),
        GeneratorPowerRange {
            radius: 5,
            vertical: 1
        }
    );

    CONFIGURED.with(|range| {
        range.set(GeneratorPowerRange {
            radius: 7,
            vertical: 2,
        })
    });
    FAIL_RADIUS.with(|fail| fail.set(true));
    let mut input = Reader::new(&prefix);
    assert!(
        object_io::load_object(&mut input, &ctx, crate::save::WORLD_VERSION, 8)
            .unwrap_err()
            .0
            .contains("missing radius")
    );
    assert_eq!(input.pos, 2);
    assert_eq!(
        generator_power_range(),
        GeneratorPowerRange {
            radius: 5,
            vertical: 2
        }
    );

    FAIL_RADIUS.with(|fail| fail.set(false));
    let object = IsoObject::new(
        8,
        ObjectKind::Generator(crate::objects::IsoGenerator {
            connected: true,
            activated: false,
            fuel: 2.5,
            condition: 67,
            last_hour: 9,
            total_power_using: 0.0,
        }),
        None,
    );
    let mut output = Writer::new();
    object_io::save_object(&mut output, &ctx, &object).unwrap();
    let mut input = Reader::new(&output.buf);
    let loaded = object_io::load_object(&mut input, &ctx, crate::save::WORLD_VERSION, 8)
        .unwrap()
        .unwrap();
    assert_eq!(input.remaining(), 0);
    assert!(
        matches!(loaded.kind, ObjectKind::Generator(g) if g.connected && g.condition == 67 && g.fuel == 2.5)
    );
    assert_eq!(
        generator_power_range(),
        GeneratorPowerRange {
            radius: 7,
            vertical: 2
        }
    );
}

#[test]
fn generator_ranges_preserve_negative_levels_and_inclusive_radius() {
    let range = GeneratorPowerRange {
        radius: 5,
        vertical: 1,
    };
    assert!(range.reaches((-8, -8, -2), -5, -4, -3));
    assert!(!range.reaches((-8, -8, -2), -5, -4, -4));
    assert!(!range.reaches((-8, -8, -2), -4, -4, -2));
    assert!(range.reaches((0, 0, MIN_LEVEL), 0, 0, MIN_LEVEL));
    assert!(!range.reaches((0, 0, MIN_LEVEL), 0, 0, MIN_LEVEL - 1));
    assert!(range.reaches((0, 0, MAX_LEVEL), 0, 0, MAX_LEVEL));
    assert!(!range.reaches((0, 0, MAX_LEVEL), 0, 0, MAX_LEVEL + 1));
    // Both the radius product and the Z arithmetic are Java int operations.
    assert!(!GeneratorPowerRange {
        radius: 50_000,
        vertical: 1
    }
    .reaches((0, 0, 0), 0, 0, 0));
}
