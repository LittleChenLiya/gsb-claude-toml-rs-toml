//! Format-preserving integer updates: a tiny before/after demonstration.
//!
//! Run with:
//!     cargo run -p `toml_edit` --example `update_integers`
//!
//! The robot only retunes two integers.  Hexadecimal base, underscore
//! grouping, signs, comments and unrelated values are preserved; a target
//! that turns out to be a string rejects the whole batch without touching the
//! document.

use toml_edit::{update_document_integers, DocumentMut, IntegerUpdate};

fn main() {
    let before = "\
# robot tuning knobs
limit = 0x00_FF # per-cycle limit
offset = 0o10
name = \"worker-42\"
";
    println!("=== before ===\n{before}");

    let mut doc: DocumentMut = before.parse().unwrap();
    let report = update_document_integers(
        &mut doc,
        [
            IntegerUpdate::new(["limit"], 256),
            IntegerUpdate::new(["offset"], -4),
        ],
    )
    .unwrap();

    println!("=== after ===");
    print!("{doc}");

    for (path, fallback) in report.fallbacks() {
        println!(
            "note: `{}` could not keep its base ({fallback:?}); decimal used",
            path.join(".")
        );
    }

    // The serialized document re-parses to the expected integers.
    let reparsed: DocumentMut = doc.to_string().parse().unwrap();
    assert_eq!(reparsed["limit"].as_integer(), Some(256));
    assert_eq!(reparsed["offset"].as_integer(), Some(-4));

    // A batch containing a string target is rejected atomically.
    let mut doc: DocumentMut = before.parse().unwrap();
    let snapshot = doc.to_string();
    let err = update_document_integers(
        &mut doc,
        [
            IntegerUpdate::new(["limit"], 256),
            IntegerUpdate::new(["name"], 1),
        ],
    )
    .unwrap_err();
    println!("rejected batch: {err}");
    assert_eq!(doc.to_string(), snapshot);
}
