//! Expand selected inline tables into standard `[table]`s
//!
//! Run with: `cargo run -p toml_edit --example expand_inline`

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let raw = "\
# Server settings
server = {host = \"localhost\", tls = {enabled = true}} # primary
logging = {level = \"info\"}
";
    let doc = raw.parse::<toml_edit::DocumentMut>()?;

    // Expand only the selected paths; `logging` stays inline and the
    // original `doc` is left unchanged.
    let expanded = doc.expand_inline_tables([["server"]])?;
    println!("{expanded}");

    // Inline tables inside arrays are out of scope and rejected, as is any
    // invalid path; the whole batch is rejected atomically.
    let doc = "plugins = [{name = \"a\"}]\n".parse::<toml_edit::DocumentMut>()?;
    let err = doc.expand_inline_tables([["plugins"]]).unwrap_err();
    println!("rejected: {err}");

    Ok(())
}
