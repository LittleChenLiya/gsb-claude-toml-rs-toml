//! Plan conversions between explicit and inline tables.
//!
//! Run with:
//!
//! ```console
//! cargo run -p toml_edit --example convert
//! ```

use toml_edit::DocumentMut;
use toml_edit::convert::ConvertPlan;

/// Three tables: two explicit, one inline.
const SOURCE: &str = r#"ui = { theme = "dark", scale = 2 }

# server of the app
[server] # primary
host = "localhost"
port = 8080

[database]
server = "db.local"
ports = [5432, 5433]
"#;

fn main() {
    let doc = SOURCE.parse::<DocumentMut>().expect("invalid doc");

    // Select exactly which tables change form; everything else is untouched.
    let mut plan = ConvertPlan::new();
    plan.inline("server").expect("valid selection");
    plan.inline("database").expect("valid selection");
    plan.expand("ui").expect("valid selection");

    // The source document is never modified.
    let converted = plan.apply(&doc).expect("lossless conversion");
    assert_eq!(doc.to_string(), SOURCE);

    println!("=== before ===");
    println!("{doc}");
    println!("=== after ===");
    println!("{converted}");

    // The parsed values are identical; only the form changed.
    assert_eq!(converted["server"]["host"].as_str(), Some("localhost"));
    assert_eq!(converted["server"]["port"].as_integer(), Some(8080));
    assert_eq!(converted["database"]["server"].as_str(), Some("db.local"));
    assert_eq!(converted["ui"]["theme"].as_str(), Some("dark"));
    assert_eq!(converted["ui"]["scale"].as_integer(), Some(2));
}
