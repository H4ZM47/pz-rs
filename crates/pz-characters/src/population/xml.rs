//! The part of JAXB (`javax.xml.bind`, the bundled reference
//! implementation) the population loaders use: a document read into a
//! small element tree, then fields bound by element name.
//!
//! Binding rules, checked against the original on invented documents:
//! * Only elements without a namespace match; names are case-sensitive.
//!   Unknown elements and attributes are ignored.
//! * A `String` element is its text after its last child element (comments
//!   are skipped, CDATA and references are text, line ends normalised),
//!   kept verbatim; an empty element is `""`. A repeated element
//!   overwrites, so the last one wins.
//! * An `int` element goes through `DatatypeConverterImpl._parseInt`
//!   ([`jaxb_int`]); text it rejects stores 0.
//! * A `boolean` element goes through `_parseBoolean` ([`jaxb_bool`]);
//!   text it rejects stores `false`.
//! * A list element appends each occurrence.
//! * A missing attribute is `null`.
//! * A document that is not well-formed, or whose root element is not the
//!   expected one, fails to load.

use quick_xml::events::Event;
use quick_xml::name::ResolveResult;
use quick_xml::reader::NsReader;
use quick_xml::XmlVersion;

/// One element of a parsed document.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Element {
    /// The local name.
    pub name: String,
    /// The element is in a namespace (JAXB binds none of these).
    pub namespaced: bool,
    /// Unprefixed attributes, by local name, in document order.
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Element(Element),
    Text(String),
}

impl Element {
    /// The child elements JAXB binds to `name`, in document order.
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> + 'a {
        self.children.iter().filter_map(move |n| match n {
            Node::Element(e) if !e.namespaced && e.name == name => Some(e),
            _ => None,
        })
    }

    /// The text JAXB reads for a `String` bound to this element: the text
    /// after the last child element.
    pub fn leaf_text(&self) -> String {
        let mut text = String::new();
        for n in &self.children {
            match n {
                Node::Element(_) => text.clear(),
                Node::Text(t) => text.push_str(t),
            }
        }
        text
    }

    /// An attribute value; `None` is `null`.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    /// A `String` field: the last `name` child's text, or `None` if absent.
    pub fn string_field(&self, name: &str) -> Option<String> {
        self.children_named(name).last().map(Element::leaf_text)
    }

    /// An `int` field, or `None` if absent.
    pub fn int_field(&self, name: &str) -> Option<i32> {
        self.children_named(name).last().map(|e| jaxb_int(&e.leaf_text()))
    }

    /// A `boolean` field, or `None` if absent.
    pub fn bool_field(&self, name: &str) -> Option<bool> {
        self.children_named(name).last().map(|e| jaxb_bool(&e.leaf_text()))
    }

    /// An `ArrayList<String>` field: every `name` child's text.
    pub fn string_list(&self, name: &str) -> Vec<String> {
        self.children_named(name).map(Element::leaf_text).collect()
    }
}

/// `WhiteSpaceProcessor.isWhiteSpace`.
fn is_ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r')
}

/// `DatatypeConverterImpl._parseInt`, with a rejected literal as 0:
/// whitespace anywhere is skipped, a `-` anywhere negates, `+` is ignored,
/// digits accumulate with `int` overflow.
pub fn jaxb_int(s: &str) -> i32 {
    let mut sign = 1i32;
    let mut r = 0i32;
    for c in s.chars() {
        if is_ws(c) {
        } else if c.is_ascii_digit() {
            r = r.wrapping_mul(10).wrapping_add(c as i32 - '0' as i32);
        } else if c == '-' {
            sign = -1;
        } else if c == '+' {
        } else {
            return 0;
        }
    }
    r.wrapping_mul(sign)
}

