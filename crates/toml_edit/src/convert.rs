//! Plan conversions between explicit tables and inline tables
//!
//! A [`ConvertPlan`] describes which tables in a [`DocumentMut`] change *form*
//! (explicit `[table]` header vs `key = { ... }` inline table) without changing
//! the *value* the document parses to.  The plan is applied to a clone of the
//! source document, so the source [`DocumentMut`] is never modified.
//!
//! Only tables explicitly selected with [`ConvertPlan::inline`] or
//! [`ConvertPlan::expand`] are converted; everything else is left untouched.
//! The conversion is structural: values keep their original [`Repr`][crate::Repr]
//! (their literal text), keys keep their original representation (quoted
//! segments stay quoted), and dotted-key proxies keep their dotted flag, so a
//! dotted path like `a.b.c` resolves to the same value before and after.  This
//! is **not** a textual brace swap.
//!
//! # Selecting tables
//!
//! Tables are selected by a dotted key path parsed with [`Key::parse`], the
//! same syntax as TOML keys:
//!
//! * `server.tls` selects `tls` under `[server]`
//! * `"a.b"` selects a table literally named `a.b`; the quoted segment is
//!   **not** split into `a` then `b`
//!
//! # Conflicts
//!
//! Two selected targets conflict when one path is a prefix of the other
//! (converting an ancestor restructures its descendants).  Conflicts are
//! reported first: [`ConvertPlan::inline`] / [`ConvertPlan::expand`] reject an
//! overlapping selection as it is added, and [`ConvertPlan::apply`] re-checks
//! all targets before validating or converting any of them.
//!
//! # Rejected constructs
//!
//! A target is rejected up front (before anything is converted) when it cannot
//! be converted losslessly:
//!
//! * the path does not exist, or does not name a table of the requested form
//!   ([`ConvertError::NotFound`] / [`ConvertError::UnexpectedKind`])
//! * the table (transitively) contains an array of tables
//!   ([`ConvertError::Unsupported`])
//! * the table body carries a comment that cannot be migrated
//!   ([`ConvertError::NonMigratableComment`])
//!
//! # Decor (whitespace and comment) rules
//!
//! "Decor" is the whitespace and comments attached to keys, values, and table
//! headers.  The rules are:
//!
//! * Comments on the *selected* entry migrate:
//!   * explicit to inline: blank lines and comments above the `[table]` header
//!     move to above the new `table = { ... }` line, and a trailing
//!     `[table] # comment` moves to `table = { ... } # comment`
//!   * inline to explicit: blank lines and comments above the
//!     `table = { ... }` line move to above the new `[table]` header, and a
//!     trailing `table = { ... } # comment` moves to `[table] # comment`
//! * Any other comment inside the converted table body (comment lines between
//!   entries, trailing `key = value # comment`, comments inside nested arrays
//!   or inline tables) has no place to go, so the target is rejected with
//!   [`ConvertError::NonMigratableComment`].
//! * Interior whitespace decor (indentation, alignment around `=`, newlines
//!   inside arrays) is normalized to the default formatting; only the literal
//!   text of each value is preserved.
//! * Expanded tables are emitted after the parent's existing sub-tables;
//!   table order does not affect the parsed value.
//!
//! # Guarantee
//!
//! For any accepted plan, the converted document parses to the same TOML
//! value as the source document.
//!
//! # Example
//!
//! ```rust
//! # #[cfg(feature = "parse")] {
//! # #[cfg(feature = "display")] {
//! use toml_edit::DocumentMut;
//! use toml_edit::convert::ConvertPlan;
//!
//! let source = "[server]\nhost = \"localhost\"\n";
//! let doc = source.parse::<DocumentMut>().unwrap();
//!
//! let mut plan = ConvertPlan::new();
//! plan.inline("server").unwrap();
//! let converted = plan.apply(&doc).unwrap();
//! assert_eq!(converted.to_string(), "server = { host = \"localhost\" }\n");
//! // the source document is unchanged
//! assert_eq!(doc.to_string(), source);
//! # }
//! # }
//! ```
//!
//! See `examples/convert.rs` for a runnable three-table example:
//!
//! ```console
//! $ cargo run -p toml_edit --example convert
//! ```
//!
//! The acceptance tests for these rules can be run with:
//!
//! ```console
//! $ cargo test -p toml_edit --features serde --test convert
//! ```

