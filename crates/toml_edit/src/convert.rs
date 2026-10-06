//! Faithful, plan-based conversion between explicit [`Table`] and [`InlineTable`] forms
//!
//! A TOML table can be spelled two ways:
//!
//! ```toml
//! [point]
//! x = 1
//! y = 2
//! ```
//!
//! ```toml
//! point = { x = 1, y = 2 }
//! ```
//!
//! Switching between the two spellings is *not* a matter of string surgery on
//! `[...]` / `{...}`: comments, dotted keys, quoted keys, and sub-tables all
//! have to keep their meaning.  [`ConversionPlan`] performs the conversion on
//! the parsed document tree instead:
//!
//! - **Value identity**: re-parsing the converted document produces the same
//!   logical content as the original.
//! - **Explicit selection**: only the tables named in the plan are converted;
//!   everything else keeps its current form.
//! - **Immutable source**: [`ConversionPlan::apply`] takes `&DocumentMut` and
//!   returns a converted clone; the source document is never modified.
//! - **Conflicts first**: overlapping targets (one path equal to, or nested
//!   under, another) are rejected when the target is registered, before any
//!   conversion runs.
//! - **No lossy conversions**: constructions that cannot be represented in
//!   the target form are rejected with a [`ConvertError`] instead of being
//!   silently dropped.
//!
//! # Dotted and quoted keys
//!
//! Dotted keys stay dotted: `a.b = 1` inside a converted table is rendered as
//! `a.b = 1` inside the braces (and vice versa), never as a nested
//! `a = { b = 1 }`.  A quoted key such as `"a.b"` is a single key; its
//! original quoting is preserved and it is never split into a dotted path.
//! Plan paths address keys by their parsed name, so the table at `"a.b"` is
//! selected with `&["a.b"]`.
//!
//! # Decor (whitespace and comment) rules
//!
//! - Whitespace-only decor (indentation, blank lines, padding) is normalized
//!   to the default formatting of the target form.
//! - Table-to-inline: the result is a single-line inline table, which has
//!   nowhere to hold comments.  Any comment in the table header decor, a key
//!   decor, a value decor, or inside nested arrays / inline tables is
//!   rejected with [`ConvertError::UnmovableComment`].
//! - Inline-to-table: a trailing line comment on the value
//!   (`key = { ... } # note`) migrates to the new table header
//!   (`[key] # note`).  Any other comment inside the braces is rejected with
//!   [`ConvertError::UnmovableComment`].  Comments attached to keys above the
//!   value stay attached to those keys.
//! - Key and value *reprs* (quoting style, number formatting) are preserved.
//!
//! # Verifying
//!
//! ```sh
//! cargo test -p toml_edit --test testsuite convert
//! cargo run -p toml_edit --example convert
//! ```
//!
//! # Example
//!
//! ```
//! # #[cfg(feature = "parse")] {
//! # #[cfg(feature = "display")] {
//! use toml_edit::{ConversionPlan, DocumentMut};
//!
//! let source = "[point]\nx = 1\ny = 2\n";
//! let doc: DocumentMut = source.parse().unwrap();
//!
//! let mut plan = ConversionPlan::new();
//! plan.to_inline(["point"]).unwrap();
//! let converted = plan.apply(&doc).unwrap();
//!
//! assert_eq!(converted.to_string(), "point = { x = 1, y = 2 }\n");
//! // the source document is unchanged
//! assert_eq!(doc.to_string(), source);
//! # }
//! # }
//! ```

use indexmap::map::MutableKeys as _;

use crate::{Decor, DocumentMut, InlineTable, Item, RawString, Table, Value};

/// A set of [`Table`] / [`InlineTable`] conversions to apply to a [`DocumentMut`]
///
/// See [module-level documentation](self) for the conversion rules.
#[derive(Debug, Default, Clone)]
pub struct ConversionPlan {
    targets: Vec<Target>,
}

#[derive(Debug, Clone)]
struct Target {
    path: Vec<String>,
    direction: Direction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    ToInline,
    ToTable,
}

/// Builders
impl ConversionPlan {
    /// Creates an empty plan
    pub fn new() -> Self {
        Self::default()
    }

