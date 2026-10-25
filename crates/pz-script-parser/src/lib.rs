//! `zombie.scripting.ScriptParser`: the brace/comma tokenizer every script
//! object parses its body with, plus comment stripping and the top-level
//! token splitter used for files and modules.
//!
//! The scanners mirror the original loops exactly, including their quirks
//! (for example, the character straight after a nested block's `}` is never
//! inspected), because mods rely on whatever the original accepts.
//!
//! This is a leaf crate shared by `pz-scripts` (script files), `pz-fs`
//! (`mods/default.txt`) and `pz-text` (`fonts.txt`).

pub mod java;

pub const DEFAULT_INDENTATION: &str = "    ";
pub const DEFAULT_EOL: &str = "\n";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlockElement {
    Block(usize),
    Value(usize),
}

/// `ScriptParser.Block`. `elements` keeps source order across values and
/// child blocks; it indexes into `values` and `children`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Block {
    pub type_: Option<String>,
    pub id: Option<String>,
    pub elements: Vec<BlockElement>,
    pub values: Vec<Value>,
    pub children: Vec<Block>,
    uid: Option<String>,
    pub comment: Option<String>,
}

/// `ScriptParser.Value`: the raw text between commas, `key = value` or not.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Value {
    pub string: String,
}

impl Value {
    pub fn new(string: impl Into<String>) -> Self {
        Value { string: string.into() }
    }

    /// `getKey`: text before the first `=`, or the whole string.
    pub fn get_key(&self) -> &str {
        match self.string.find('=') {
            Some(p) => &self.string[..p],
            None => &self.string,
        }
    }

    /// `getValue`: text after the first `=`, or "".
    pub fn get_value(&self) -> &str {
        match self.string.find('=') {
            Some(p) => &self.string[p + 1..],
            None => "",
        }
    }

    pub fn pretty_print(&self, indent: usize, sb: &mut String, eol: &str, indentation: &str) {
        sb.push_str(&indentation.repeat(indent));
        sb.push_str(java::trim(&self.string));
        sb.push(',');
        sb.push_str(eol);
    }
}

impl Block {
    pub fn uid(&self) -> Option<&str> {
        self.uid.as_deref()
    }

    pub fn type_str(&self) -> &str {
        self.type_.as_deref().unwrap_or("")
    }

    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    fn push_child(&mut self, child: Block) {
        self.children.push(child);
        self.elements.push(BlockElement::Block(self.children.len() - 1));
    }

    fn push_value(&mut self, value: Value) {
        self.values.push(value);
        self.elements.push(BlockElement::Value(self.values.len() - 1));
    }

    pub fn add_block(&mut self, type_: &str, id: Option<&str>) -> &mut Block {
        self.push_child(Block {
            type_: Some(type_.to_string()),
            id: id.map(str::to_string),
            ..Block::default()
        });
        self.children.last_mut().unwrap()
    }

    /// `getBlock(type, id)`: first child with that type and id (null-safe).
    pub fn get_block(&self, type_: &str, id: Option<&str>) -> Option<&Block> {
        self.children
            .iter()
            .find(|b| b.type_.as_deref() == Some(type_) && b.id.as_deref() == id)
    }

    fn value_index(&self, key: &str) -> Option<usize> {
        self.values.iter().position(|v| {
            matches!(v.string.find('='), Some(p) if p > 0) && java::trim(v.get_key()) == key
        })
    }

    /// `getValue(key)`: first `key = ...` value whose trimmed key matches.
    pub fn get_value(&self, key: &str) -> Option<&Value> {
        self.value_index(key).map(|i| &self.values[i])
    }

    pub fn set_value(&mut self, key: &str, value: &str) {
        match self.value_index(key) {
            Some(i) => self.values[i].string = format!("{key} = {value}"),
            None => {
                self.add_value(key, value);
            }
        }
    }

    pub fn add_value(&mut self, key: &str, value: &str) -> &mut Value {
        self.push_value(Value::new(format!("{key} = {value}")));
        self.values.last_mut().unwrap()
    }

    /// `moveValueAfter`: reorders both `values` and `elements`.
    pub fn move_value_after(&mut self, key_move: &str, key_after: &str) {
        let (Some(mv), Some(af)) = (self.value_index(key_move), self.value_index(key_after)) else {
            return;
        };
        // Replay the Java list operations on value identities (original
        // indices), then renumber.
        let mut vals: Vec<usize> = (0..self.values.len()).collect();
        let mut els = self.elements.clone();
        vals.retain(|&v| v != mv);
        els.retain(|e| *e != BlockElement::Value(mv));
        let at = els.iter().position(|e| *e == BlockElement::Value(af)).unwrap() + 1;
        els.insert(at, BlockElement::Value(mv));
        let at = vals.iter().position(|&v| v == af).unwrap() + 1;
        vals.insert(at, mv);
        let mut new_pos = vec![0; vals.len()];
        for (pos, &old) in vals.iter().enumerate() {
            new_pos[old] = pos;
        }
        let mut old_values: Vec<Option<Value>> = std::mem::take(&mut self.values).into_iter().map(Some).collect();
        self.values = vals.iter().map(|&old| old_values[old].take().unwrap()).collect();
        self.elements = els
            .into_iter()
            .map(|e| match e {
                BlockElement::Value(i) => BlockElement::Value(new_pos[i]),
                b => b,
            })
            .collect();
    }