use std::fmt;

use indexmap::map::MutableKeys as _;

use crate::repr::Decor;
use crate::table::KeyValuePairs;
use crate::{DocumentMut, InlineTable, Item, Key, RawString, Table, Value};

/// A plan for converting selected tables between explicit and inline form
///
/// See the [module-level documentation](self) for the conversion, decor, and
/// conflict rules.
#[derive(Debug, Default, Clone)]
pub struct ConvertPlan {
    targets: Vec<Target>,
}

#[derive(Debug, Clone)]
struct Target {
    /// The path as written by the caller, for error messages
    path: String,
    keys: Vec<Key>,
    direction: Direction,
}

/// The direction of a single table conversion
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// `[table]` becomes `table = { ... }`
    Inline,
    /// `table = { ... }` becomes `[table]`
    Expand,
}

impl ConvertPlan {
    /// Create an empty plan
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of selected tables
    pub fn len(&self) -> usize {
        self.targets.len()
    }

    /// Whether no tables are selected
    pub fn is_empty(&self) -> bool {
        self.targets.is_empty()
    }

    /// Select the explicit table at `path` to become an inline table
    ///
    /// `path` is a dotted key path (see [`Key::parse`]); quoted segments such
    /// as `"a.b"` name a single key and are never split.
    ///
    /// Returns an error immediately if `path` is invalid or overlaps a
    /// previously selected target.
    pub fn inline(&mut self, path: &str) -> Result<&mut Self, ConvertError> {
        self.push(path, Direction::Inline)
    }

    /// Select the inline table at `path` to become an explicit table
    ///
    /// `path` is a dotted key path (see [`Key::parse`]); quoted segments such
    /// as `"a.b"` name a single key and are never split.
    ///
    /// Returns an error immediately if `path` is invalid or overlaps a
    /// previously selected target.
    pub fn expand(&mut self, path: &str) -> Result<&mut Self, ConvertError> {
        self.push(path, Direction::Expand)
    }

    fn push(&mut self, path: &str, direction: Direction) -> Result<&mut Self, ConvertError> {
        let keys = Key::parse(path).map_err(|_| ConvertError::InvalidPath {
            path: path.to_owned(),
        })?;
        if keys.is_empty() {
            return Err(ConvertError::InvalidPath {
                path: path.to_owned(),
            });
        }
        if let Some(existing) = self
            .targets
            .iter()
            .find(|existing| paths_overlap(&existing.keys, &keys))
        {
            return Err(ConvertError::Conflict {
                first: existing.path.clone(),
                second: path.to_owned(),
            });
        }
        self.targets.push(Target {
            path: path.to_owned(),
            keys,
            direction,
        });
        Ok(self)
    }

    /// Apply the plan to a clone of `doc`, returning the converted document
    ///
    /// `doc` itself is never modified.  Targets are checked for conflicts
    /// first, then each target is validated; if any target cannot be
    /// converted losslessly, an error is returned and no document is
    /// produced.
    pub fn apply(&self, doc: &DocumentMut) -> Result<DocumentMut, ConvertError> {
        // Conflicts come first, before any per-target validation.
        for (i, target) in self.targets.iter().enumerate() {
            if let Some(existing) = self.targets[..i]
                .iter()
                .find(|existing| paths_overlap(&existing.keys, &target.keys))
            {
                return Err(ConvertError::Conflict {
                    first: existing.path.clone(),
                    second: target.path.clone(),
                });
            }
        }

        let mut converted = doc.clone();
        for target in &self.targets {
            apply_target(&mut converted, target)?;
        }
        Ok(converted)
    }
}