    /// Select the explicit [`Table`] at `path` to be converted to an [`InlineTable`] value
    ///
    /// `path` addresses keys by their parsed name, from the root of the
    /// document; the table at `"a.b"` (a quoted key) is selected with
    /// `&["a.b"]`.
    ///
    /// # Errors
    ///
    /// Returns [`ConvertError::EmptyPath`] for an empty path and
    /// [`ConvertError::Conflict`] when the path overlaps a target that is
    /// already registered.  All other validation happens in
    /// [`apply`][ConversionPlan::apply].
    pub fn to_inline<'p>(
        &mut self,
        path: impl IntoIterator<Item = &'p str>,
    ) -> Result<&mut Self, ConvertError> {
        self.push(path, Direction::ToInline)
    }

    /// Select the [`InlineTable`] value at `path` to be converted to an explicit [`Table`]
    ///
    /// `path` addresses keys by their parsed name, from the root of the
    /// document; the value at `"a.b"` (a quoted key) is selected with
    /// `&["a.b"]`.
    ///
    /// # Errors
    ///
    /// Returns [`ConvertError::EmptyPath`] for an empty path and
    /// [`ConvertError::Conflict`] when the path overlaps a target that is
    /// already registered.  All other validation happens in
    /// [`apply`][ConversionPlan::apply].
    pub fn to_table<'p>(
        &mut self,
        path: impl IntoIterator<Item = &'p str>,
    ) -> Result<&mut Self, ConvertError> {
        self.push(path, Direction::ToTable)
    }

    fn push<'p>(
        &mut self,
        path: impl IntoIterator<Item = &'p str>,
        direction: Direction,
    ) -> Result<&mut Self, ConvertError> {
        let path: Vec<String> = path.into_iter().map(str::to_owned).collect();
        if path.is_empty() {
            return Err(ConvertError::EmptyPath);
        }
        if let Some(existing) = self.targets.iter().find(|t| overlaps(&t.path, &path)) {
            return Err(ConvertError::Conflict {
                existing: existing.path.clone(),
                new: path,
            });
        }
        self.targets.push(Target { path, direction });
        Ok(self)
    }
}

/// Accessors
impl ConversionPlan {
    /// Number of conversion targets in the plan
    pub fn len(&self) -> usize {
        self.targets.len()
    }

    /// Whether the plan has no conversion targets
    pub fn is_empty(&self) -> bool {
        self.targets.is_empty()
    }
}

/// Application
impl ConversionPlan {
    /// Validate the plan against `document` and return a converted clone
    ///
    /// Every target is validated against the source document before anything
    /// is converted; `document` is never modified.  Only the explicitly
    /// selected tables are converted.
    ///
    /// # Errors
    ///
    /// Returns the first [`ConvertError`] encountered while validating the
    /// targets against `document`.
    pub fn apply(&self, document: &DocumentMut) -> Result<DocumentMut, ConvertError> {
        for target in &self.targets {
            validate(document.as_item(), target)?;
        }
        let mut converted = document.clone();
        for target in &self.targets {
            let (leaf, parent_path) =
                target.path.split_last().expect("empty paths are rejected");
            let parent = navigate_mut(converted.as_item_mut(), parent_path)
                .expect("target was validated against an identical document");
            convert_target(parent, leaf, target.direction);
        }
        Ok(converted)
    }
}

/// An error while planning or applying a [`Table`] / [`InlineTable`] conversion
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConvertError {
    /// Two targets overlap: one path is equal to, or nested under, the other
    Conflict {
        /// Path of the already-registered target
        existing: Vec<String>,
        /// Path whose registration was rejected
        new: Vec<String>,
    },
    /// A conversion target was registered with an empty path
    EmptyPath,
    /// No item exists at the target path
    NotFound {
        /// The target path
        path: Vec<String>,
    },
    /// The item at the target path has the wrong kind for the conversion
    WrongKind {
        /// The target path
        path: Vec<String>,
        /// What the conversion requires
        expected: &'static str,
        /// What was found instead
        found: &'static str,
    },
    /// The target table is implicit (it has no header of its own)
    ImplicitTable {
        /// The target path
        path: Vec<String>,
    },
    /// The target is a dotted-key proxy, not an explicitly spelled table
    DottedTable {
        /// The target path
        path: Vec<String>,
    },
    /// The table contains sub-tables or arrays of tables that were not selected
    ChildTables {
        /// Path of the child that cannot be moved
        path: Vec<String>,
    },
    /// A comment cannot be represented in the target form
    UnmovableComment {
        /// Path of the item carrying the comment
        path: Vec<String>,
    },
    /// The target is nested where a standard table cannot be rendered
    UnsupportedNesting {
        /// The target path
        path: Vec<String>,
    },
}

