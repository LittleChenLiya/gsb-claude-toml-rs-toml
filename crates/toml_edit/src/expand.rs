//! Explicit expansion of selected inline tables into standard tables

use crate::table::TableLike;
use crate::{DocumentMut, InlineTable, Item, KeyMut, Table, Value};

/// Expansion operations on a [`DocumentMut`]
impl DocumentMut {
    /// Expand the inline tables at the selected key paths into standard tables
    ///
    /// For each selected path, the inline table it points to — and every inline table nested
    /// within it — is rewritten as a standard `[table]`, preserving values, logical key
    /// identity, and table hierarchy.  Everything not reachable from a selected path is left
    /// untouched.
    ///
    /// The expansion is not applied in place: on success a new [`DocumentMut`] is returned and
    /// `self` is left unchanged.
    ///
    /// # Supported scope
    ///
    /// - Each path is a sequence of *logical* keys: every segment names exactly one key, so a
    ///   literal key like `"a.b"` is a single segment and is never split into multiple levels.
    /// - The selected item must be an inline table value.  A path that selects an existing
    ///   standard table is a no-op.
    /// - Inline tables on the path above a selected table are expanded as well, since a
    ///   standard table cannot be represented inside an inline table.
    /// - Dotted-key proxies inside an expanded table keep their dotted form (e.g. `b.c = 1`
    ///   stays `b.c = 1` under the new `[a]` header).
    /// - Inline tables inside arrays are **not** supported: a path that selects an array, an
    ///   array of tables, or that traverses one is rejected.  Plain arrays are never turned
    ///   into arrays of tables.
    /// - The batch is atomic: every path is validated first and if any path is invalid, the
    ///   whole batch is rejected with an [`ExpandInlineError`] and no expansion is performed.
    ///
    /// The serialized result can be parsed back into the same data by the current parser.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[cfg(feature = "parse")] {
    /// let doc = "a = {b = {c = 1}, d = 2}".parse::<toml_edit::DocumentMut>()?;
    /// let expanded = doc.expand_inline_tables([["a"]])?;
    /// let expected = "[a]\nd = 2\n\n[a.b]\nc = 1\n";
    /// assert_eq!(expanded.to_string(), expected);
    /// // The original document is unchanged
    /// assert_eq!(doc.to_string(), "a = {b = {c = 1}, d = 2}\n");
    /// # }
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn expand_inline_tables<I, P, S>(&self, paths: I) -> Result<Self, ExpandInlineError>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<[S]>,
        S: AsRef<str>,
    {
        let paths: Vec<Vec<String>> = paths
            .into_iter()
            .map(|path| {
                path.as_ref()
                    .iter()
                    .map(|segment| segment.as_ref().to_owned())
                    .collect()
            })
            .collect();
        // Validate the whole batch before touching anything
        for path in &paths {
            validate_expand_path(self.as_table(), path)?;
        }
        let mut expanded = self.clone();
        for path in &paths {
            expand_path(expanded.as_table_mut(), path);
        }
        Ok(expanded)
    }
}

/// Checks that `path` selects an expandable table, without modifying anything
fn validate_expand_path(root: &Table, path: &[String]) -> Result<(), ExpandInlineError> {
    if path.is_empty() {
        return Err(ExpandInlineError::EmptyPath);
    }
    let mut current: &dyn TableLike = root;
    for (i, segment) in path.iter().enumerate() {
        let last = i + 1 == path.len();
        let Some(item) = current.get(segment) else {
            return Err(ExpandInlineError::InvalidPath {
                path: path.to_vec(),
                segment: i,
                found: None,
            });
        };
        match item {
            // Already a standard table: valid, expanding it is a no-op
            Item::Table(table) => {
                if !last {
                    current = table;
                }
            }
            Item::Value(Value::InlineTable(table)) => {
                if !last {
                    current = table;
                }
            }
            // Arrays (including inline tables inside them) and arrays of tables are
            // explicitly unsupported
            Item::Value(_) | Item::ArrayOfTables(_) | Item::None => {
                return Err(ExpandInlineError::InvalidPath {
                    path: path.to_vec(),
                    segment: i,
                    found: Some(item.type_name()),
                });
            }
        }
    }
    Ok(())
}