    pub fn pretty_print(&self, indent: usize, sb: &mut String, eol: &str, indentation: &str) {
        let pad = indentation.repeat(indent);
        sb.push_str(&pad);
        if !java::is_null_or_whitespace(self.comment.as_deref()) {
            sb.push_str(self.comment.as_deref().unwrap());
            sb.push_str(eol);
            sb.push_str(&pad);
        }
        // Java appends a null type as "null".
        sb.push_str(self.type_.as_deref().unwrap_or("null"));
        if let Some(id) = &self.id {
            sb.push(' ');
            sb.push_str(id);
        }
        sb.push_str(eol);
        sb.push_str(&pad);
        sb.push('{');
        sb.push_str(eol);
        self.pretty_print_elements(indent + 1, sb, eol, indentation);
        sb.push_str(&pad);
        sb.push('}');
        sb.push_str(eol);
    }

    pub fn pretty_print_elements(&self, indent: usize, sb: &mut String, eol: &str, indentation: &str) {
        let mut prev: Option<&BlockElement> = None;
        for element in &self.elements {
            match element {
                BlockElement::Block(i) => {
                    if prev.is_some() {
                        sb.push_str(eol);
                    }
                    self.children[*i].pretty_print(indent, sb, eol, indentation);
                }
                BlockElement::Value(i) => {
                    if matches!(prev, Some(BlockElement::Block(_))) {
                        sb.push_str(eol);
                    }
                    self.values[*i].pretty_print(indent, sb, eol, indentation);
                }
            }
            prev = Some(element);
        }
    }
}

