use crate::fail;
use wasm_bindgen::{prelude::*, JsCast};
use web_sys::{DomParser, Element, HtmlElement, Node, SupportedType};

#[wasm_bindgen]
pub fn render_math(target: HtmlElement, mathml: String) -> Result<(), JsValue> {
    if mathml.len() > 262144 || mathml.contains("<!") || mathml.contains("<?") {
        return Err(fail("Invalid formula"));
    }
    let document = DomParser::new()?.parse_from_string(&mathml, SupportedType::ApplicationXml)?;
    let root = document
        .document_element()
        .ok_or_else(|| fail("Invalid formula"))?;
    let mut remaining = 8192usize;
    validate(&root, 0, &mut remaining)?;
    if root.local_name() != "math" {
        return Err(fail("Invalid formula"));
    }
    target.set_text_content(None);
    target.append_child(&root)?;
    Ok(())
}

fn validate(element: &Element, depth: u32, remaining: &mut usize) -> Result<(), JsValue> {
    *remaining = remaining
        .checked_sub(1)
        .ok_or_else(|| fail("Formula too large"))?;
    if depth > 64
        || element.namespace_uri().as_deref() != Some("http://www.w3.org/1998/Math/MathML")
        || !matches!(
            element.local_name().as_str(),
            "math"
                | "mrow"
                | "mi"
                | "mn"
                | "mo"
                | "mtext"
                | "mspace"
                | "ms"
                | "mfrac"
                | "msqrt"
                | "mroot"
                | "mstyle"
                | "merror"
                | "mpadded"
                | "mphantom"
                | "mfenced"
                | "menclose"
                | "msub"
                | "msup"
                | "msubsup"
                | "munder"
                | "mover"
                | "munderover"
                | "mmultiscripts"
                | "mprescripts"
                | "none"
                | "mtable"
                | "mtr"
                | "mtd"
                | "mlabeledtr"
        )
    {
        return Err(fail("Unsupported formula element"));
    }
    // The converter tags environments with a class; it is styling only, so drop it rather than reject the formula.
    element.remove_attribute("class")?;
    let attributes = element.attributes();
    for i in 0..attributes.length() {
        let attribute = attributes
            .item(i)
            .ok_or_else(|| fail("Invalid formula attribute"))?;
        if attribute.name() == "style" {
            if attribute.value().len() > 256
                || !attribute.value().split(';').all(|entry| {
                    entry.trim().is_empty()
                        || entry.split_once(':').is_some_and(|(key, value)| {
                            matches!(
                                key.trim(),
                                "color" | "background-color" | "border" | "margin-left" | "height"
                            ) && value
                                .bytes()
                                .all(|b| b.is_ascii_alphanumeric() || b" #.(),%+-".contains(&b))
                        })
                })
            {
                return Err(fail("Unsupported formula style"));
            }
            continue;
        }
        if !matches!(
            attribute.name().as_str(),
            "xmlns"
                | "display"
                | "displaystyle"
                | "scriptlevel"
                | "mathvariant"
                | "mathsize"
                | "mathcolor"
                | "mathbackground"
                | "stretchy"
                | "symmetric"
                | "fence"
                | "separator"
                | "accent"
                | "accentunder"
                | "largeop"
                | "movablelimits"
                | "form"
                | "lspace"
                | "rspace"
                | "minsize"
                | "maxsize"
                | "width"
                | "height"
                | "depth"
                | "voffset"
                | "linethickness"
                | "bevelled"
                | "notation"
                | "rowalign"
                | "columnalign"
                | "columnspacing"
                | "rowspacing"
                | "columnlines"
                | "rowlines"
                | "frame"
                | "framespacing"
                | "equalrows"
                | "equalcolumns"
                | "columnspan"
                | "rowspan"
        ) || attribute.value().len() > 256
        {
            return Err(fail("Unsupported formula attribute"));
        }
    }
    let children = element.child_nodes();
    for i in 0..children.length() {
        let node = children
            .item(i)
            .ok_or_else(|| fail("Invalid formula node"))?;
        if node.node_type() == Node::ELEMENT_NODE {
            validate(&node.dyn_into::<Element>()?, depth + 1, remaining)?;
        } else if node.node_type() != Node::TEXT_NODE {
            return Err(fail("Unsupported formula node"));
        }
    }
    Ok(())
}
