//! Text: `zombie.core.Translator` (translation tables) and
//! `zombie.ui.TextManager` / `AngelCodeFont` (bitmap fonts).

pub mod font;
pub mod jfmt;
pub mod json;
pub mod text_manager;
pub mod translator;

pub use font::{AngelCodeFont, CharDef, FontError};
pub use jfmt::JavaArg;
pub use text_manager::{TextManager, TextOptions};
pub use translator::{Language, Languages, ModDirs, Translator, TranslatorEnv, TranslatorError};

/// `zombie.ui.UIFont`, in original ordinal order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum UIFont {
    Small,
    Medium,
    Large,
    Massive,
    MainMenu1,
    MainMenu2,
    Cred1,
    Cred2,
    NewSmall,
    NewMedium,
    NewLarge,
    Code,
    CodeSmall,
    CodeMedium,
    CodeLarge,
    MediumNew,
    AutoNormSmall,
    AutoNormMedium,
    AutoNormLarge,
    Dialogue,
    Intro,
    Handwritten,
    DebugConsole,
    Title,
    SdfRegular,
    SdfBold,
    SdfItalic,
    SdfBoldItalic,
    SdfOldRegular,
    SdfOldBold,
    SdfOldItalic,
    SdfOldBoldItalic,
    SdfRobertoSans,
    SdfCaveat,
}

impl UIFont {
    /// `UIFont.values()`.
    pub const ALL: [UIFont; 34] = {
        use UIFont::*;
        [
            Small,
            Medium,
            Large,
            Massive,
            MainMenu1,
            MainMenu2,
            Cred1,
            Cred2,
            NewSmall,
            NewMedium,
            NewLarge,
            Code,
            CodeSmall,
            CodeMedium,
            CodeLarge,
            MediumNew,
            AutoNormSmall,
            AutoNormMedium,
            AutoNormLarge,
            Dialogue,
            Intro,
            Handwritten,
            DebugConsole,
            Title,
            SdfRegular,
            SdfBold,
            SdfItalic,
            SdfBoldItalic,
            SdfOldRegular,
            SdfOldBold,
            SdfOldItalic,
            SdfOldBoldItalic,
            SdfRobertoSans,
            SdfCaveat,
        ]
    };

    /// `name()`.
    pub fn name(self) -> &'static str {
        const NAMES: [&str; 34] = [
            "Small",
            "Medium",
            "Large",
            "Massive",
            "MainMenu1",
            "MainMenu2",
            "Cred1",
            "Cred2",
            "NewSmall",
            "NewMedium",
            "NewLarge",
            "Code",
            "CodeSmall",
            "CodeMedium",
            "CodeLarge",
            "MediumNew",
            "AutoNormSmall",
            "AutoNormMedium",
            "AutoNormLarge",
            "Dialogue",
            "Intro",
            "Handwritten",
            "DebugConsole",
            "Title",
            "SdfRegular",
            "SdfBold",
            "SdfItalic",
            "SdfBoldItalic",
            "SdfOldRegular",
            "SdfOldBold",
            "SdfOldItalic",
            "SdfOldBoldItalic",
            "SdfRobertoSans",
            "SdfCaveat",
        ];
        NAMES[self as usize]
    }

    /// `ordinal()`.
    pub fn ordinal(self) -> usize {
        self as usize
    }

    /// `UIFont.FromString`: exact, case-sensitive name match.
    pub fn from_string(s: &str) -> Option<UIFont> {
        UIFont::ALL.into_iter().find(|f| f.name() == s)
    }
}
