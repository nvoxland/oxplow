//! Layer-1 **container-parse helpers** — the host-owned, reusable functions a
//! collection plugin uses to turn a raw report into a generic JSON value it can
//! reshape. These are the *trustworthy* half of the two-layer model: they are
//! compiled into oxplow, do no I/O, and are shared by every script runtime
//! (jaq / Starlark). A plugin receives values, never the filesystem — which is
//! what keeps an in-process parse deterministic and `observed`-eligible.
//!
//! All helpers return [`serde_json::Value`] (the lingua franca every runtime
//! adapts to) or a [`HelperError`].
//!
//! ## XML representation
//!
//! [`parse_xml`] produces an **explicit, ordered** tree — deliberately not the
//! lossy `@attr` / `#text` convention — so traversal is predictable and
//! repeated children + mixed content survive:
//!
//! ```json
//! { "tag": "testcase",
//!   "attrs": { "name": "t1", "classname": "m" },
//!   "text": "optional, only when non-empty",
//!   "children": [ { "tag": "failure", "attrs": {}, "children": [] } ] }
//! ```
//!
//! `attrs` and `children` are always present (possibly empty); `text` appears
//! only when the element has non-whitespace text.

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use serde_json::{Map, Number, Value};

/// Errors from a container-parse helper. Each wraps the underlying engine's
/// message; helpers never panic on malformed input.
#[derive(Debug, thiserror::Error)]
pub enum HelperError {
    #[error("xml error: {0}")]
    Xml(String),
    #[error("json error: {0}")]
    Json(String),
    #[error("regex error: {0}")]
    Regex(String),
    #[error("xpath error: {0}")]
    XPath(String),
}

/// Parse JSON text into a value (thin wrapper over `serde_json`).
pub fn parse_json(content: &str) -> Result<Value, HelperError> {
    serde_json::from_str(content).map_err(|e| HelperError::Json(e.to_string()))
}

/// Split into an array of line strings (line terminators stripped, `\r\n` and
/// `\n` both handled).
pub fn lines(content: &str) -> Value {
    Value::Array(
        content
            .lines()
            .map(|l| Value::String(l.to_string()))
            .collect(),
    )
}

/// Parse an lcov `.info` body into an array of records. Each record is an
/// object mapping a record key (`SF`, `DA`, `FN`, …) to the **array** of its
/// values, because keys like `DA` repeat within one record:
///
/// ```json
/// [ { "SF": ["src/a.rs"], "DA": ["1,1", "2,0"] } ]
/// ```
///
/// Records are delimited by `end_of_record`; a trailing record without that
/// marker is still emitted.
pub fn lcov_records(content: &str) -> Value {
    let mut records: Vec<Value> = Vec::new();
    let mut cur: Map<String, Value> = Map::new();
    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if line == "end_of_record" {
            if !cur.is_empty() {
                records.push(Value::Object(std::mem::take(&mut cur)));
            }
            continue;
        }
        if let Some((key, val)) = line.split_once(':') {
            let entry = cur
                .entry(key.to_string())
                .or_insert_with(|| Value::Array(Vec::new()));
            if let Value::Array(arr) = entry {
                arr.push(Value::String(val.to_string()));
            }
        }
    }
    if !cur.is_empty() {
        records.push(Value::Object(cur));
    }
    Value::Array(records)
}

/// Find every match of `pattern` in `text`. Returns an array of matches; each
/// match is an array whose first element is the full match and whose remaining
/// elements are the capture groups (`null` for groups that didn't participate).
pub fn regex_find(pattern: &str, text: &str) -> Result<Value, HelperError> {
    let re = regex::Regex::new(pattern).map_err(|e| HelperError::Regex(e.to_string()))?;
    let mut out: Vec<Value> = Vec::new();
    for caps in re.captures_iter(text) {
        let row: Vec<Value> = (0..caps.len())
            .map(|i| match caps.get(i) {
                Some(m) => Value::String(m.as_str().to_string()),
                None => Value::Null,
            })
            .collect();
        out.push(Value::Array(row));
    }
    Ok(Value::Array(out))
}