/// `DatatypeConverterImpl._parseFloat`, with a rejected literal as 0:
/// trimmed; `NaN`, `INF`, `-INF`; otherwise it must start and end with a
/// digit, sign or period and go through `Float.parseFloat`.
pub fn jaxb_float(s: &str) -> f32 {
    let t = s.trim_matches(is_ws);
    match t {
        "NaN" => return f32::NAN,
        "INF" => return f32::INFINITY,
        "-INF" => return f32::NEG_INFINITY,
        _ => {}
    }
    let edge = |c: Option<char>| c.is_some_and(|c| c.is_ascii_digit() || matches!(c, '+' | '-' | '.'));
    if !edge(t.chars().next()) || !edge(t.chars().last()) {
        return 0.0;
    }
    pz_scripts::java::parse_f32(t).unwrap_or(0.0)
}

/// `DatatypeConverterImpl._parseBoolean` as JAXB stores it in a
/// `boolean`: `null` results and exceptions are `false`.
pub fn jaxb_bool(s: &str) -> bool {
    parse_boolean(&s.encode_utf16().collect::<Vec<u16>>()) == Some(true)
}

/// `_parseBoolean`'s scan over UTF-16 units; `None` is `null` or an
/// out-of-range read.
fn parse_boolean(lit: &[u16]) -> Option<bool> {
    let ws = |c: u16| c < 0x80 && is_ws(c as u8 as char);
    let len = lit.len();
    if len == 0 {
        return None;
    }
    let mut i = 0;
    let mut ch;
    loop {
        ch = *lit.get(i)?;
        i += 1;
        if !(ws(ch) && i < len) {
            break;
        }
    }
    let mut value = false;
    let word = |w: &[u8], i: &mut usize| -> Option<bool> {
        let mut idx = 0;
        loop {
            let c = *lit.get(*i)?;
            *i += 1;
            let m = w[idx] as u16 == c;
            idx += 1;
            if !(m && *i < len && idx < w.len()) {
                break;
            }
        }
        Some(idx == w.len())
    };
    match ch {
        0x31 => value = true,
        0x30 => value = false,
        0x74 => {
            if !word(b"rue", &mut i)? {
                return Some(false);
            }
            value = true;
        }
        0x66 => {
            if !word(b"alse", &mut i)? {
                return Some(false);
            }
            value = false;
        }
        _ => {}
    }
    if i < len {
        loop {
            ch = lit[i];
            i += 1;
            if !(ws(ch) && i < len) {
                break;
            }
        }
    }
    (i == len).then_some(value)
}