impl std::fmt::Display for ConvertError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Conflict { existing, new } => write!(
                f,
                "conversion target `{}` overlaps with `{}`",
                join_path(new),
                join_path(existing)
            ),
            Self::EmptyPath => write!(f, "conversion target path is empty"),
            Self::NotFound { path } => {
                write!(f, "no item at `{}`", join_path(path))
            }
            Self::WrongKind {
                path,
                expected,
                found,
            } => write!(
                f,
                "expected {expected} at `{}`, found {found}",
                join_path(path)
            ),
            Self::ImplicitTable { path } => write!(
                f,
                "table at `{}` is implicit and has no header to convert",
                join_path(path)
            ),
            Self::DottedTable { path } => write!(
                f,
                "table at `{}` is a dotted-key proxy, not an explicit table",
                join_path(path)
            ),
            Self::ChildTables { path } => write!(
                f,
                "table at `{}` was not selected and cannot move with its parent",
                join_path(path)
            ),
            Self::UnmovableComment { path } => write!(
                f,
                "comment at `{}` cannot be represented in the target form",
                join_path(path)
            ),
            Self::UnsupportedNesting { path } => write!(
                f,
                "inline table at `{}` is nested where a standard table cannot be rendered",
                join_path(path)
            ),
        }
    }
}

impl std::error::Error for ConvertError {}

fn join_path(path: &[String]) -> String {
    path.join(".")
}

/// Two targets overlap when one path is equal to, or a prefix of, the other
fn overlaps(a: &[String], b: &[String]) -> bool {
    a.iter().zip(b.iter()).all(|(x, y)| x == y)
}

fn child<'i>(parent: &'i Item, key: &str) -> Option<&'i Item> {
    child_with_key(parent, key).map(|(_, item)| item)
}

fn child_with_key<'i>(parent: &'i Item, key: &str) -> Option<(&'i crate::Key, &'i Item)> {
    match parent {
        Item::Table(table) => table.get_key_value(key),
        Item::Value(Value::InlineTable(table)) => table.get_key_value(key),
        _ => None,
    }
}

fn navigate<'i>(root: &'i Item, path: &[String]) -> Option<&'i Item> {
    let mut current = root;
    for segment in path {
        current = child(current, segment)?;
    }
    Some(current)
}

fn navigate_mut<'i>(root: &'i mut Item, path: &[String]) -> Option<&'i mut Item> {
    let mut current = root;
    for segment in path {
        current = match current {
            Item::Table(table) => table.items.get_mut(segment.as_str())?,
            Item::Value(Value::InlineTable(table)) => table.items.get_mut(segment.as_str())?,
            _ => return None,
        };
    }
    Some(current)
}

fn validate(root: &Item, target: &Target) -> Result<(), ConvertError> {
    let (leaf, parent_path) = target.path.split_last().expect("empty paths are rejected");
    let parent_item = navigate(root, parent_path);
    let (key, item) = parent_item
        .and_then(|parent| child_with_key(parent, leaf))
        .ok_or_else(|| ConvertError::NotFound {
            path: target.path.clone(),
        })?;
    match target.direction {
        Direction::ToInline => {
            // The key decor is reset to value-position defaults; it must not
            // carry comments.
            check_key_decor(key, &target.path)?;
            validate_to_inline(item, &target.path)
        }
        Direction::ToTable => {
            validate_to_table(item, &target.path)?;
            if !matches!(parent_item, Some(Item::Table(_))) {
                return Err(ConvertError::UnsupportedNesting {
                    path: target.path.clone(),
                });
            }
            // Comment lines in the key prefix stay above the new `[header]`;
            // the suffix (padding before `=`) is reset and must be clean.
            if let Some(suffix) = key.leaf_decor().suffix() {
                check_raw(suffix, &target.path)?;
            }
            check_decor(key.dotted_decor(), &target.path)
        }
    }
}

