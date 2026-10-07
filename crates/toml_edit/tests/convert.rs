//! Acceptance tests for `toml_edit::convert` (table form conversion plans).

#![recursion_limit = "256"]

use toml_edit::DocumentMut;
use toml_edit::convert::{ConvertError, ConvertPlan};

/// Parse a document back into a semantic value for comparison.
fn semantic(doc: &DocumentMut) -> toml_types::Value {
    toml_edit::de::from_str(&doc.to_string()).expect("converted document should parse")
}

fn parse(source: &str) -> DocumentMut {
    source.parse::<DocumentMut>().expect("invalid doc")
}

fn plan_apply(source: &str, plan: &ConvertPlan) -> DocumentMut {
    let doc = parse(source);
    let converted = plan.apply(&doc).expect("plan should apply");
    // The source document is never modified.
    assert_eq!(doc.to_string(), source);
    // The converted document parses to the same TOML value.
    assert_eq!(semantic(&doc), semantic(&converted));
    converted
}

#[test]
fn inline_simple_table_keeps_value() {
    let source = "[server]\nhost = \"localhost\"\nport = 8080\n";
    let mut plan = ConvertPlan::new();
    plan.inline("server").unwrap();
    let converted = plan_apply(source, &plan);
    assert_eq!(
        converted.to_string(),
        "server = { host = \"localhost\", port = 8080 }\n"
    );
}

#[test]
fn inline_preserves_value_repr_and_key_repr() {
    let source = "[server]\nport = 0x1F90\nname = 'literal'\nratio = 1e3\n";
    let mut plan = ConvertPlan::new();
    plan.inline("server").unwrap();
    let converted = plan_apply(source, &plan);
    // Value identity: the literal representations survive the move.
    assert_eq!(
        converted.to_string(),
        "server = { port = 0x1F90, name = 'literal', ratio = 1e3 }\n"
    );
}

#[test]
fn inline_keeps_dotted_key_paths() {
    let source = "[a]\nx = 1\nb.c = 2\n[a.d]\ny = 3\n";
    let mut plan = ConvertPlan::new();
    plan.inline("a").unwrap();
    let converted = plan_apply(source, &plan);
    // The dotted key `b.c` keeps its dotted form inside the inline table.
    assert_eq!(
        converted.to_string(),
        "a = { x = 1, b.c = 2, d = { y = 3 } }\n"
    );
    // The dotted path still resolves to the same value.
    assert_eq!(converted["a"]["b"]["c"].as_integer(), Some(2));
    assert_eq!(converted["a"]["d"]["y"].as_integer(), Some(3));
}

#[test]
fn inline_quoted_dotted_key_is_not_split() {
    let source = "[\"a.b\"]\nx = 1\n";
    let mut plan = ConvertPlan::new();
    plan.inline("\"a.b\"").unwrap();
    let converted = plan_apply(source, &plan);
    assert_eq!(converted.to_string(), "\"a.b\" = { x = 1 }\n");
    // The literal `a.b` key exists; no `a` -> `b` nesting was introduced.
    assert_eq!(converted["a.b"]["x"].as_integer(), Some(1));
    assert!(converted.as_table().get("a").is_none());

    // The unquoted path names a different (missing) table.
    let mut plan = ConvertPlan::new();
    plan.inline("a.b").unwrap();
    let err = plan.apply(&parse(source)).unwrap_err();
    assert!(
        matches!(err, ConvertError::NotFound { .. }),
        "unexpected error: {err}"
    );
}