/// Applies a (pre-validated) expansion path
fn expand_path(table: &mut Table, path: &[String]) {
    let Some((segment, rest)) = path.split_first() else {
        return;
    };
    let Some((mut key, item)) = table.get_key_value_mut(segment) else {
        return;
    };
    if rest.is_empty() {
        if matches!(item, Item::Value(Value::InlineTable(_))) {
            convert_entry(&mut key, item);
        }
        return;
    }
    match item {
        Item::Table(child) => expand_path(child, rest),
        Item::Value(Value::InlineTable(_)) => {
            // Ancestor of a selected table: it has to become a standard table for the
            // selected expansion to be representable
            convert_entry(&mut key, item);
            if let Item::Table(child) = item {
                expand_path(child, rest);
            }
        }
        _ => {}
    }
}

/// Replaces an inline-table `item` with a standard [`Table`], moving the line-level comments
/// from the key and the value onto the new table header
fn convert_entry(key: &mut KeyMut<'_>, item: &mut Item) {
    let Item::Value(Value::InlineTable(inline)) = std::mem::replace(item, Item::None) else {
        return;
    };
    // The trailing comment of the `key = {...} # comment` line lives on the decor of the
    // inline table; move it behind the new `[key]` header
    let trailing_comment = inline
        .decor()
        .suffix()
        .and_then(|suffix| suffix.as_str())
        .filter(|suffix| suffix.contains('#'))
        .map(str::to_owned);
    let mut table = inline_to_table(inline);
    if let Some(trailing_comment) = trailing_comment {
        table.decor_mut().set_suffix(trailing_comment);
    }
    // Leading comments of the `key = {...}` line live on the decor of the key; move them in
    // front of the new `[key]` header.  Whitespace-only prefixes are dropped in favor of the
    // standard table spacing.
    let leading_comment = key
        .leaf_decor()
        .prefix()
        .and_then(|prefix| prefix.as_str())
        .filter(|prefix| prefix.contains('#'))
        .map(str::to_owned);
    if let Some(leading_comment) = leading_comment {
        table.decor_mut().set_prefix(leading_comment);
    }
    key.leaf_decor_mut().clear();
    key.dotted_decor_mut().clear();
    *item = Item::Table(table);
}

/// Recursively rewrites an [`InlineTable`] and every inline table nested within it as
/// standard [`Table`]s
fn inline_to_table(inline: InlineTable) -> Table {
    let dotted = inline.is_dotted();
    // `into_table` moves the key/value pairs over and normalizes their formatting; comments
    // cannot occur inside an inline table, so no comments are lost by this
    let mut table = inline.into_table();
    table.set_dotted(dotted);
    for (_, item) in table.iter_mut() {
        if matches!(item, Item::Value(Value::InlineTable(_))) {
            let Item::Value(Value::InlineTable(nested)) = std::mem::replace(item, Item::None)
            else {
                continue;
            };
            *item = Item::Table(inline_to_table(nested));
        }
    }
    table
}

/// An error occurred while expanding selected inline tables
///
/// See [`DocumentMut::expand_inline_tables`]
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExpandInlineError {
    /// A selected path was empty; the document root is always a standard table
    EmptyPath,
    /// A segment of a selected path did not resolve to an expandable table
    InvalidPath {
        /// The full selected path that was rejected
        path: Vec<String>,
        /// The offset of the segment that failed to resolve
        segment: usize,
        /// The type of item found at that segment, or [`None`] if the key does not exist
        found: Option<&'static str>,
    },
}

impl std::fmt::Display for ExpandInlineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyPath => {
                write!(f, "cannot expand inline table: selected path is empty")
            }
            Self::InvalidPath {
                path,
                segment,
                found,
            } => {
                write!(f, "cannot expand inline table at path `")?;
                let mut segments = path.iter();
                if let Some(first) = segments.next() {
                    write!(f, "{first}")?;
                }
                for segment in segments {
                    write!(f, ".{segment}")?;
                }
                write!(f, "`: ")?;
                match found {
                    Some(found) => write!(
                        f,
                        "segment `{}` resolved to {found}, not an inline table",
                        path[*segment],
                    ),
                    None => write!(f, "segment `{}` does not exist", path[*segment]),
                }
            }
        }
    }
}

impl std::error::Error for ExpandInlineError {}
