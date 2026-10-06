//! Tests for `toml_edit::ConversionPlan` (table <-> inline table conversion).
//!
//! Run with: cargo test -p toml_edit --test testsuite convert

use toml_edit::{ConversionPlan, ConvertError, DocumentMut, Item, Value};

/// Logical content of a document as `(key path, scalar)` pairs, in order.
///
/// Table spelling (standard vs inline vs dotted) is ignored on purpose: two
/// documents with the same logical values produce the same output here.
fn logical(doc: &DocumentMut) -> Vec<(Vec<String>, String)> {
    let mut out = Vec::new();
    collect_item(doc.as_item(), &mut Vec::new(), &mut out);
    // Key order is not semantically significant in TOML
    out.sort();
    out
}

fn collect_item(item: &Item, path: &mut Vec<String>, out: &mut Vec<(Vec<String>, String)>) {
    match item {
        Item::None => {}
        Item::Table(table) => {
            for (key, item) in table.iter() {
                path.push(key.to_owned());
                collect_item(item, path, out);
                path.pop();
            }
        }
        Item::ArrayOfTables(tables) => {
            for (index, table) in tables.iter().enumerate() {
                path.push(index.to_string());
                collect_item(&Item::Table(table.clone()), path, out);
                path.pop();
            }
        }
        Item::Value(value) => collect_value(value, path, out),
    }
}

fn collect_value(value: &Value, path: &mut Vec<String>, out: &mut Vec<(Vec<String>, String)>) {
    match value {
        Value::Array(array) => {
            for (index, element) in array.iter().enumerate() {
                path.push(index.to_string());
                collect_value(element, path, out);
                path.pop();
            }
        }
        Value::InlineTable(table) => {
            for (key, element) in table.iter() {
                path.push(key.to_owned());
                collect_value(element, path, out);
                path.pop();
            }
        }
        _ => out.push((path.clone(), value.to_string().trim().to_owned())),
    }
}

#[track_caller]
fn assert_conversion(source: &str, plan: &ConversionPlan, expected: &str) {
    let doc: DocumentMut = source.parse().unwrap();
    let before = logical(&doc);

    let converted = plan.apply(&doc).unwrap();

    // The source document is never modified
    assert_eq!(doc.to_string(), source, "source document was modified");
    // The rendering is deterministic
    assert_eq!(converted.to_string(), expected, "unexpected rendering");
    // The converted output parses to the same logical values
    let reparsed: DocumentMut = converted.to_string().parse().unwrap();
    assert_eq!(before, logical(&reparsed), "values changed");
}

#[track_caller]
fn assert_rejected(source: &str, plan: &ConversionPlan, expected: ConvertError) {
    let doc: DocumentMut = source.parse().unwrap();
    let err = plan.apply(&doc).unwrap_err();
    assert_eq!(err, expected);
    // A rejected plan leaves the source document untouched
    assert_eq!(doc.to_string(), source, "source document was modified");
}

#[test]
fn table_to_inline_simple() {
    let source = "[point]\nx = 1\ny = 2\n";
    let mut plan = ConversionPlan::new();
    plan.to_inline(["point"]).unwrap();
    assert_conversion(source, &plan, "point = { x = 1, y = 2 }\n");
}

#[test]
fn table_to_inline_preserves_dotted_and_quoted_keys() {
    // `a.b` is a dotted key path and must stay dotted; `"x.y"` is a single
    // quoted key and must not be split apart.
    let source = "[t]\na.b = 1\n\"x.y\" = 2\n";
    let mut plan = ConversionPlan::new();
    plan.to_inline(["t"]).unwrap();
    assert_conversion(source, &plan, "t = { a.b = 1, \"x.y\" = 2 }\n");
}

#[test]
fn table_to_inline_deeply_dotted_keys() {
    let source = "[t]\na.b.c = 1\n";
    let mut plan = ConversionPlan::new();
    plan.to_inline(["t"]).unwrap();
    assert_conversion(source, &plan, "t = { a.b.c = 1 }\n");
}

#[test]
fn table_to_inline_nested_values() {
    // `#` inside a string is not a comment and must not cause a rejection
    let source = "[t]\ns = \"a # b\"\nlist = [\n  1,\n  2,\n]\nnested = { x = 1 }\n";
    let mut plan = ConversionPlan::new();
    plan.to_inline(["t"]).unwrap();
    assert_conversion(
        source,
        &plan,
        "t = { s = \"a # b\", list = [1, 2], nested = { x = 1 } }\n",
    );
}

