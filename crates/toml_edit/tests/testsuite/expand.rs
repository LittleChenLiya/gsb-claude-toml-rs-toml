use toml_edit::{DocumentMut, ExpandError, Key};

fn key(s: &str) -> Key {
    s.parse().unwrap()
}

fn path(keys: &[&str]) -> Vec<Key> {
    keys.iter().map(|s| key(s)).collect()
}

#[test]
fn basic_expansion() {
    let input = "a = { b = { c = 1 }, d = 2 }\n";
    let mut doc = input.parse::<DocumentMut>().unwrap();
    // Expanding `a` recursively expands its inline subtable `b`.
    let paths: &[&[Key]] = &[&path(&["a"])];
    doc.expand_inline_tables(paths).unwrap();
    let expected = "[a]\nd = 2\n\n[a.b]\nc = 1\n";
    assert_eq!(doc.to_string(), expected);
    let reparsed = doc.to_string().parse::<DocumentMut>().unwrap();
    assert_eq!(reparsed["a"]["d"].as_integer().unwrap(), 2);
    assert_eq!(reparsed["a"]["b"]["c"].as_integer().unwrap(), 1);
}

#[test]
fn expand_top_level_recursively() {
    let input = "a = { b = { c = 1 }, d = 2 }\n";
    let mut doc = input.parse::<DocumentMut>().unwrap();
    let paths: &[&[Key]] = &[&path(&["a"])];
    doc.expand_inline_tables(paths).unwrap();
    let expected = "[a]\nd = 2\n\n[a.b]\nc = 1\n";
    assert_eq!(doc.to_string(), expected);
}

#[test]
fn literal_key_not_split() {
    let input = "a = { \"b.c\" = 1, d = 2 }\n";
    let mut doc = input.parse::<DocumentMut>().unwrap();
    let paths: &[&[Key]] = &[&path(&["a"])];
    doc.expand_inline_tables(paths).unwrap();
    let expected = "[a]\n\"b.c\" = 1\nd = 2\n";
    assert_eq!(doc.to_string(), expected);
    let reparsed = doc.to_string().parse::<DocumentMut>().unwrap();
    assert_eq!(reparsed["a"]["b.c"].as_integer().unwrap(), 1);
    assert_eq!(reparsed["a"]["d"].as_integer().unwrap(), 2);
}

#[test]
fn array_inline_table_rejected() {
    let input = "a = [{ b = 1 }]\n";
    let mut doc = input.parse::<DocumentMut>().unwrap();
    let paths: &[&[Key]] = &[&path(&["a"])];
    let err = doc.expand_inline_tables(paths).unwrap_err();
    assert_eq!(err, ExpandError::InArray);
    assert_eq!(doc.to_string(), input);
}

#[test]
fn nested_array_inline_table_rejected() {
    let input = "a = [{ b = { c = 1 } }]\n";
    let mut doc = input.parse::<DocumentMut>().unwrap();
    let paths: &[&[Key]] = &[&path(&["a", "b"])];
    let err = doc.expand_inline_tables(paths).unwrap_err();
    assert_eq!(err, ExpandError::InArray);
    assert_eq!(doc.to_string(), input);
}

#[test]
fn multiple_paths() {
    let input = "a = { x = 1 }\nb = { y = 2 }\n";
    let mut doc = input.parse::<DocumentMut>().unwrap();
    let paths: &[&[Key]] = &[&path(&["a"]), &path(&["b"])];
    doc.expand_inline_tables(paths).unwrap();
    let expected = "[a]\nx = 1\n\n[b]\ny = 2\n";
    assert_eq!(doc.to_string(), expected);
    let reparsed = doc.to_string().parse::<DocumentMut>().unwrap();
    assert_eq!(reparsed["a"]["x"].as_integer().unwrap(), 1);
    assert_eq!(reparsed["b"]["y"].as_integer().unwrap(), 2);
}

#[test]
fn empty_inline_table() {
    let input = "a = {}\n";
    let mut doc = input.parse::<DocumentMut>().unwrap();
    let paths: &[&[Key]] = &[&path(&["a"])];
    doc.expand_inline_tables(paths).unwrap();
    // An empty regular table is visible.
    assert_eq!(doc.to_string(), "[a]\n");
    let reparsed = doc.to_string().parse::<DocumentMut>().unwrap();
    assert!(reparsed["a"].as_table_like().is_some());
}

#[test]
fn nested_empty_inline_table() {
    let input = "a = { b = {} }\n";
    let mut doc = input.parse::<DocumentMut>().unwrap();
    let paths: &[&[Key]] = &[&path(&["a"])];
    doc.expand_inline_tables(paths).unwrap();
    let expected = "[a]\n\n[a.b]\n";
    assert_eq!(doc.to_string(), expected);
    let reparsed = doc.to_string().parse::<DocumentMut>().unwrap();
    assert!(reparsed["a"].as_table_like().is_some());
    assert!(reparsed["a"]["b"].as_table_like().is_some());
}