/// An error while planning or applying a table form conversion
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConvertError {
    /// The selection path is not a valid TOML key path
    InvalidPath {
        /// The offending path
        path: String,
    },
    /// Two selected targets overlap (one path is a prefix of the other)
    Conflict {
        /// The path selected first
        first: String,
        /// The overlapping path rejected later
        second: String,
    },
    /// No item exists at the selected path
    NotFound {
        /// The selected path
        path: String,
    },
    /// The item at (or along) the selected path has the wrong kind
    UnexpectedKind {
        /// The selected path
        path: String,
        /// The kind the conversion requires
        expected: &'static str,
        /// The kind actually found
        actual: &'static str,
    },
    /// The table contains a construct with no lossless conversion
    Unsupported {
        /// The selected path
        path: String,
        /// Why the construct cannot be converted
        reason: &'static str,
    },
    /// A comment inside the table body cannot be migrated
    ///
    /// Only comments attached to the selected entry itself migrate; see the
    /// [module-level documentation](self) for the decor rules.
    NonMigratableComment {
        /// The selected path
        path: String,
    },
}

impl fmt::Display for ConvertError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPath { path } => {
                write!(f, "invalid table selection path: `{path}`")
            }
            Self::Conflict { first, second } => {
                write!(
                    f,
                    "table selections `{first}` and `{second}` overlap; \
                     converting one restructures the other"
                )
            }
            Self::NotFound { path } => {
                write!(f, "no table found at `{path}`")
            }
            Self::UnexpectedKind {
                path,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "cannot convert `{path}`: expected {expected}, found {actual}"
                )
            }
            Self::Unsupported { path, reason } => {
                write!(f, "cannot convert `{path}`: {reason}")
            }
            Self::NonMigratableComment { path } => {
                write!(
                    f,
                    "cannot convert `{path}`: a comment inside the table body \
                     cannot be migrated"
                )
            }
        }
    }
}

impl std::error::Error for ConvertError {}

/// Whether either path is a prefix of (or equal to) the other
fn paths_overlap(a: &[Key], b: &[Key]) -> bool {
    a.iter().zip(b.iter()).all(|(x, y)| x.get() == y.get())
}

fn apply_target(doc: &mut DocumentMut, target: &Target) -> Result<(), ConvertError> {
    let (leaf, parents) = target
        .keys
        .split_last()
        .expect("targets always have at least one key");

    // Validate against the source before mutating anything.
    let parent = navigate(doc.as_table(), parents, target)?;
    match target.direction {
        Direction::Inline => {
            let table = match parent.get(leaf.get()) {
                Some(Item::Table(table)) => table,
                Some(item) => {
                    return Err(ConvertError::UnexpectedKind {
                        path: target.path.clone(),
                        expected: "an explicit table",
                        actual: item.type_name(),
                    });
                }
                None => {
                    return Err(ConvertError::NotFound {
                        path: target.path.clone(),
                    });
                }
            };
            validate_table_for_inline(table, target)?;
        }
        Direction::Expand => {
            let inline = match parent.get(leaf.get()) {
                Some(Item::Value(Value::InlineTable(inline))) => inline,
                Some(item) => {
                    return Err(ConvertError::UnexpectedKind {
                        path: target.path.clone(),
                        expected: "an inline table",
                        actual: item.type_name(),
                    });
                }
                None => {
                    return Err(ConvertError::NotFound {
                        path: target.path.clone(),
                    });
                }
            };
            validate_inline_for_expand(inline, target)?;
        }
    }

    // Validated: mutate the clone.
    let parent = navigate_mut(doc.as_table_mut(), parents)?;
    let (mut key, item) = parent
        .get_key_value_mut(leaf.get())
        .expect("target was validated");
    let placeholder = std::mem::take(item);
    match (target.direction, placeholder) {
        (Direction::Inline, Item::Table(table)) => {
            let header_prefix = table.decor().prefix().cloned();
            let header_suffix = table.decor().suffix().cloned();
            let mut value = Value::InlineTable(table_into_inline(table));
            // The header comment becomes the trailing comment of the new line.
            if let Some(suffix) = header_suffix {
                value.decor_mut().set_suffix(suffix);
            }
            *item = Item::Value(value);
            // Comments and blank lines above the header move to above the new
            // `key = { ... }` line; the rest of the key decor resets to the
            // default key/value formatting.
            key.leaf_decor_mut().clear();
            key.dotted_decor_mut().clear();
            if let Some(prefix) = header_prefix {
                key.leaf_decor_mut().set_prefix(prefix);
            }
        }
        (Direction::Expand, Item::Value(Value::InlineTable(inline))) => {
            let line_prefix = key.leaf_decor().prefix().cloned();
            let trailing_comment = inline.decor().suffix().cloned();
            let mut table = inline_into_table(inline);
            // Comments and blank lines above the `key = { ... }` line move to
            // above the new header; the trailing comment moves behind it.
            if let Some(prefix) = line_prefix {
                table.decor_mut().set_prefix(prefix);
            }
            if let Some(suffix) = trailing_comment {
                table.decor_mut().set_suffix(suffix);
            }
            *item = Item::Table(table);
            // The key is now encoded inside a `[...]` header; reset its decor
            // so the line decor is not duplicated before the header.
            key.leaf_decor_mut().clear();
            key.dotted_decor_mut().clear();
        }
        _ => unreachable!("target was validated"),
    }
    Ok(())
}

