use toml_edit::{DocumentMut, ExpandInlineError};

fn parse(raw: &str) -> DocumentMut {
    raw.parse::<DocumentMut>().unwrap()
}

#[test]
fn expand_basic() {
    let doc = parse("a={b={c=1},d=2}");
    let expanded = doc.expand_inline_tables([["a"]]).unwrap();
    assert_eq!(expanded.to_string(), "[a]\nd = 2\n\n[a.b]\nc = 1\n");
    // The serialized result parses back to the same data
    let back = parse(&expanded.to_string());
    assert_eq!(back["a"]["b"]["c"].as_integer(), Some(1));
    assert_eq!(back["a"]["d"].as_integer(), Some(2));
    // The original document is unchanged
    assert_eq!(doc.to_string(), "a={b={c=1},d=2}\n");
}

#[test]
fn expand_literal_dotted_key_stays_single_key() {
    let doc = parse("\"a.b\" = {c = 1}\nother = 2\n");
    let expanded = doc.expand_inline_tables([["a.b"]]).unwrap();
    assert_eq!(expanded.to_string(), "other = 2\n\n[\"a.b\"]\nc = 1\n");
    let back = parse(&expanded.to_string());
    // `"a.b"` is one literal key, not a nested `a` -> `b` path
    assert!(back.get("a").is_none());
    assert_eq!(back["a.b"]["c"].as_integer(), Some(1));
    assert_eq!(back["other"].as_integer(), Some(2));
}

#[test]
fn expand_multiple_paths() {
    let doc = parse("a = {x = 1}\nb = {y = 2}\nc = {z = 3}\n");
    let expanded = doc.expand_inline_tables([["a"], ["b"]]).unwrap();
    assert_eq!(
        expanded.to_string(),
        "c = {z = 3}\n\n[a]\nx = 1\n\n[b]\ny = 2\n"
    );
    let back = parse(&expanded.to_string());
    assert_eq!(back["a"]["x"].as_integer(), Some(1));
    assert_eq!(back["b"]["y"].as_integer(), Some(2));
    // Unselected inline table is untouched
    assert_eq!(back["c"]["z"].as_integer(), Some(3));
    assert!(back["c"].is_inline_table());
}

#[test]
fn expand_nested_and_multiple_paths() {
    let doc = parse("a = {b = {c = 1}, d = 2}\ne = {f = 3}\n");
    let expanded = doc.expand_inline_tables([["a"], ["e"]]).unwrap();
    assert_eq!(
        expanded.to_string(),
        "[a]\nd = 2\n\n[a.b]\nc = 1\n\n[e]\nf = 3\n"
    );
    let back = parse(&expanded.to_string());
    assert_eq!(back["a"]["b"]["c"].as_integer(), Some(1));
    assert_eq!(back["a"]["d"].as_integer(), Some(2));
    assert_eq!(back["e"]["f"].as_integer(), Some(3));
}

#[test]
fn expand_empty_inline_table() {
    let doc = parse("a = {}\n");
    let expanded = doc.expand_inline_tables([["a"]]).unwrap();
    assert_eq!(expanded.to_string(), "[a]\n");
    let back = parse(&expanded.to_string());
    let table = back["a"].as_table().unwrap();
    assert!(table.is_empty());
}

#[test]
fn expand_preserves_unselected_comments() {
    let raw = "\
# leading comment on a
a = {b = 1} # trailing comment on a
# comment on keep
keep = {x = 1} # stays inline
";
    let doc = parse(raw);
    let expanded = doc.expand_inline_tables([["a"]]).unwrap();
    assert_eq!(
        expanded.to_string(),
        "\
# comment on keep
keep = {x = 1} # stays inline
# leading comment on a
[a] # trailing comment on a
b = 1
"
    );
    // The unselected `keep` line, comments included, is untouched
    let back = parse(&expanded.to_string());
    assert!(back["keep"].is_inline_table());
    assert_eq!(back["keep"]["x"].as_integer(), Some(1));
}

#[test]
fn expand_nested_path_expands_inline_ancestors() {
    let doc = parse("a = {b = {c = 1}}\n");
    let expanded = doc.expand_inline_tables([["a", "b"]]).unwrap();
    assert_eq!(expanded.to_string(), "[a]\n\n[a.b]\nc = 1\n");
    let back = parse(&expanded.to_string());
    assert_eq!(back["a"]["b"]["c"].as_integer(), Some(1));
}

#[test]
fn expand_keeps_dotted_keys_dotted() {
    let doc = parse("a = {b.c = 1}\n");
    let expanded = doc.expand_inline_tables([["a"]]).unwrap();
    assert_eq!(expanded.to_string(), "[a]\nb.c = 1\n");
    let back = parse(&expanded.to_string());
    assert_eq!(back["a"]["b"]["c"].as_integer(), Some(1));
}