fn validate_to_inline(item: &Item, path: &[String]) -> Result<(), ConvertError> {
    let Item::Table(table) = item else {
        return Err(ConvertError::WrongKind {
            path: path.to_vec(),
            expected: "table",
            found: item.type_name(),
        });
    };
    if table.is_dotted() {
        return Err(ConvertError::DottedTable {
            path: path.to_vec(),
        });
    }
    if table.is_implicit() {
        return Err(ConvertError::ImplicitTable {
            path: path.to_vec(),
        });
    }
    check_table(table, path)
}

fn validate_to_table(item: &Item, path: &[String]) -> Result<(), ConvertError> {
    let Item::Value(Value::InlineTable(table)) = item else {
        return Err(ConvertError::WrongKind {
            path: path.to_vec(),
            expected: "inline table",
            found: item.type_name(),
        });
    };
    if table.is_dotted() {
        return Err(ConvertError::DottedTable {
            path: path.to_vec(),
        });
    }
    if table.is_implicit() {
        return Err(ConvertError::ImplicitTable {
            path: path.to_vec(),
        });
    }
    // The decor before `{` must be plain whitespace; a trailing comment after
    // `}` migrates onto the new table header, everything else is rejected.
    if let Some(prefix) = table.decor().prefix() {
        check_raw(prefix, path)?;
    }
    if let Some(suffix) = table.decor().suffix() {
        if suffix.as_str().is_none() {
            // Cannot inspect the decor; refuse to guess.
            return Err(ConvertError::UnmovableComment {
                path: path.to_vec(),
            });
        }
    }
    check_raw(table.trailing(), path)?;
    for (key, item) in &table.items {
        let mut key_path = path.to_vec();
        key_path.push(key.get().to_owned());
        check_key_decor(key, &key_path)?;
        if let Item::Value(value) = item {
            check_value(value, &key_path)?;
        }
    }
    Ok(())
}

/// Reject comments anywhere in a table being inlined
fn check_table(table: &Table, path: &[String]) -> Result<(), ConvertError> {
    check_decor(table.decor(), path)?;
    for (key, item) in &table.items {
        let mut key_path = path.to_vec();
        key_path.push(key.get().to_owned());
        check_key_decor(key, &key_path)?;
        match item {
            Item::None => {}
            Item::Value(value) => check_value(value, &key_path)?,
            // Dotted-key proxies are part of this table's body
            Item::Table(child) if child.is_dotted() => check_table(child, &key_path)?,
            Item::Table(_) | Item::ArrayOfTables(_) => {
                return Err(ConvertError::ChildTables { path: key_path });
            }
        }
    }
    Ok(())
}

