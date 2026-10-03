//! Before/after text demo for the integer update tool.
//!
//! Run with:
//! ```sh
//! cargo run -p toml_edit --example integer_update
//! ```

use toml_edit::{DocumentMut, IntegerRadix, IntegerUpdate};

fn main() {
    println!("=== Example 1: hexadecimal preserved, comment preserved ===");
    demo("limit = 0x00_FF # comment\n", &[("limit", 256)]);

    println!("\n=== Example 2: negative value falls back to decimal ===");
    demo("offset = 0x00_FF # base offset\n", &[("offset", -1)]);

    println!("\n=== Example 3: dotted keys and quoted keys ===");
    demo(
        "\"db\".\"host\" = 1\n\n[server]\nport = 8080\n",
        &[("db.host", 2), ("server.port", 9090)],
    );

    println!("\n=== Example 4: i64 boundaries ===");
    demo(
        "min = 0x00\nmax = 0\n",
        &[("min", i64::MIN), ("max", i64::MAX)],
    );

    println!("\n=== Example 5: batch rejected when one target is a string ===");
    demo_reject("a = 1\nb = \"hello\"\n", &[("a", 2), ("b", 3)]);
}

fn demo(before: &str, updates: &[(&str, i64)]) {
    println!("--- before ---");
    print!("{before}");

    let mut doc = before.parse::<DocumentMut>().expect("invalid doc");
    let reqs: Vec<IntegerUpdate> = updates
        .iter()
        .map(|(path, value)| IntegerUpdate::new(path, *value).unwrap())
        .collect();
    let report = doc.update_integers(&reqs).expect("update failed");

    println!("--- after ---");
    print!("{doc}");

    for (i, outcome) in report.outcomes().iter().enumerate() {
        let path = outcome
            .path()
            .iter()
            .map(|k| k.get())
            .collect::<Vec<_>>()
            .join(".");
        let note = if outcome.fell_back() {
            format!(" (fell back to {})", IntegerRadix::Decimal)
        } else {
            String::new()
        };
        println!(
            "  update {}: `{path}` -> {} [{}]{}",
            i + 1,
            outcome.value(),
            outcome.radix(),
            note
        );
    }
}

fn demo_reject(before: &str, updates: &[(&str, i64)]) {
    println!("--- before ---");
    print!("{before}");

    let mut doc = before.parse::<DocumentMut>().expect("invalid doc");
    let reqs: Vec<IntegerUpdate> = updates
        .iter()
        .map(|(path, value)| IntegerUpdate::new(path, *value).unwrap())
        .collect();
    let result = doc.update_integers(&reqs);

    println!("--- result ---");
    match result {
        Ok(_) => println!("  unexpectedly applied"),
        Err(err) => println!("  rejected: {err}"),
    }
    println!("--- after (unchanged) ---");
    print!("{doc}");
}