/// Evaluate an XPath expression against XML `content`. A node-set yields an
/// array of each node's string value (handy for `//testcase/@name`-style
/// extraction); boolean/number/string results yield the matching scalar. For
/// full tree traversal prefer [`parse_xml`] + the script's own navigation.
pub fn xpath(content: &str, expr: &str) -> Result<Value, HelperError> {
    let package =
        sxd_document::parser::parse(content).map_err(|e| HelperError::XPath(format!("{e:?}")))?;
    let document = package.as_document();
    let value = sxd_xpath::evaluate_xpath(&document, expr)
        .map_err(|e| HelperError::XPath(e.to_string()))?;
    Ok(match value {
        sxd_xpath::Value::Nodeset(ns) => Value::Array(
            ns.document_order()
                .into_iter()
                .map(|n| Value::String(n.string_value()))
                .collect(),
        ),
        sxd_xpath::Value::Boolean(b) => Value::Bool(b),
        sxd_xpath::Value::Number(n) => Number::from_f64(n)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        sxd_xpath::Value::String(s) => Value::String(s),
    })
}

/// Parse XML into the explicit ordered tree documented at the module level.
pub fn parse_xml(content: &str) -> Result<Value, HelperError> {
    let mut reader = Reader::from_str(content);
    let mut stack: Vec<Frame> = Vec::new();
    let mut root: Option<Value> = None;
    loop {
        match reader
            .read_event()
            .map_err(|e| HelperError::Xml(e.to_string()))?
        {
            Event::Start(e) => stack.push(start_frame(&e)?),
            Event::Empty(e) => {
                let frame = start_frame(&e)?;
                finalize(frame, &mut stack, &mut root);
            }
            // quick-xml 0.39 emits each `&entity;` as a separate GeneralRef
            // event, so plain Text is already literal (no unescape needed).
            Event::Text(e) => {
                if let Some(top) = stack.last_mut() {
                    let t = e.decode().map_err(|er| HelperError::Xml(er.to_string()))?;
                    top.text.push_str(&t);
                }
            }
            Event::CData(e) => {
                if let Some(top) = stack.last_mut() {
                    // CDATA is verbatim — no entity resolution.
                    let t = e.decode().map_err(|er| HelperError::Xml(er.to_string()))?;
                    top.text.push_str(&t);
                }
            }
            Event::GeneralRef(e) => {
                if let Some(top) = stack.last_mut() {
                    if let Some(ch) = e
                        .resolve_char_ref()
                        .map_err(|er| HelperError::Xml(er.to_string()))?
                    {
                        // Numeric character reference (&#60; / &#x3c;).
                        top.text.push(ch);
                    } else {
                        // Named entity. Resolve the predefined five; preserve
                        // anything else verbatim rather than silently dropping.
                        let name = e.decode().map_err(|er| HelperError::Xml(er.to_string()))?;
                        match name.as_ref() {
                            "lt" => top.text.push('<'),
                            "gt" => top.text.push('>'),
                            "amp" => top.text.push('&'),
                            "quot" => top.text.push('"'),
                            "apos" => top.text.push('\''),
                            other => {
                                top.text.push('&');
                                top.text.push_str(other);
                                top.text.push(';');
                            }
                        }
                    }
                }
            }
            Event::End(_) => {
                if let Some(frame) = stack.pop() {
                    finalize(frame, &mut stack, &mut root);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    root.ok_or_else(|| HelperError::Xml("no root element".into()))
}

/// An element under construction while streaming XML events.
struct Frame {
    tag: String,
    attrs: Map<String, Value>,
    text: String,
    children: Vec<Value>,
}

fn start_frame(e: &BytesStart) -> Result<Frame, HelperError> {
    let tag = String::from_utf8_lossy(e.name().as_ref()).into_owned();
    let mut attrs = Map::new();
    for a in e.attributes() {
        let a = a.map_err(|er| HelperError::Xml(er.to_string()))?;
        let key = String::from_utf8_lossy(a.key.as_ref()).into_owned();
        let val = a
            .unescape_value()
            .map_err(|er| HelperError::Xml(er.to_string()))?
            .into_owned();
        attrs.insert(key, Value::String(val));
    }
    Ok(Frame {
        tag,
        attrs,
        text: String::new(),
        children: Vec::new(),
    })
}

fn finalize(frame: Frame, stack: &mut [Frame], root: &mut Option<Value>) {
    let value = frame_to_value(frame);
    match stack.last_mut() {
        Some(parent) => parent.children.push(value),
        None => *root = Some(value),
    }
}

fn frame_to_value(frame: Frame) -> Value {
    let mut obj = Map::new();
    obj.insert("tag".into(), Value::String(frame.tag));
    obj.insert("attrs".into(), Value::Object(frame.attrs));
    let text = frame.text.trim();
    if !text.is_empty() {
        obj.insert("text".into(), Value::String(text.to_string()));
    }
    obj.insert("children".into(), Value::Array(frame.children));
    Value::Object(obj)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_json_round_trips() {
        let v = parse_json(r#"{"a":1,"b":["x"]}"#).expect("valid json");
        assert_eq!(v["a"], serde_json::json!(1));
        assert_eq!(v["b"][0], serde_json::json!("x"));
        assert!(parse_json("{not json").is_err());
    }

    #[test]
    fn lines_splits_and_strips_terminators() {
        let v = lines("a\nb\r\nc");
        assert_eq!(v, serde_json::json!(["a", "b", "c"]));
    }

    #[test]
    fn lcov_records_group_repeated_keys() {
        let body =
            "SF:src/a.rs\nDA:1,1\nDA:2,0\nend_of_record\nSF:src/b.rs\nDA:1,1\nend_of_record\n";
        let v = lcov_records(body);
        let arr = v.as_array().expect("array");
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["SF"], serde_json::json!(["src/a.rs"]));
        assert_eq!(arr[0]["DA"], serde_json::json!(["1,1", "2,0"]));
        assert_eq!(arr[1]["SF"], serde_json::json!(["src/b.rs"]));
    }

    #[test]
    fn lcov_emits_trailing_record_without_marker() {
        let v = lcov_records("SF:x\nDA:1,1");
        assert_eq!(v.as_array().expect("array").len(), 1);
    }

    #[test]
    fn regex_find_returns_full_match_and_groups() {
        let v = regex_find(r"(\d+),(\d+)", "1,2 and 3,4").expect("valid regex");
        assert_eq!(v, serde_json::json!([["1,2", "1", "2"], ["3,4", "3", "4"]]));
        assert!(regex_find("(", "x").is_err());
    }

    #[test]
    fn parse_xml_builds_explicit_ordered_tree() {
        let xml = r#"<testsuite name="s">
            <testcase classname="m" name="t1"/>
            <testcase classname="m" name="t2"><failure/></testcase>
        </testsuite>"#;
        let v = parse_xml(xml).expect("valid xml");
        assert_eq!(v["tag"], serde_json::json!("testsuite"));
        assert_eq!(v["attrs"]["name"], serde_json::json!("s"));
        let kids = v["children"].as_array().expect("children");
        // Two testcases, in document order (repeated children preserved).
        assert_eq!(kids.len(), 2);
        assert_eq!(kids[0]["attrs"]["name"], serde_json::json!("t1"));
        assert_eq!(kids[1]["attrs"]["name"], serde_json::json!("t2"));
        assert_eq!(kids[1]["children"][0]["tag"], serde_json::json!("failure"));
    }

    #[test]
    fn parse_xml_captures_text_and_unescapes_attrs() {
        let v = parse_xml(r#"<n a="x &amp; y">hello &lt;world&gt;</n>"#).expect("valid xml");
        assert_eq!(v["attrs"]["a"], serde_json::json!("x & y"));
        assert_eq!(v["text"], serde_json::json!("hello <world>"));
    }

    #[test]
    fn parse_xml_omits_empty_text_key() {
        let v = parse_xml("<n/>").expect("valid xml");
        assert!(v.get("text").is_none());
        assert_eq!(v["attrs"], serde_json::json!({}));
        assert_eq!(v["children"], serde_json::json!([]));
    }

    #[test]
    fn parse_xml_errors_on_malformed_input() {
        assert!(parse_xml("<a><b></a>").is_err());
    }

    #[test]
    fn xpath_selects_attribute_values_in_document_order() {
        let xml = r#"<r><testcase name="t1"/><testcase name="t2"/></r>"#;
        let v = xpath(xml, "//testcase/@name").expect("valid xpath");
        assert_eq!(v, serde_json::json!(["t1", "t2"]));
    }

    #[test]
    fn xpath_returns_scalars_for_count() {
        let xml = r#"<r><x/><x/><x/></r>"#;
        let v = xpath(xml, "count(//x)").expect("valid xpath");
        assert_eq!(v, serde_json::json!(3.0));
    }
}