fn check_value(value: &Value, path: &[String]) -> Result<(), ConvertError> {
    check_decor(value.decor(), path)?;
    match value {
        Value::Array(array) => {
            check_raw(array.trailing(), path)?;
            for element in array.iter() {
                check_value(element, path)?;
            }
        }
        Value::InlineTable(table) => {
            check_raw(table.trailing(), path)?;
            for (key, item) in &table.items {
                let mut key_path = path.to_vec();
                key_path.push(key.get().to_owned());
                check_key_decor(key, &key_path)?;
                if let Item::Value(value) = item {
                    check_value(value, &key_path)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn check_key_decor(key: &crate::Key, path: &[String]) -> Result<(), ConvertError> {
    check_decor(key.leaf_decor(), path)?;
    check_decor(key.dotted_decor(), path)
}

fn check_decor(decor: &Decor, path: &[String]) -> Result<(), ConvertError> {
    if let Some(prefix) = decor.prefix() {
        check_raw(prefix, path)?;
    }
    if let Some(suffix) = decor.suffix() {
        check_raw(suffix, path)?;
    }
    Ok(())
}

fn check_raw(raw: &RawString, path: &[String]) -> Result<(), ConvertError> {
    match raw.as_str() {
        // Decor is whitespace and comments only, so `#` always starts a comment
        Some(text) if !text.contains('#') => Ok(()),
        // `None` means the decor cannot be inspected; refuse to guess
        _ => Err(ConvertError::UnmovableComment {
            path: path.to_vec(),
        }),
    }
}

fn convert_target(parent: &mut Item, leaf: &str, direction: Direction) {
    let items = match parent {
        Item::Table(table) => &mut table.items,
        Item::Value(Value::InlineTable(table)) => &mut table.items,
        _ => unreachable!("target was validated"),
    };
    let (key, item) = items
        .iter_mut2()
        .find(|(key, _)| key.get() == leaf)
        .expect("target was validated");
    match direction {
        Direction::ToInline => {
            // The key decor came from the `[header]`; reset it so the
            // `key = { ... }` line gets default spacing.
            key.leaf_decor_mut().clear();
            key.dotted_decor_mut().clear();
        }
        Direction::ToTable => {
            // Keep the prefix (comment lines above the key stay above the
            // new `[header]`); reset the suffix that padded ` = `.
            key.leaf_decor_mut().set_suffix("");
            key.dotted_decor_mut().clear();
        }
    }
    convert_item(item, direction);
}

fn convert_item(item: &mut Item, direction: Direction) {
    match direction {
        Direction::ToInline => {
            let Item::Table(table) = item else {
                unreachable!("target was validated");
            };
            let table = std::mem::take(table);
            let inline = table_into_inline(table);
            *item = Item::Value(Value::InlineTable(inline));
        }
        Direction::ToTable => {
            let Item::Value(Value::InlineTable(inline)) = item else {
                unreachable!("target was validated");
            };
            let suffix = migratable_suffix(inline.decor());
            let inline = std::mem::take(inline);
            let mut table = inline_into_table(inline);
            if let Some(suffix) = suffix {
                table.decor_mut().set_suffix(suffix);
            }
            *item = Item::Table(table);
        }
    }
}

/// A trailing line comment on the value moves onto the new table header
fn migratable_suffix(decor: &Decor) -> Option<String> {
    let suffix = decor.suffix()?.as_str()?;
    suffix.contains('#').then(|| suffix.to_owned())
}

fn table_into_inline(table: Table) -> InlineTable {
    let dotted = table.is_dotted();
    let mut items = table.items;
    for (key, item) in items.iter_mut2() {
        key.leaf_decor_mut().clear();
        key.dotted_decor_mut().clear();
        match item {
            Item::Value(value) => normalize_value(value),
            // Dotted-key proxies stay dotted so `a.b = 1` keeps its shape
            Item::Table(child) => {
                let child = std::mem::take(child);
                let child = table_into_inline(child);
                *item = Item::Value(Value::InlineTable(child));
            }
            Item::None | Item::ArrayOfTables(_) => {}
        }
    }
    let mut inline = InlineTable::with_pairs(items);
    inline.set_dotted(dotted);
    inline
}

fn inline_into_table(inline: InlineTable) -> Table {
    let dotted = inline.is_dotted();
    let mut items = inline.items;
    for (key, item) in items.iter_mut2() {
        key.leaf_decor_mut().clear();
        key.dotted_decor_mut().clear();
        match item {
            // Dotted-key proxies stay dotted so `a.b = 1` keeps its shape
            Item::Value(Value::InlineTable(child)) if child.is_dotted() => {
                let child = std::mem::take(child);
                let child = inline_into_table(child);
                *item = Item::Table(child);
            }
            Item::Value(value) => normalize_value(value),
            Item::None | Item::Table(_) | Item::ArrayOfTables(_) => {}
        }
    }
    let mut table = Table::with_pairs(items);
    table.set_dotted(dotted);
    table
}

/// Normalize whitespace-only decor; comments were rejected during validation
fn normalize_value(value: &mut Value) {
    value.decor_mut().clear();
    match value {
        Value::Array(array) => {
            array.set_trailing("");
            array.set_trailing_comma(false);
            for element in array.iter_mut() {
                normalize_value(element);
            }
        }
        Value::InlineTable(table) => {
            table.set_trailing("");
            table.set_trailing_comma(false);
            for (key, item) in table.items.iter_mut2() {
                key.leaf_decor_mut().clear();
                key.dotted_decor_mut().clear();
                if let Item::Value(value) = item {
                    normalize_value(value);
                }
            }
        }
        _ => {}
    }
}
