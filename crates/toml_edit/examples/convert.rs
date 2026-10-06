//! Example: planning faithful conversions between `[table]` and `{ inline }` forms.
//!
//! Three tables are selected in one [`ConversionPlan`]: two standard tables
//! become inline tables and one inline table becomes a standard table.  The
//! source document is never modified; `apply` returns a converted clone that
//! parses to the same values.
//!
//! Run with: cargo run -p toml_edit --example convert
//! Test with: cargo test -p toml_edit --example convert

use toml_edit::{ConversionPlan, DocumentMut};

fn main() {
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

    let doc: DocumentMut = source.parse().unwrap();

    // Register three targets; overlapping paths would be rejected here,
    // before any conversion runs.
    let mut plan = ConversionPlan::new();
    plan.to_inline(["point"]).unwrap();
    plan.to_inline(["server"]).unwrap();
    plan.to_table(["style"]).unwrap();

    let converted = plan.apply(&doc).unwrap();

    println!("--- source (unchanged) ---\n{source}");
    println!("--- converted ---\n{converted}");

    let expected = "\
title = \"shapes\"
point = { x = 1, y = 2 }
server = { host = \"localhost\", ports = [8000, 8001] }

[style]
color = \"red\"
width = 2
";
    assert_eq!(converted.to_string(), expected);
    assert_eq!(doc.to_string(), source, "source document must not change");

    // The converted document still parses and holds the same values.
    let reparsed: DocumentMut = converted.to_string().parse().unwrap();
    assert_eq!(reparsed["point"]["x"].as_integer(), Some(1));
    assert_eq!(reparsed["server"]["ports"][1].as_integer(), Some(8001));
    assert_eq!(reparsed["style"]["color"].as_str(), Some("red"));
    assert_eq!(reparsed["title"].as_str(), Some("shapes"));

    // Constructions that cannot be converted faithfully are rejected, not
    // silently dropped: comments cannot live inside a single-line inline
    // table.
    let commented: DocumentMut = "# about t\n[t]\na = 1\n".parse().unwrap();
    let mut plan = ConversionPlan::new();
    plan.to_inline(["t"]).unwrap();
    let err = plan.apply(&commented).unwrap_err();
    println!("--- rejected ---\n{err}");
}
