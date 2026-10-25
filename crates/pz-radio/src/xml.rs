//! The part of the JDK's DOM parser `RadioData` uses: a document read into
//! an element tree with qualified node names, attribute values normalised
//! (XML 1.0 attribute-value normalisation, references resolved) and the
//! text content of elements.

use quick_xml::events::Event;
use quick_xml::reader::Reader;
use quick_xml::XmlVersion;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Element {
    /// `getNodeName()`: the qualified name.
    pub name: String,
    /// Attributes by qualified name, in document order.
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Element(Element),
    Text(String),
}

impl Element {
    /// The element children, in document order (`RadioData.getChildNodes`).
    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|n| match n {
            Node::Element(e) => Some(e),
            Node::Text(_) => None,
        })
    }

    /// `getAttributes().getNamedItem(name).getTextContent()`.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// `getTextContent()`: the text of every descendant, in order.
    pub fn text_content(&self) -> String {
        let mut s = String::new();
        self.collect_text(&mut s);
        s
    }

    fn collect_text(&self, out: &mut String) {
        for n in &self.children {
            match n {
                Node::Element(e) => e.collect_text(out),
                Node::Text(t) => out.push_str(t),
            }
        }
    }

    /// The first element named `name` in document order, this one
    /// included (`getElementsByTagName(name).item(0)` on a document).
    pub fn first_named(&self, name: &str) -> Option<&Element> {
        if self.name == name {
            return Some(self);
        }
        self.elements().find_map(|e| e.first_named(name))
    }
}

fn push_text(stack: &mut [Element], s: &str) {
    if let Some(top) = stack.last_mut() {
        if let Some(Node::Text(t)) = top.children.last_mut() {
            t.push_str(s);
        } else {
            top.children.push(Node::Text(s.to_string()));
        }
    }
}

/// Parses a whole document; `Err` where the JDK parser throws.
pub fn parse_document(bytes: &[u8]) -> Result<Element, String> {
    let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut reader = Reader::from_str(text);
    reader.config_mut().check_end_names = true;
    let mut stack: Vec<Element> = Vec::new();
    let mut root: Option<Element> = None;
    loop {
        let ev = reader.read_event().map_err(|e| e.to_string())?;
        match ev {
            Event::Start(ref e) | Event::Empty(ref e) => {
                if root.is_some() {
                    return Err("content after the root element".into());
                }
                let mut el = Element {
                    name: String::from_utf8_lossy(e.name().as_ref()).into_owned(),
                    ..Default::default()
                };
                for a in e.attributes() {
                    let a = a.map_err(|e| e.to_string())?;
                    let v = a
                        .normalized_value(XmlVersion::Implicit1_0)
                        .map_err(|e| e.to_string())?;
                    el.attrs.push((
                        String::from_utf8_lossy(a.key.as_ref()).into_owned(),
                        v.into_owned(),
                    ));
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
                push_text(&mut stack, &s);
            }
            Event::CData(t) => {
                let s = t.xml10_content().map_err(|e| e.to_string())?;
                push_text(&mut stack, &s);
            }
            Event::GeneralRef(r) => {
                let s = if r.is_char_ref() {
                    r.resolve_char_ref()
                        .map_err(|e| e.to_string())?
                        .ok_or("bad character reference")?
                        .to_string()
                } else {
                    let name = r.decode().map_err(|e| e.to_string())?;
                    quick_xml::escape::resolve_predefined_entity(&name)
                        .ok_or_else(|| format!("undeclared entity {name}"))?
                        .to_string()
                };
                push_text(&mut stack, &s);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_tree() {
        let doc =
            parse_document(b"<?xml version='1.0'?><A x=' 1\t2 '><B>t&amp;u</B><!-- c --><C/>v</A>")
                .unwrap();
        assert_eq!(doc.name, "A");
        assert_eq!(doc.attr("x"), Some(" 1 2 "));
        assert_eq!(doc.text_content(), "t&uv");
        assert_eq!(doc.elements().count(), 2);
        assert!(doc.first_named("C").is_some());
        assert!(parse_document(b"<A><B></A>").is_err());
    }
}
