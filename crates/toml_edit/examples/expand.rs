//! Example for how to use `DocumentMut::expand_inline_tables` to explicitly
//! convert selected inline tables into regular tables.

use toml_edit::{DocumentMut, Key};

fn main() {
    let input = r#"
# top comment
a = { b = { c = 1 }, d = 2 } # inline comment
# trailing comment
"#;

    let mut doc: DocumentMut = input.parse().expect("input is valid TOML");

    println!("** before expansion");
    println!("{doc}");

    // Expand the inline table at path `a` (and its inline subtables).
    let paths: &[&[Key]] = &[&["a".parse().unwrap()]];
    doc.expand_inline_tables(paths)
        .expect("expansion should succeed");

    println!("** after expansion");
    println!("{doc}");

    // The expanded document can be parsed back to the same data.
    let reparsed: DocumentMut = doc.to_string().parse().expect("reparse should succeed");
    assert_eq!(reparsed["a"]["d"].as_integer().unwrap(), 2);
    assert_eq!(reparsed["a"]["b"]["c"].as_integer().unwrap(), 1);
    println!("** data read back correctly");
}