#[test]
fn inline_rejects_non_migratable_comments() {
    // A comment line inside the table body.
    let source = "[server]\n# about host\nhost = \"localhost\"\n";
    let mut plan = ConvertPlan::new();
    plan.inline("server").unwrap();
    let err = plan.apply(&parse(source)).unwrap_err();
    assert!(
        matches!(err, ConvertError::NonMigratableComment { .. }),
        "unexpected error: {err}"
    );

    // A trailing comment on an entry.
    let source = "[server]\nhost = \"localhost\" # why\n";
    let err = plan.apply(&parse(source)).unwrap_err();
    assert!(
        matches!(err, ConvertError::NonMigratableComment { .. }),
        "unexpected error: {err}"
    );

    // A comment inside a nested array.
    let source = "[server]\nports = [\n  8080, # http\n]\n";
    let err = plan.apply(&parse(source)).unwrap_err();
    assert!(
        matches!(err, ConvertError::NonMigratableComment { .. }),
        "unexpected error: {err}"
    );

    // A comment on a nested table header.
    let source = "[a]\nx = 1\n[a.b] # nested\ny = 2\n";
    let mut plan = ConvertPlan::new();
    plan.inline("a").unwrap();
    let err = plan.apply(&parse(source)).unwrap_err();
    assert!(
        matches!(err, ConvertError::NonMigratableComment { .. }),
        "unexpected error: {err}"
    );
}

#[test]
fn inline_migrates_header_comments() {
    let source = "# about server\n[server] # trailing\nhost = \"localhost\"\n";
    let mut plan = ConvertPlan::new();
    plan.inline("server").unwrap();
    let converted = plan_apply(source, &plan);
    assert_eq!(
        converted.to_string(),
        "# about server\nserver = { host = \"localhost\" } # trailing\n"
    );
}

#[test]
fn expand_simple_inline_table() {
    let source = "[server]\nui = { theme = \"dark\", scale = 2 }\n";
    let mut plan = ConvertPlan::new();
    plan.expand("server.ui").unwrap();
    let converted = plan_apply(source, &plan);
    assert_eq!(
        converted.to_string(),
        "[server]\n[server.ui]\ntheme = \"dark\"\nscale = 2\n"
    );
}

#[test]
fn expand_migrates_comments() {
    let source = "# about ui\nui = { theme = \"dark\" } # trailing\n";
    let mut plan = ConvertPlan::new();
    plan.expand("ui").unwrap();
    let converted = plan_apply(source, &plan);
    assert_eq!(
        converted.to_string(),
        "# about ui\n[ui] # trailing\ntheme = \"dark\"\n"
    );
}

#[test]
fn expand_keeps_dotted_key_paths() {
    let source = "ui = { theme.name = \"dark\", scale = 2 }\n";
    let mut plan = ConvertPlan::new();
    plan.expand("ui").unwrap();
    let converted = plan_apply(source, &plan);
    assert_eq!(
        converted.to_string(),
        "[ui]\ntheme.name = \"dark\"\nscale = 2\n"
    );
    assert_eq!(converted["ui"]["theme"]["name"].as_str(), Some("dark"));
}

#[test]
fn only_selected_tables_are_converted() {
    let source = "[a]\nx = 1\n[b]\ny = 2\n";
    let mut plan = ConvertPlan::new();
    plan.inline("a").unwrap();
    let converted = plan_apply(source, &plan);
    assert_eq!(converted.to_string(), "a = { x = 1 }\n[b]\ny = 2\n");
}

#[test]
fn multi_target_plan_applies_all_selections() {
    let source =
        "ui = { theme = \"dark\" }\n[server]\nhost = \"localhost\"\n[database]\nport = 5432\n";
    let mut plan = ConvertPlan::new();
    plan.inline("server").unwrap();
    plan.inline("database").unwrap();
    plan.expand("ui").unwrap();
    assert_eq!(plan.len(), 3);
    let converted = plan_apply(source, &plan);
    assert_eq!(converted["server"]["host"].as_str(), Some("localhost"));
    assert_eq!(converted["database"]["port"].as_integer(), Some(5432));
    assert_eq!(converted["ui"]["theme"].as_str(), Some("dark"));
}