#[test]
fn unselected_comments_preserved() {
    let input = "# top comment\na = { x = 1 } # inline comment\n# trailing comment\n";
    let mut doc = input.parse::<DocumentMut>().unwrap();
    let paths: &[&[Key]] = &[&path(&["a"])];
    doc.expand_inline_tables(paths).unwrap();
    let expected = "# top comment\n[a]\nx = 1 # inline comment\n# trailing comment\n";
    assert_eq!(doc.to_string(), expected);
}

#[test]
fn path_not_found() {
    let input = "a = { x = 1 }\n";
    let mut doc = input.parse::<DocumentMut>().unwrap();
    let paths: &[&[Key]] = &[&path(&["b"])];
    let err = doc.expand_inline_tables(paths).unwrap_err();
    assert_eq!(err, ExpandError::NotFound);
    assert_eq!(doc.to_string(), input);
}

#[test]
fn path_not_inline_table() {
    let input = "a = 1\n";
    let mut doc = input.parse::<DocumentMut>().unwrap();
    let paths: &[&[Key]] = &[&path(&["a"])];
    let err = doc.expand_inline_tables(paths).unwrap_err();
    assert_eq!(err, ExpandError::NotInlineTable);
    assert_eq!(doc.to_string(), input);
}

#[test]
fn path_regular_table_rejected() {
    let input = "[a]\nx = 1\n";
    let mut doc = input.parse::<DocumentMut>().unwrap();
    let paths: &[&[Key]] = &[&path(&["a"])];
    let err = doc.expand_inline_tables(paths).unwrap_err();
    assert_eq!(err, ExpandError::NotInlineTable);
    assert_eq!(doc.to_string(), input);
}

#[test]
fn array_of_tables_not_converted() {
    let input = "[[a]]\nx = 1\n";
    let mut doc = input.parse::<DocumentMut>().unwrap();
    let paths: &[&[Key]] = &[&path(&["a"])];
    let err = doc.expand_inline_tables(paths).unwrap_err();
    assert_eq!(err, ExpandError::NotInlineTable);
    assert_eq!(doc.to_string(), input);
}

#[test]
fn regular_array_not_converted() {
    let input = "a = [1, 2, 3]\n";
    let mut doc = input.parse::<DocumentMut>().unwrap();
    let paths: &[&[Key]] = &[&path(&["a"])];
    let err = doc.expand_inline_tables(paths).unwrap_err();
    assert_eq!(err, ExpandError::NotInlineTable);
    assert_eq!(doc.to_string(), input);
}

#[test]
fn atomic_batch_rejection() {
    let input = "a = { x = 1 }\nb = 2\n";
    let mut doc = input.parse::<DocumentMut>().unwrap();
    let paths: &[&[Key]] = &[&path(&["a"]), &path(&["b"])];
    let err = doc.expand_inline_tables(paths).unwrap_err();
    assert_eq!(err, ExpandError::NotInlineTable);
    assert_eq!(doc.to_string(), input);
}

#[test]
fn inline_table_with_array_values() {
    let input = "a = { xs = [1, 2, 3], ys = [\"a\", \"b\"] }\n";
    let mut doc = input.parse::<DocumentMut>().unwrap();
    let paths: &[&[Key]] = &[&path(&["a"])];
    doc.expand_inline_tables(paths).unwrap();
    let expected = "[a]\nxs = [1, 2, 3]\nys = [\"a\", \"b\"]\n";
    assert_eq!(doc.to_string(), expected);
    let reparsed = doc.to_string().parse::<DocumentMut>().unwrap();
    assert_eq!(reparsed["a"]["xs"][0].as_integer().unwrap(), 1);
    assert_eq!(reparsed["a"]["ys"][0].as_str().unwrap(), "a");
}

#[test]
fn deeply_nested_inline_tables() {
    let input = "a = { b = { c = { d = 1 } } }\n";
    let mut doc = input.parse::<DocumentMut>().unwrap();
    let paths: &[&[Key]] = &[&path(&["a"])];
    doc.expand_inline_tables(paths).unwrap();
    let expected = "[a]\n\n[a.b]\n\n[a.b.c]\nd = 1\n";
    assert_eq!(doc.to_string(), expected);
    let reparsed = doc.to_string().parse::<DocumentMut>().unwrap();
    assert_eq!(reparsed["a"]["b"]["c"]["d"].as_integer().unwrap(), 1);
}

#[test]
fn expand_nested_path_in_regular_table() {
    let input = "[a]\nb = { c = 1 }\n";
    let mut doc = input.parse::<DocumentMut>().unwrap();
    let paths: &[&[Key]] = &[&path(&["a", "b"])];
    doc.expand_inline_tables(paths).unwrap();
    let expected = "[a]\n\n[a.b]\nc = 1\n";
    assert_eq!(doc.to_string(), expected);
    let reparsed = doc.to_string().parse::<DocumentMut>().unwrap();
    assert_eq!(reparsed["a"]["b"]["c"].as_integer().unwrap(), 1);
}

#[test]
fn original_document_not_modified() {
    let input = "a = { b = { c = 1 }, d = 2 }\n";
    let doc = input.parse::<toml_edit::Document<String>>().unwrap();
    let original = doc.to_string();
    let mut mutable = doc.into_mut();
    let paths: &[&[Key]] = &[&path(&["a"])];
    mutable.expand_inline_tables(paths).unwrap();
    assert_eq!(original, input);
}