#[test]
fn table_to_inline_empty() {
    let source = "[t]\n";
    let mut plan = ConversionPlan::new();
    plan.to_inline(["t"]).unwrap();
    assert_conversion(source, &plan, "t = {}\n");
}

#[test]
fn table_to_inline_nested_path() {
    let source = "[a]\nb = 1\n[a.t]\nx = 2\n";
    let mut plan = ConversionPlan::new();
    plan.to_inline(["a", "t"]).unwrap();
    assert_conversion(source, &plan, "[a]\nb = 1\nt = { x = 2 }\n");
}

#[test]
fn table_to_inline_header_comment_rejected() {
    let source = "# header comment\n[t]\na = 1\n";
    let mut plan = ConversionPlan::new();
    plan.to_inline(["t"]).unwrap();
    assert_rejected(
        source,
        &plan,
        ConvertError::UnmovableComment {
            path: vec!["t".to_owned()],
        },
    );
}

#[test]
fn table_to_inline_header_trailing_comment_rejected() {
    let source = "[t] # tail\na = 1\n";
    let mut plan = ConversionPlan::new();
    plan.to_inline(["t"]).unwrap();
    assert_rejected(
        source,
        &plan,
        ConvertError::UnmovableComment {
            path: vec!["t".to_owned()],
        },
    );
}

#[test]
fn table_to_inline_key_comment_rejected() {
    let source = "[t]\n# key comment\na = 1\n";
    let mut plan = ConversionPlan::new();
    plan.to_inline(["t"]).unwrap();
    assert_rejected(
        source,
        &plan,
        ConvertError::UnmovableComment {
            path: vec!["t".to_owned(), "a".to_owned()],
        },
    );
}

#[test]
fn table_to_inline_value_comment_rejected() {
    let source = "[t]\na = 1 # value comment\n";
    let mut plan = ConversionPlan::new();
    plan.to_inline(["t"]).unwrap();
    assert_rejected(
        source,
        &plan,
        ConvertError::UnmovableComment {
            path: vec!["t".to_owned(), "a".to_owned()],
        },
    );
}

#[test]
fn table_to_inline_array_comment_rejected() {
    let source = "[t]\na = [\n  1, # element\n  2,\n]\n";
    let mut plan = ConversionPlan::new();
    plan.to_inline(["t"]).unwrap();
    assert_rejected(
        source,
        &plan,
        ConvertError::UnmovableComment {
            path: vec!["t".to_owned(), "a".to_owned()],
        },
    );
}

#[test]
fn table_to_inline_dotted_key_comment_rejected() {
    let source = "[t]\na.b = 1 # comment\n";
    let mut plan = ConversionPlan::new();
    plan.to_inline(["t"]).unwrap();
    assert_rejected(
        source,
        &plan,
        ConvertError::UnmovableComment {
            path: vec!["t".to_owned(), "a".to_owned(), "b".to_owned()],
        },
    );
}

#[test]
fn table_to_inline_child_table_rejected() {
    let source = "[t]\na = 1\n[t.u]\nb = 2\n";
    let mut plan = ConversionPlan::new();
    plan.to_inline(["t"]).unwrap();
    assert_rejected(
        source,
        &plan,
        ConvertError::ChildTables {
            path: vec!["t".to_owned(), "u".to_owned()],
        },
    );
}

#[test]
fn table_to_inline_child_array_of_tables_rejected() {
    let source = "[t]\na = 1\n[[t.u]]\nb = 2\n";
    let mut plan = ConversionPlan::new();
    plan.to_inline(["t"]).unwrap();
    assert_rejected(
        source,
        &plan,
        ConvertError::ChildTables {
            path: vec!["t".to_owned(), "u".to_owned()],
        },
    );
}

#[test]
fn table_to_inline_implicit_table_rejected() {
    let source = "[t.u]\nb = 1\n";
    let mut plan = ConversionPlan::new();
    plan.to_inline(["t"]).unwrap();
    assert_rejected(
        source,
        &plan,
        ConvertError::ImplicitTable {
            path: vec!["t".to_owned()],
        },
    );
}

#[test]
fn table_to_inline_dotted_proxy_rejected() {
    let source = "[t]\na.b = 1\n";
    let mut plan = ConversionPlan::new();
    plan.to_inline(["t", "a"]).unwrap();
    assert_rejected(
        source,
        &plan,
        ConvertError::DottedTable {
            path: vec!["t".to_owned(), "a".to_owned()],
        },
    );
}