fn navigate<'a>(
    mut table: &'a Table,
    parents: &[Key],
    target: &Target,
) -> Result<&'a Table, ConvertError> {
    for key in parents {
        table = match table.get(key.get()) {
            Some(Item::Table(child)) => child,
            Some(item) => {
                return Err(ConvertError::UnexpectedKind {
                    path: target.path.clone(),
                    expected: "an explicit table along the path",
                    actual: item.type_name(),
                });
            }
            None => {
                return Err(ConvertError::NotFound {
                    path: target.path.clone(),
                });
            }
        };
    }
    Ok(table)
}

fn navigate_mut<'a>(
    mut table: &'a mut Table,
    parents: &[Key],
) -> Result<&'a mut Table, ConvertError> {
    for key in parents {
        table = match table.get_mut(key.get()) {
            Some(Item::Table(child)) => child,
            Some(_) | None => unreachable!("target was validated"),
        };
    }
    Ok(table)
}

/// Every comment inside the table body must be rejected; only the selected
/// table's own header decor migrates (handled by the caller).
fn validate_table_for_inline(table: &Table, target: &Target) -> Result<(), ConvertError> {
    for (key, item) in table.iter() {
        let key = table.key(key).expect("iter yields existing keys");
        if key_has_comment(key) {
            return Err(ConvertError::NonMigratableComment {
                path: target.path.clone(),
            });
        }
        match item {
            Item::None => {}
            Item::Value(value) => validate_value(value, target)?,
            Item::Table(child) => {
                // A nested `[table.child]` header's decor is interior to the body.
                if decor_has_comment(child.decor()) {
                    return Err(ConvertError::NonMigratableComment {
                        path: target.path.clone(),
                    });
                }
                validate_table_for_inline(child, target)?;
            }
            Item::ArrayOfTables(_) => {
                return Err(ConvertError::Unsupported {
                    path: target.path.clone(),
                    reason: "contains an array of tables, which has no inline table form",
                });
            }
        }
    }
    Ok(())
}

/// The selected inline table's trailing comment migrates to the new header;
/// everything else inside must be comment-free.
fn validate_inline_for_expand(inline: &InlineTable, target: &Target) -> Result<(), ConvertError> {
    if raw_has_comment(inline.decor().prefix()) || raw_has_comment(Some(inline.trailing())) {
        return Err(ConvertError::NonMigratableComment {
            path: target.path.clone(),
        });
    }
    for (key, value) in inline.iter() {
        let key = inline.key(key).expect("iter yields existing keys");
        if key_has_comment(key) {
            return Err(ConvertError::NonMigratableComment {
                path: target.path.clone(),
            });
        }
        validate_value(value, target)?;
    }
    Ok(())
}