#[test]
fn overlapping_targets_conflict_first() {
    // Ancestor/descendant selections conflict, reported when added.
    let mut plan = ConvertPlan::new();
    plan.inline("a").unwrap();
    let err = plan.inline("a.b").unwrap_err();
    assert!(
        matches!(err, ConvertError::Conflict { .. }),
        "unexpected error: {err}"
    );
    let message = err.to_string();
    assert!(message.contains("a.b"), "message: {message}");

    // The same table selected twice conflicts, even across directions.
    let mut plan = ConvertPlan::new();
    plan.inline("a").unwrap();
    assert!(matches!(
        plan.expand("a").unwrap_err(),
        ConvertError::Conflict { .. }
    ));

    // The conflict is reported before any target is validated: `missing`
    // does not exist, but the overlap with `missing.child` wins.
    let mut plan = ConvertPlan::new();
    plan.inline("missing").unwrap();
    let err = plan.inline("missing.child").unwrap_err();
    assert!(
        matches!(err, ConvertError::Conflict { .. }),
        "unexpected error: {err}"
    );

    // Non-overlapping selections on the same parent are fine.
    let mut plan = ConvertPlan::new();
    plan.inline("a.b").unwrap();
    plan.inline("a.c").unwrap();
    assert_eq!(plan.len(), 2);
}

#[test]
fn invalid_or_missing_targets_are_rejected() {
    let mut plan = ConvertPlan::new();
    assert!(matches!(
        plan.inline("").unwrap_err(),
        ConvertError::InvalidPath { .. }
    ));
    assert!(matches!(
        plan.inline("a..b").unwrap_err(),
        ConvertError::InvalidPath { .. }
    ));
    assert!(plan.is_empty());

    let source = "[server]\nhost = \"localhost\"\n";
    let mut plan = ConvertPlan::new();
    plan.inline("missing").unwrap();
    assert!(matches!(
        plan.apply(&parse(source)).unwrap_err(),
        ConvertError::NotFound { .. }
    ));

    // An inline table is not an explicit table, and vice versa.
    let mut plan = ConvertPlan::new();
    plan.inline("server.host").unwrap();
    assert!(matches!(
        plan.apply(&parse(source)).unwrap_err(),
        ConvertError::UnexpectedKind { .. }
    ));
    let mut plan = ConvertPlan::new();
    plan.expand("server").unwrap();
    assert!(matches!(
        plan.apply(&parse(source)).unwrap_err(),
        ConvertError::UnexpectedKind { .. }
    ));
}

#[test]
fn unsupported_constructs_are_rejected() {
    // An array of tables is not a table.
    let source = "[[a]]\nx = 1\n";
    let mut plan = ConvertPlan::new();
    plan.inline("a").unwrap();
    assert!(matches!(
        plan.apply(&parse(source)).unwrap_err(),
        ConvertError::UnexpectedKind { .. }
    ));

    // A table containing an array of tables cannot be inlined.
    let source = "[a]\nx = 1\n[[a.b]]\ny = 2\n";
    let mut plan = ConvertPlan::new();
    plan.inline("a").unwrap();
    assert!(matches!(
        plan.apply(&parse(source)).unwrap_err(),
        ConvertError::Unsupported { .. }
    ));
}

#[test]
fn round_trip_restores_value() {
    let source = "[server]\nhost = \"localhost\"\nport = 8080\n";
    let doc = parse(source);

    let mut inline_plan = ConvertPlan::new();
    inline_plan.inline("server").unwrap();
    let inlined = inline_plan.apply(&doc).unwrap();

    let mut expand_plan = ConvertPlan::new();
    expand_plan.expand("server").unwrap();
    let restored = expand_plan.apply(&inlined).unwrap();

    assert_eq!(semantic(&doc), semantic(&restored));
    assert_eq!(restored.to_string(), source);
}

#[test]
fn expand_through_dotted_key_parent() {
    // `a` is a dotted-key proxy table; expanding `a.b` must keep the path.
    let source = "a.b = { x = 1 }\n";
    let mut plan = ConvertPlan::new();
    plan.expand("a.b").unwrap();
    let converted = plan_apply(source, &plan);
    assert_eq!(converted["a"]["b"]["x"].as_integer(), Some(1));
    assert_eq!(converted.to_string(), "[a.b]\nx = 1\n");
}