/// `ScriptParser.readBlock`. `current_object` is
/// `ScriptBucket.getCurrentScriptObject()`, used to tag child blocks with a
/// uid holding the UTF-16 offset of their `{`.
pub fn read_block(s: &str, mut start: usize, block: &mut Block, current_object: Option<&str>) -> usize {
    let b = s.as_bytes();
    let mut i = start;
    while i < b.len() {
        match b[i] {
            b'{' => {
                let header = java::trim(&s[start..i]);
                let ss = java::split_ws(header);
                let mut child = Block {
                    // `ss[0]` throws in Java when the header is all whitespace
                    // after a non-empty trim, which cannot happen; an empty
                    // header gives [""].
                    type_: Some(ss.first().copied().unwrap_or("").to_string()),
                    id: ss.get(1).map(|s| s.to_string()),
                    ..Block::default()
                };
                if let Some(obj) = current_object {
                    let off = java::utf16_len(&s[..i]);
                    child.uid = Some(format!("UID:{obj}@{}@{off}", child.type_str()));
                }
                let end = read_block(s, i + 1, &mut child, current_object);
                block.push_child(child);
                start = end;
                i = end;
            }
            b'}' => return i + 1,
            b',' => {
                block.push_value(Value::new(&s[start..i]));
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    i
}

/// `ScriptParser.parse`: a nameless root block whose children are the
/// top-level blocks of `s`.
pub fn parse(s: &str) -> Block {
    parse_with(s, None)
}

pub fn parse_with(s: &str, current_object: Option<&str>) -> Block {
    let mut block = Block::default();
    read_block(s, 0, &mut block, current_object);
    block
}

/// `ScriptParser.stripComments`: removes `/* ... */` comments, scanning from
/// the end and treating nested comments as one span.
pub fn strip_comments(total_file: &str) -> String {
    let mut sb = total_file.to_string();
    let mut end = java::last_index_of_all(&sb, "*/");
    while end != -1 {
        let mut start = java::last_index_of(&sb, "/*", end - 1);
        if start == -1 {
            break;
        }
        let mut inner_end = java::last_index_of(&sb, "*/", end - 1);
        while inner_end > start {
            let inner_start = start;
            start = java::last_index_of(&sb, "/*", start - 2);
            if start == -1 {
                break;
            }
            inner_end = java::last_index_of(&sb, "*/", inner_start - 2);
        }
        if start == -1 {
            break;
        }
        sb.replace_range(start as usize..(end + 2) as usize, "");
        end = java::last_index_of(&sb, "*/", start);
    }
    sb
}

/// `ScriptParser.parseTokens`: splits text into top-level `header { ... }`
/// tokens by brace depth, plus any trailing text.
pub fn parse_tokens(total_file: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut rest: &str = total_file;
    loop {
        let mut depth = 0i32;
        let mut open: i64 = 0;
        let mut closed: i64 = 0;
        if java::index_of(rest, "}", open + 1) == -1 {
            break;
        }
        loop {
            open = java::index_of(rest, "{", open + 1);
            closed = java::index_of(rest, "}", closed + 1);
            if (closed < open && closed != -1) || open == -1 {
                open = closed;
                depth -= 1;
            } else {
                closed = open;
                depth += 1;
            }
            if depth <= 0 {
                break;
            }
        }
        // substring(0, -1 + 1) when no brace was found: an empty token.
        let cut = (open + 1) as usize;
        tokens.push(java::trim(&rest[..cut]).to_string());
        rest = &rest[cut..];
    }
    if !java::trim(rest).is_empty() {
        tokens.push(java::trim(rest).to_string());
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_blocks_and_values() {
        let root = parse("item Axe { Weight = 3, DisplayName = Axe, model { a = 1, }, }");
        assert_eq!(root.children.len(), 1);
        let item = &root.children[0];
        assert_eq!(item.type_.as_deref(), Some("item"));
        assert_eq!(item.id.as_deref(), Some("Axe"));
        assert_eq!(item.values.len(), 2);
        assert_eq!(java::trim(item.values[0].get_key()), "Weight");
        assert_eq!(java::trim(item.values[1].get_value()), "Axe");
        assert_eq!(item.get_value("Weight").map(|v| v.get_value()), Some(" 3"));
        let model = &item.children[0];
        assert_eq!(model.type_.as_deref(), Some("model"));
        assert_eq!(model.id, None);
        assert_eq!(item.elements.len(), 3);
    }

    #[test]
    fn char_after_nested_block_is_skipped() {
        // The outer '}' directly follows the inner one, so the outer block
        // is not closed there and swallows the following text.
        let root = parse("a { b { x, }} c, }");
        let a = &root.children[0];
        assert_eq!(a.children.len(), 1);
        assert_eq!(a.values.len(), 1);
        assert_eq!(a.values[0].string, "} c");
    }

    #[test]
    fn strips_comments_including_nested() {
        assert_eq!(strip_comments("a /* x */ b"), "a  b");
        assert_eq!(strip_comments("a /* x /* y */ z */ b"), "a  b");
        assert_eq!(strip_comments("a */ b"), "a */ b");
        assert_eq!(strip_comments("/*a*/x/*b*/"), "x");
    }

    #[test]
    fn tokens_split_by_depth() {
        let t = parse_tokens("module Base { item A { x = 1, } }\nmodule B { }\n trailing");
        assert_eq!(t, vec!["module Base { item A { x = 1, } }", "module B { }", "trailing"]);
    }

    #[test]
    fn pretty_print_round_trip() {
        let root = parse("item Axe { Weight = 3, model { a = 1, }, }");
        let mut sb = String::new();
        root.children[0].pretty_print(0, &mut sb, DEFAULT_EOL, DEFAULT_INDENTATION);
        assert_eq!(sb, "item Axe\n{\n    Weight = 3,\n\n    model\n    {\n        a = 1,\n    }\n}\n");
    }

    #[test]
    fn parses_mod_list() {
        // `ActiveModsFile.read` joins lines without separators.
        let text = "VERSION = 1,mods{    mod = A,    mod = B\\,}maps{}";
        let b = parse(&strip_comments(text));
        assert_eq!(b.get_value("VERSION").map(|v| java::trim(v.get_value())), Some("1"));
        let mods = b.get_block("mods", None).unwrap();
        let ids: Vec<_> = mods.values.iter().map(|v| java::trim(v.get_value())).collect();
        assert_eq!(ids, ["A", "B\\"]);
        assert!(b.get_block("maps", None).unwrap().values.is_empty());
    }

    #[test]
    fn root_pretty_print_elements() {
        let mut b = Block::default();
        b.set_value("VERSION", "1");
        b.add_block("mods", None).add_value("mod", "A");
        b.add_block("maps", None);
        let mut s = String::new();
        b.pretty_print_elements(0, &mut s, "\n", "    ");
        assert_eq!(s, "VERSION = 1,\n\nmods\n{\n    mod = A,\n}\n\nmaps\n{\n}\n");
    }

    #[test]
    fn utf16_offsets_match_byte_scan() {
        // Non-ASCII text before the delimiters: splits are unaffected and the
        // uid offset is in UTF-16 units.
        let root = parse_with(&strip_comments("/* é */ font Ünï { fnt = ä.fnt, }"), Some("X"));
        let f = &root.children[0];
        assert_eq!((f.type_str(), f.id.as_deref()), ("font", Some("Ünï")));
        assert_eq!(java::trim(f.get_value("fnt").unwrap().get_value()), "ä.fnt");
        assert_eq!(f.uid(), Some("UID:X@font@10"));
    }

    #[test]
    fn move_value_after_reorders() {
        let mut b = parse("x { a = 1, b = 2, c = 3, }").children.remove(0);
        b.move_value_after("a", "c");
        let keys: Vec<_> = b.values.iter().map(|v| java::trim(v.get_key()).to_string()).collect();
        assert_eq!(keys, vec!["b", "c", "a"]);
        assert_eq!(b.elements, vec![BlockElement::Value(0), BlockElement::Value(1), BlockElement::Value(2)]);
    }
}