/// Parse a document (UTF-8, optional BOM) into its root element.
pub fn parse_document(bytes: &[u8]) -> Result<Element, String> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
    let mut reader = NsReader::from_str(text);
    reader.config_mut().check_end_names = true;
    let mut stack: Vec<Element> = Vec::new();
    let mut root: Option<Element> = None;
    let push_text = |stack: &mut Vec<Element>, t: &str| -> Result<(), String> {
        match stack.last_mut() {
            Some(top) => {
                if let Some(Node::Text(prev)) = top.children.last_mut() {
                    prev.push_str(t);
                } else {
                    top.children.push(Node::Text(t.to_string()));
                }
                Ok(())
            }
            None if t.chars().all(is_ws) => Ok(()),
            None => Err("content outside the root element".into()),
        }
    };
    loop {
        let (ns, ev) = reader.read_resolved_event().map_err(|e| e.to_string())?;
        let namespaced = !matches!(ns, ResolveResult::Unbound);
        match ev {
            Event::Start(ref e) | Event::Empty(ref e) => {
                if root.is_some() {
                    return Err("content after the root element".into());
                }
                let mut el = Element {
                    name: String::from_utf8_lossy(e.local_name().as_ref()).into_owned(),
                    namespaced,
                    ..Default::default()
                };
                for a in e.attributes() {
                    let a = a.map_err(|e| e.to_string())?;
                    let key = a.key.as_ref();
                    if key.starts_with(b"xmlns") || key.contains(&b':') {
                        continue;
                    }
                    let v = a.normalized_value(XmlVersion::Implicit1_0).map_err(|e| e.to_string())?;
                    el.attrs.push((String::from_utf8_lossy(key).into_owned(), v.into_owned()));
                }
                if matches!(ev, Event::Empty(_)) {
                    close(&mut stack, &mut root, el);
                } else {
                    stack.push(el);
                }
            }
            Event::End(_) => {
                let el = stack.pop().ok_or("unbalanced end tag")?;
                close(&mut stack, &mut root, el);
            }
            Event::Text(t) => {
                let s = t.xml10_content().map_err(|e| e.to_string())?;
                push_text(&mut stack, &s)?;
            }
            Event::CData(t) => {
                let s = t.xml10_content().map_err(|e| e.to_string())?;
                push_text(&mut stack, &s)?;
            }
            Event::GeneralRef(r) => {
                let s = if r.is_char_ref() {
                    r.resolve_char_ref().map_err(|e| e.to_string())?.ok_or("bad character reference")?.to_string()
                } else {
                    let name = r.decode().map_err(|e| e.to_string())?;
                    quick_xml::escape::resolve_predefined_entity(&name)
                        .ok_or_else(|| format!("undeclared entity {name}"))?
                        .to_string()
                };
                push_text(&mut stack, &s)?;
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !stack.is_empty() {
        return Err("unexpected end of document".into());
    }
    root.ok_or_else(|| "no root element".into())
}

fn close(stack: &mut [Element], root: &mut Option<Element>, el: Element) {
    match stack.last_mut() {
        Some(parent) => parent.children.push(Node::Element(el)),
        None => *root = Some(el),
    }
}

/// Parse `bytes` and check JAXB's root element name.
pub fn parse_root(bytes: &[u8], root: &str) -> Result<Element, String> {
    let el = parse_document(bytes)?;
    if el.namespaced || el.name != root {
        return Err(format!("unexpected element \"{}\" (expected \"{root}\")", el.name));
    }
    Ok(el)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ints_follow_parse_int() {
        assert_eq!(jaxb_int(" 3 "), 3);
        assert_eq!(jaxb_int("+2"), 2);
        assert_eq!(jaxb_int("1 2"), 12);
        assert_eq!(jaxb_int("3-"), -3);
        assert_eq!(jaxb_int("abc"), 0);
        assert_eq!(jaxb_int("-0x10"), 0);
        assert_eq!(jaxb_int(""), 0);
        assert_eq!(jaxb_int("2147483648"), i32::MIN);
    }

    #[test]
    fn booleans_follow_parse_boolean() {
        for t in ["1", "true", " true ", "\ttrue\t", "truex", "1a", "true "] {
            assert!(jaxb_bool(t), "{t:?}");
        }
        for f in ["0", "false", "True", "TRUE", "yes", "maybe", "", " ", " tru", "t", "falsey", "0 ", "1ab"] {
            assert!(!jaxb_bool(f), "{f:?}");
        }
    }

    #[test]
    fn text_and_structure() {
        let doc = "\u{feff}<?xml version=\"1.0\"?><r><a>x<b>y</b>t</a><a>  <!--c--> q&amp;<![CDATA[<c>]]>&#x31;</a><a/>\
                   <n xmlns=\"urn:x\"><a>no</a></n><p:a xmlns:p=\"urn:p\">no</p:a><e k=\"v\" p:k=\"w\" xmlns:p=\"urn:p\"/></r>";
        let r = parse_root(doc.as_bytes(), "r").unwrap();
        assert_eq!(r.string_list("a"), vec!["t".to_string(), "   q&<c>1".into(), String::new()]);
        assert_eq!(r.children_named("n").count(), 0);
        let e = r.children_named("e").next().unwrap();
        assert_eq!(e.attrs, vec![("k".to_string(), "v".to_string())]);
        assert!(parse_root(doc.as_bytes(), "R").is_err());
        assert!(parse_document(b"<r><a></r>").is_err());
        assert!(parse_document(b"<r><a>").is_err());
        assert!(parse_document(b"<r>&nope;</r>").is_err());
    }
}