#[test]
fn missing_target_rejected() {
    let source = "[t]\na = 1\n";
    let mut plan = ConversionPlan::new();
    plan.to_inline(["missing"]).unwrap();
    assert_rejected(
        source,
        &plan,
        ConvertError::NotFound {
            path: vec!["missing".to_owned()],
        },
    );
}

#[test]
fn wrong_kind_rejected() {
    let source = "a = 1\n[t]\nb = 2\n";
    let mut plan = ConversionPlan::new();
    plan.to_inline(["a"]).unwrap();
    assert_rejected(
        source,
        &plan,
        ConvertError::WrongKind {
            path: vec!["a".to_owned()],
            expected: "table",
            found: "integer",
        },
    );

    let mut plan = ConversionPlan::new();
    plan.to_table(["t"]).unwrap();
    assert_rejected(
        source,
        &plan,
        ConvertError::WrongKind {
            path: vec!["t".to_owned()],
            expected: "inline table",
            found: "table",
        },
    );
}

#[test]
fn inline_to_table_simple() {
    let source = "point = { x = 1, y = 2 }\n";
    let mut plan = ConversionPlan::new();
    plan.to_table(["point"]).unwrap();
    assert_conversion(source, &plan, "[point]\nx = 1\ny = 2\n");
}

#[test]
fn inline_to_table_trailing_comment_migrates_to_header() {
    let source = "point = { x = 1 } # note\n";
    let mut plan = ConversionPlan::new();
    plan.to_table(["point"]).unwrap();
    assert_conversion(source, &plan, "[point] # note\nx = 1\n");
}

#[test]
fn inline_to_table_comment_above_key_stays() {
    let source = "# keep me\npoint = { x = 1 }\n";
    let mut plan = ConversionPlan::new();
    plan.to_table(["point"]).unwrap();
    assert_conversion(source, &plan, "# keep me\n[point]\nx = 1\n");
}

#[test]
fn inline_to_table_inner_comment_rejected() {
    let source = "point = {\n  x = 1, # note\n}\n";
    let mut plan = ConversionPlan::new();
    plan.to_table(["point"]).unwrap();
    assert_rejected(
        source,
        &plan,
        ConvertError::UnmovableComment {
            path: vec!["point".to_owned()],
        },
    );
}

#[test]
fn inline_to_table_preserves_dotted_and_quoted_keys() {
    let source = "t = { a.b = 1, \"x.y\" = 2 }\n";
    let mut plan = ConversionPlan::new();
    plan.to_table(["t"]).unwrap();
    assert_conversion(source, &plan, "[t]\na.b = 1\n\"x.y\" = 2\n");
}

#[test]
fn inline_to_table_nested_inline_stays_inline() {
    // Only the explicitly selected table is converted
    let source = "t = { u = { x = 1 }, y = 2 }\n";
    let mut plan = ConversionPlan::new();
    plan.to_table(["t"]).unwrap();
    assert_conversion(source, &plan, "[t]\nu = { x = 1 }\ny = 2\n");
}

#[test]
fn inline_to_table_dotted_proxy_rejected() {
    let source = "t = { a.b = 1 }\n";
    let mut plan = ConversionPlan::new();
    plan.to_table(["t", "a"]).unwrap();
    assert_rejected(
        source,
        &plan,
        ConvertError::DottedTable {
            path: vec!["t".to_owned(), "a".to_owned()],
        },
    );
}

#[test]
fn inline_to_table_nested_in_inline_rejected() {
    let source = "a = { b = { c = 1 } }\n";
    let mut plan = ConversionPlan::new();
    plan.to_table(["a", "b"]).unwrap();
    assert_rejected(
        source,
        &plan,
        ConvertError::UnsupportedNesting {
            path: vec!["a".to_owned(), "b".to_owned()],
        },
    );
}

#[test]
fn inline_to_table_nested_path() {
    let source = "[a]\nt = { x = 1 }\n";
    let mut plan = ConversionPlan::new();
    plan.to_table(["a", "t"]).unwrap();
    assert_conversion(source, &plan, "[a]\n\n[a.t]\nx = 1\n");
}

#[test]
fn empty_path_rejected() {
    let mut plan = ConversionPlan::new();
    assert_eq!(plan.to_inline(Vec::<&str>::new()).unwrap_err(), ConvertError::EmptyPath);
    assert_eq!(plan.to_table(Vec::<&str>::new()).unwrap_err(), ConvertError::EmptyPath);
}

