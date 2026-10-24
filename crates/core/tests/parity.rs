//! The cleanup against parity.json: what the original Python version of
//! localflow returned for about a thousand cases (the dictation turn,
//! headings, file references, learning's diffs), kept as the reference now
//! that localflow is Rust only.

use localflow_core::difflib::{SequenceMatcher, Tag};
use localflow_core::{Part, eval};
use serde_json::{Value, json};

fn fixture() -> Value {
    serde_json::from_str(include_str!("parity.json")).unwrap()
}

fn strs(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().map(|s| s.as_str().unwrap().to_string()).collect()).unwrap_or_default()
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap()
}

#[test]
fn user_turn() {
    for c in fixture()["user_turn"].as_array().unwrap() {
        assert_eq!(localflow_core::user_turn(s(&c[0]), &strs(&c[1])), s(&c[2]));
    }
}

#[test]
fn markdown_headings() {
    for c in fixture()["headings"].as_array().unwrap() {
        assert_eq!(localflow_core::markdown_headings(s(&c[0])), s(&c[1]), "input: {:?}", c[0]);
    }
}

#[test]
fn plausible() {
    for c in fixture()["plausible"].as_array().unwrap() {
        assert_eq!(localflow_core::plausible(s(&c[0]), s(&c[1]), &strs(&c[2])), c[3].as_bool().unwrap(), "case: {c}");
    }
}

#[test]
fn file_refs() {
    for c in fixture()["file_refs"].as_array().unwrap() {
        let parts: Vec<Value> = localflow_core::file_refs(s(&c[0]), &strs(&c[1]))
            .into_iter()
            .map(|p| match p {
                Part::Text(t) => json!(["text", t]),
                Part::File(f) => json!(["file", f]),
            })
            .collect();
        assert_eq!(Value::Array(parts), c[2], "case: {c}");
    }
}

#[test]
fn corrections() {
    for c in fixture()["corrections"].as_array().unwrap() {
        let got = localflow_core::corrections(s(&c[0]), s(&c[1]))
            .map(|pairs| pairs.into_iter().map(|(h, m)| json!([h, m])).collect::<Vec<_>>());
        assert_eq!(got.map_or(Value::Null, Value::Array), c[2], "case: {c}");
    }
}

#[test]
fn edited_region() {
    for c in fixture()["edited_region"].as_array().unwrap() {
        assert_eq!(localflow_core::edited_region(s(&c[0]), s(&c[1])).map_or(Value::Null, Value::from), c[2], "case: {c}");
    }
}

#[test]
fn difflib() {
    for c in fixture()["difflib"].as_array().unwrap() {
        let (a, b): (Vec<i64>, Vec<i64>) = (serde_json::from_value(c["a"].clone()).unwrap(),
                                            serde_json::from_value(c["b"].clone()).unwrap());
        let sm = SequenceMatcher::new(&a, &b);
        let blocks: Vec<Value> = sm.matching_blocks().iter().map(|m| json!([m.a, m.b, m.size])).collect();
        assert_eq!(Value::Array(blocks), c["blocks"], "case: {c}");
        let ops: Vec<Value> = sm.opcodes().iter().map(|o| {
            let tag = match o.tag {
                Tag::Equal => "equal",
                Tag::Replace => "replace",
                Tag::Delete => "delete",
                Tag::Insert => "insert",
            };
            json!([tag, o.i1, o.i2, o.j1, o.j2])
        }).collect();
        assert_eq!(Value::Array(ops), c["opcodes"], "case: {c}");
    }
}

#[test]
fn eval_grading() {
    for c in fixture()["grade"].as_array().unwrap() {
        assert_eq!(eval::grade(s(&c[0]), s(&c[1])), c[2].as_bool().unwrap(), "case: {c}");
    }
    assert!(!eval::cases().is_empty());
}