#[test]
fn expand_keeps_arrays_inline() {
    let doc = parse("a = {arr = [{x = 1}], n = 2}\n");
    let expanded = doc.expand_inline_tables([["a"]]).unwrap();
    // Inline tables inside arrays are out of scope and stay inline
    assert_eq!(expanded.to_string(), "[a]\narr = [{x = 1}]\nn = 2\n");
    let back = parse(&expanded.to_string());
    assert_eq!(back["a"]["arr"][0]["x"].as_integer(), Some(1));
    assert_eq!(back["a"]["n"].as_integer(), Some(2));
}

#[test]
fn expand_standard_table_is_noop() {
    let doc = parse("[a]\nb = 1\n");
    let expanded = doc.expand_inline_tables([["a"]]).unwrap();
    assert_eq!(expanded.to_string(), "[a]\nb = 1\n");
}

#[test]
fn expand_rejects_inline_table_in_array() {
    let doc = parse("arr = [{x = 1}]\n");
    let err = doc.expand_inline_tables([["arr"]]).unwrap_err();
    assert_eq!(
        err,
        ExpandInlineError::InvalidPath {
            path: vec!["arr".to_owned()],
            segment: 0,
            found: Some("array"),
        }
    );
    // The array was not converted to an array of tables or anything else
    assert_eq!(doc.to_string(), "arr = [{x = 1}]\n");
}

#[test]
fn expand_rejects_array_of_tables() {
    let doc = parse("[[t]]\nx = 1\n");
    let err = doc.expand_inline_tables([["t"]]).unwrap_err();
    assert_eq!(
        err,
        ExpandInlineError::InvalidPath {
            path: vec!["t".to_owned()],
            segment: 0,
            found: Some("array of tables"),
        }
    );
    assert_eq!(doc.to_string(), "[[t]]\nx = 1\n");
}

#[test]
fn expand_rejects_scalar_and_missing_paths() {
    let doc = parse("a = 1\n");
    let err = doc.expand_inline_tables([["a"]]).unwrap_err();
    assert_eq!(
        err,
        ExpandInlineError::InvalidPath {
            path: vec!["a".to_owned()],
            segment: 0,
            found: Some("integer"),
        }
    );
    let err = doc.expand_inline_tables([["missing"]]).unwrap_err();
    assert_eq!(
        err,
        ExpandInlineError::InvalidPath {
            path: vec!["missing".to_owned()],
            segment: 0,
            found: None,
        }
    );
    let err = doc.expand_inline_tables([["a", "b"]]).unwrap_err();
    assert_eq!(
        err,
        ExpandInlineError::InvalidPath {
            path: vec!["a".to_owned(), "b".to_owned()],
            segment: 0,
            found: Some("integer"),
        }
    );
}

#[test]
fn expand_rejects_empty_path() {
    let doc = parse("a = {b = 1}\n");
    let err = doc.expand_inline_tables([&[] as &[&str]]).unwrap_err();
    assert_eq!(err, ExpandInlineError::EmptyPath);
}

#[test]
fn expand_batch_is_atomic() {
    let doc = parse("ok = {x = 1}\n");
    let err = doc.expand_inline_tables([["ok"], ["missing"]]).unwrap_err();
    assert_eq!(
        err,
        ExpandInlineError::InvalidPath {
            path: vec!["missing".to_owned()],
            segment: 0,
            found: None,
        }
    );
    // The whole batch was rejected: nothing was expanded
    assert_eq!(doc.to_string(), "ok = {x = 1}\n");
}

#[test]
fn expand_error_display() {
    let doc = parse("a = 1\n");
    let err = doc.expand_inline_tables([["a"]]).unwrap_err();
    assert_eq!(
        err.to_string(),
        "cannot expand inline table at path `a`: segment `a` resolved to integer, not an inline table"
    );
    let err = doc.expand_inline_tables([["a", "b"]]).unwrap_err();
    assert_eq!(
        err.to_string(),
        "cannot expand inline table at path `a.b`: segment `a` resolved to integer, not an inline table"
    );
}

#[test]
fn expand_roundtrip_preserves_data() {
    let raw = "\
title = \"demo\"
point = {x = 1, y = {z = 2}, tags = [\"a\", \"b\"]}
other = {nested = {deep = {leaf = true}}}
";
    let doc = parse(raw);
    let expanded = doc.expand_inline_tables([["point"], ["other"]]).unwrap();
    let back = parse(&expanded.to_string());
    assert_eq!(back["title"].as_str(), Some("demo"));
    assert_eq!(back["point"]["x"].as_integer(), Some(1));
    assert_eq!(back["point"]["y"]["z"].as_integer(), Some(2));
    assert_eq!(back["point"]["tags"][1].as_str(), Some("b"));
    assert_eq!(
        back["other"]["nested"]["deep"]["leaf"].as_bool(),
        Some(true)
    );
}
