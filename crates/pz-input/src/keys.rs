//! What GLFW knows about each LWJGL key: whether it exists on the keyboard,
//! whether it is a keypad key, and the character a printable key shows on a
//! US layout (`glfwGetKeyName` reports the active layout's character; the
//! window layer refines these with [`crate::InputState::learn_key_name`]).

/// How `Keyboard.initKeyNames` treats a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyClass {
    /// No GLFW key: `isKeyDown` is always false.
    Unmapped,
    /// GLFW_KEY_KP_0..KP_9: named `KP_<digit>`.
    KeypadDigit,
    /// GLFW_KEY_KP_DECIMAL..KP_EQUAL: named `KP_<name>`.
    KeypadOther,
    /// Named by the layout's character.
    Printable(char),
    /// Named after its `KEY_` constant.
    Named,
}

/// The class of LWJGL key `k` (PC set-1 scan code numbering).
pub fn class(k: i32) -> KeyClass {
    use KeyClass::*;
    let printable = |c| Printable(c);
    match k {
        2..=10 => printable((b'1' + (k - 2) as u8) as char),
        11 => printable('0'),
        12 => printable('-'),
        13 => printable('='),
        16 => printable('Q'),
        17 => printable('W'),
        18 => printable('E'),
        19 => printable('R'),
        20 => printable('T'),
        21 => printable('Y'),
        22 => printable('U'),
        23 => printable('I'),
        24 => printable('O'),
        25 => printable('P'),
        26 => printable('['),
        27 => printable(']'),
        30 => printable('A'),
        31 => printable('S'),
        32 => printable('D'),
        33 => printable('F'),
        34 => printable('G'),
        35 => printable('H'),
        36 => printable('J'),
        37 => printable('K'),
        38 => printable('L'),
        39 => printable(';'),
        40 => printable('\''),
        41 => printable('`'),
        43 => printable('\\'),
        44 => printable('Z'),
        45 => printable('X'),
        46 => printable('C'),
        47 => printable('V'),
        48 => printable('B'),
        49 => printable('N'),
        50 => printable('M'),
        51 => printable(','),
        52 => printable('.'),
        53 => printable('/'),
        // Keypad digits.
        71..=73 | 75..=77 | 79..=82 => KeypadDigit,
        // Keypad multiply, subtract, add, decimal, equals, enter, divide.
        55 | 74 | 78 | 83 | 141 | 156 | 181 => KeypadOther,
        1 | 14 | 15 | 28 | 29 | 42 | 54 | 56 | 57 | 58 | 59..=68 | 69 | 70 | 87 | 88 | 100..=105 | 157 | 183 | 184 | 197 | 199 | 200 | 201 | 203 | 205 | 207
        | 208 | 209 | 210 | 211 | 219 | 220 | 221 => Named,
        _ => Unmapped,
    }
}

/// Whether `k` is a printable key (named by the layout).
pub fn is_printable(k: i32) -> bool {
    matches!(class(k), KeyClass::Printable(_))
}
