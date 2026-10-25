//! The parts of `zombie.scripting.ScriptParser` that `FontsFile` uses
//! (`stripComments`, `parse`, `Block.getValue`).
//!
//! The script workstream owns the full parser; this copy is kept to the
//! block/value reading `fonts.txt` needs and can be replaced by it.

/// `ScriptParser.Block`.
#[derive(Clone, Debug, Default)]
pub struct Block {
    pub r#type: String,
    pub id: Option<String>,
    pub values: Vec<Value>,
    pub children: Vec<Block>,
}

/// `ScriptParser.Value`: the raw text between separators.
#[derive(Clone, Debug)]
pub struct Value {
    pub string: Vec<u16>,
}

impl Value {
    /// `getKey()`: text before the first '='.
    pub fn key(&self) -> String {
        match self.string.iter().position(|&c| c == 0x3d) {
            Some(p) => String::from_utf16_lossy(&self.string[..p]),
            None => String::from_utf16_lossy(&self.string),
        }
    }

    /// `getValue()`: text after the first '=' ("" if there is none).
    pub fn value(&self) -> String {
        match self.string.iter().position(|&c| c == 0x3d) {
            Some(p) => String::from_utf16_lossy(&self.string[p + 1..]),
            None => String::new(),
        }
    }
}

impl Block {
    /// `getValue(key)`: the first value whose trimmed key equals `key`.
    pub fn get_value(&self, key: &str) -> Option<&Value> {
        self.values.iter().find(|v| {
            let p = v.string.iter().position(|&c| c == 0x3d);
            matches!(p, Some(p) if p > 0) && java_trim(&v.key()) == key
        })
    }
}

pub(crate) fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

/// `ScriptParser.parse`.
pub fn parse(s: &str) -> Block {
    let s: Vec<u16> = s.encode_utf16().collect();
    let mut block = Block::default();
    read_block(&s, 0, &mut block);
    block
}

/// `ScriptParser.readBlock`.
fn read_block(s: &[u16], mut start: usize, block: &mut Block) -> usize {
    let mut i = start;
    while i < s.len() {
        match s[i] {
            0x7b => {
                let header = String::from_utf16_lossy(&s[start..i]);
                let header = java_trim(&header);
                // header.split("\\s+")
                let mut parts = header
                    .split([' ', '\t', '\n', '\u{b}', '\u{c}', '\r'])
                    .filter(|p| !p.is_empty());
                let mut child = Block {
                    r#type: parts.next().unwrap_or("").to_string(),
                    id: parts.next().map(str::to_string),
                    ..Block::default()
                };
                i = read_block(s, i + 1, &mut child);
                block.children.push(child);
                start = i;
                // The original's `continue` runs the loop's `++i`, so the
                // char right after '}' is never tested as a separator.
                i += 1;
                continue;
            }
            0x7d => return i + 1,
            0x2c => {
                block.values.push(Value {
                    string: s[start..i].to_vec(),
                });
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    i
}

fn last_index_of(s: &[u16], pat: &[u16; 2], from: isize) -> isize {
    // StringBuilder.lastIndexOf(str, fromIndex)
    let max = s.len() as isize - 2;
    let mut i = from.min(max);
    while i >= 0 {
        if s[i as usize] == pat[0] && s[i as usize + 1] == pat[1] {
            return i;
        }
        i -= 1;
    }
    -1
}

/// `ScriptParser.stripComments`: removes `/* ... */`, innermost-last,
/// allowing nesting.
pub fn strip_comments(total: &str) -> String {
    const OPEN: [u16; 2] = [0x2f, 0x2a];
    const CLOSE: [u16; 2] = [0x2a, 0x2f];
    let mut sb: Vec<u16> = total.encode_utf16().collect();
    let mut end = last_index_of(&sb, &CLOSE, sb.len() as isize);
    while end != -1 {
        let mut start = last_index_of(&sb, &OPEN, end - 1);
        if start == -1 {
            break;
        }
        let mut inner_end = last_index_of(&sb, &CLOSE, end - 1);
        while inner_end > start {
            let inner_start = start;
            start = last_index_of(&sb, &OPEN, start - 2);
            if start == -1 {
                break;
            }
            inner_end = last_index_of(&sb, &CLOSE, inner_start - 2);
        }
        if start == -1 {
            break;
        }
        sb.drain(start as usize..(end as usize + 2).min(sb.len()));
        end = last_index_of(&sb, &CLOSE, start);
    }
    String::from_utf16_lossy(&sb)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_and_values() {
        let b = parse(&strip_comments(
            "VERSION = 1,\n/* c */ font Small { fnt = a.fnt, img = a.png, }",
        ));
        assert_eq!(b.get_value("VERSION").unwrap().value().trim(), "1");
        let f = &b.children[0];
        assert_eq!(
            (f.r#type.as_str(), f.id.as_deref()),
            ("font", Some("Small"))
        );
        assert_eq!(f.get_value("fnt").unwrap().value().trim(), "a.fnt");
    }
}