fn validate_value(value: &Value, target: &Target) -> Result<(), ConvertError> {
    if decor_has_comment(value.decor()) {
        return Err(ConvertError::NonMigratableComment {
            path: target.path.clone(),
        });
    }
    match value {
        Value::Array(array) => {
            if decor_has_comment(array.decor()) || raw_has_comment(Some(array.trailing())) {
                return Err(ConvertError::NonMigratableComment {
                    path: target.path.clone(),
                });
            }
            for element in array.iter() {
                validate_value(element, target)?;
            }
        }
        Value::InlineTable(inline) => {
            if decor_has_comment(inline.decor()) || raw_has_comment(Some(inline.trailing())) {
                return Err(ConvertError::NonMigratableComment {
                    path: target.path.clone(),
                });
            }
            for (key, element) in inline.iter() {
                let key = inline.key(key).expect("iter yields existing keys");
                if key_has_comment(key) {
                    return Err(ConvertError::NonMigratableComment {
                        path: target.path.clone(),
                    });
                }
                validate_value(element, target)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Convert an explicit table into an inline table, preserving value
/// representations and dotted-key flags.
///
/// The table must already be validated by [`validate_table_for_inline`].
fn table_into_inline(table: Table) -> InlineTable {
    let dotted = table.is_dotted();
    let mut items: KeyValuePairs = table.items;
    for (key, item) in items.iter_mut2() {
        key.as_mut().leaf_decor_mut().clear();
        key.as_mut().dotted_decor_mut().clear();
        match item {
            Item::None | Item::ArrayOfTables(_) => {}
            Item::Value(value) => normalize_value_decor(value),
            Item::Table(_) => {
                let Item::Table(child) = std::mem::take(item) else {
                    unreachable!()
                };
                *item = Item::Value(Value::InlineTable(table_into_inline(child)));
            }
        }
    }
    let mut inline = InlineTable::with_pairs(items);
    inline.set_dotted(dotted);
    inline
}

/// Convert an inline table into an explicit table, preserving value
/// representations and dotted-key flags.
///
/// The table must already be validated by [`validate_inline_for_expand`].
fn inline_into_table(inline: InlineTable) -> Table {
    let dotted = inline.is_dotted();
    let mut items: KeyValuePairs = inline.items;
    for (key, item) in items.iter_mut2() {
        key.as_mut().leaf_decor_mut().clear();
        key.as_mut().dotted_decor_mut().clear();
        if let Item::Value(value) = item {
            normalize_value_decor(value);
        }
    }
    let mut table = Table::with_pairs(items);
    table.set_dotted(dotted);
    table
}

/// Reset interior whitespace decor to the default formatting
///
/// Value representations (the literal text) are untouched; only the
/// whitespace decor that is meaningless in the new form is cleared.  Comments
/// cannot be present: the target was validated first.
fn normalize_value_decor(value: &mut Value) {
    value.decor_mut().clear();
    match value {
        Value::Array(array) => {
            array.decor_mut().clear();
            array.set_trailing("");
            for element in array.iter_mut() {
                normalize_value_decor(element);
            }
        }
        Value::InlineTable(inline) => {
            inline.decor_mut().clear();
            inline.set_trailing("");
            for (mut key, element) in inline.iter_mut() {
                key.leaf_decor_mut().clear();
                key.dotted_decor_mut().clear();
                normalize_value_decor(element);
            }
        }
        _ => {}
    }
}

fn key_has_comment(key: &Key) -> bool {
    decor_has_comment(key.leaf_decor()) || decor_has_comment(key.dotted_decor())
}

fn decor_has_comment(decor: &Decor) -> bool {
    raw_has_comment(decor.prefix()) || raw_has_comment(decor.suffix())
}

fn raw_has_comment(raw: Option<&RawString>) -> bool {
    raw.and_then(|raw| raw.as_str())
        .is_some_and(|raw| raw.contains('#'))
}