#[test]
fn conflicting_targets_rejected_first() {
    // Same path twice
    let mut plan = ConversionPlan::new();
    plan.to_inline(["a"]).unwrap();
    assert_eq!(
        plan.to_inline(["a"]).unwrap_err(),
        ConvertError::Conflict {
            existing: vec!["a".to_owned()],
            new: vec!["a".to_owned()],
        }
    );

    // Parent and child overlap, regardless of direction
    let mut plan = ConversionPlan::new();
    plan.to_inline(["a"]).unwrap();
    assert_eq!(
        plan.to_table(["a", "b"]).unwrap_err(),
        ConvertError::Conflict {
            existing: vec!["a".to_owned()],
            new: vec!["a".to_owned(), "b".to_owned()],
        }
    );

    // Child then parent
    let mut plan = ConversionPlan::new();
    plan.to_table(["a", "b"]).unwrap();
    assert_eq!(
        plan.to_inline(["a"]).unwrap_err(),
        ConvertError::Conflict {
            existing: vec!["a".to_owned(), "b".to_owned()],
            new: vec!["a".to_owned()],
        }
    );

    // Siblings do not overlap
    let mut plan = ConversionPlan::new();
    plan.to_inline(["a", "b"]).unwrap();
    plan.to_inline(["a", "c"]).unwrap();
    plan.to_inline(["ab"]).unwrap();
    assert_eq!(plan.len(), 3);
}

#[test]
fn multi_target_plan_three_tables() {
    let source = "\
title = \"shapes\"
style = { color = \"red\", width = 2 }

[point]
x = 1
y = 2

[server]
host = \"localhost\"
ports = [8000, 8001]
";
    let expected = "\
title = \"shapes\"
point = { x = 1, y = 2 }
server = { host = \"localhost\", ports = [8000, 8001] }

[style]
color = \"red\"
width = 2
";
    let mut plan = ConversionPlan::new();
    plan.to_inline(["point"]).unwrap();
    plan.to_inline(["server"]).unwrap();
    plan.to_table(["style"]).unwrap();
    assert_conversion(source, &plan, expected);
}

#[test]
fn only_selected_tables_are_converted() {
    let source = "\
[picked]
x = 1

[other]
y = 2
";
    let expected = "\
picked = { x = 1 }

[other]
y = 2
";
    let mut plan = ConversionPlan::new();
    plan.to_inline(["picked"]).unwrap();
    assert_conversion(source, &plan, expected);
}

#[test]
fn roundtrip_returns_same_values() {
    let source = "[t]\na = 1\nb.c = \"x\"\nd = [1, 2]\n";
    let doc: DocumentMut = source.parse().unwrap();

    let mut to_inline = ConversionPlan::new();
    to_inline.to_inline(["t"]).unwrap();
    let inlined = to_inline.apply(&doc).unwrap();
    assert_eq!(inlined.to_string(), "t = { a = 1, b.c = \"x\", d = [1, 2] }\n");

    let mut to_table = ConversionPlan::new();
    to_table.to_table(["t"]).unwrap();
    let expanded = to_table.apply(&inlined).unwrap();
    let reparsed: DocumentMut = expanded.to_string().parse().unwrap();
    assert_eq!(logical(&doc), logical(&reparsed));
}

#[test]
fn plan_can_be_reused_and_source_stays_untouched() {
    let source = "[t]\na = 1\n";
    let doc: DocumentMut = source.parse().unwrap();
    let mut plan = ConversionPlan::new();
    plan.to_inline(["t"]).unwrap();

    let first = plan.apply(&doc).unwrap();
    let second = plan.apply(&doc).unwrap();
    assert_eq!(first.to_string(), second.to_string());
    assert_eq!(doc.to_string(), source);
}

#[test]
fn error_display() {
    let err = ConvertError::UnmovableComment {
        path: vec!["t".to_owned(), "a".to_owned()],
    };
    assert_eq!(
        err.to_string(),
        "comment at `t.a` cannot be represented in the target form"
    );
    let err = ConvertError::Conflict {
        existing: vec!["a".to_owned()],
        new: vec!["a".to_owned(), "b".to_owned()],
    };
    assert_eq!(
        err.to_string(),
        "conversion target `a.b` overlaps with `a`"
    );
}
