//! Expansion of inline tables into regular tables.
//!
//! This module provides an entry point for explicitly converting selected
//! inline tables (and their inline subtables) into regular tables while
//! preserving values, key identity, and table hierarchy.

use std::error::Error;
use std::fmt;

use indexmap::map::MutableKeys;

use crate::key::Key;
use crate::table::TableLike;
use crate::{InlineTable, Item, RawString, Table, Value};

/// An error that can occur when expanding inline tables.
///
/// If any selected path is invalid, the entire batch of paths is rejected
/// atomically (no changes are made to the document).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExpandError {
    /// The path does not exist in the document.
    NotFound,
    /// The path does not point to an inline table.
    ///
    /// This can happen if the path points to a scalar value, a regular table,
    /// or an array of tables.
    NotInlineTable,
    /// The path points into an array, which is not supported.
    ///
    /// Inline tables nested inside arrays are not supported by this entry
    /// point.
    InArray,
}

impl fmt::Display for ExpandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => write!(f, "path not found"),
            Self::NotInlineTable => {
                write!(f, "path does not point to an inline table")
            }
            Self::InArray => {
                write!(f, "inline tables inside arrays are not supported")
            }
        }
    }
}

impl Error for ExpandError {}

/// Validates that the given path points to an inline table.
///
/// This is used to reject invalid paths before making any changes.
pub(crate) fn validate_path(root: &dyn TableLike, path: &[Key]) -> Result<(), ExpandError> {
    if path.is_empty() {
        return Err(ExpandError::NotFound);
    }
    let key = &path[0];
    let item = root.get(key.get()).ok_or(ExpandError::NotFound)?;
    if path.len() == 1 {
        // The final step must point to an inline table.
        match item {
            Item::Value(Value::InlineTable(_)) => Ok(()),
            Item::Value(Value::Array(arr)) => {
                // Inline tables inside arrays are not supported.
                if arr.iter().any(|v| v.is_inline_table()) {
                    Err(ExpandError::InArray)
                } else {
                    Err(ExpandError::NotInlineTable)
                }
            }
            _ => Err(ExpandError::NotInlineTable),
        }
    } else {
        // Intermediate steps must be navigable. Navigating through an array
        // is not supported because inline tables inside arrays are not.
        match item {
            Item::Table(t) => validate_path(t, &path[1..]),
            Item::Value(Value::InlineTable(t)) => validate_path(t, &path[1..]),
            Item::Value(Value::Array(_)) => Err(ExpandError::InArray),
            _ => Err(ExpandError::NotInlineTable),
        }
    }
}

/// Expands the inline table at the given path.
///
/// The inline table (and its inline subtables) are converted to regular
/// tables.
pub(crate) fn expand_path(root: &mut dyn TableLike, path: &[Key]) -> Result<(), ExpandError> {
    if path.is_empty() {
        return Err(ExpandError::NotFound);
    }
    let key = &path[0];
    if path.len() == 1 {
        // Clear the key's suffix decor so the table header is formatted
        // correctly (no leftover inline-table spacing). Preserve the prefix
        // decor only if it contains comments so that leading comments are kept.
        if let Some(mut k) = root.key_mut(key.get()) {
            let prefix = k.leaf_decor().prefix().cloned();
            k.leaf_decor_mut().clear();
            if let Some(prefix) = prefix {
                let preserve = prefix
                    .as_str()
                    .is_some_and(|s| s.contains('#') || s.contains('\n'));
                if preserve {
                    k.leaf_decor_mut().set_prefix(prefix);
                }
            }
        }
        let item = root.get_mut(key.get()).ok_or(ExpandError::NotFound)?;
        match item {
            Item::Value(Value::InlineTable(t)) => {
                let inline = std::mem::take(t);
                let expanded = expand_inline_table(inline);
                *item = Item::Table(expanded);
                Ok(())
            }
            _ => Err(ExpandError::NotInlineTable),
        }
    } else {
        let item = root.get_mut(key.get()).ok_or(ExpandError::NotFound)?;
        match item {
            Item::Table(t) => expand_path(t, &path[1..]),
            Item::Value(Value::InlineTable(t)) => expand_path(t, &path[1..]),
            Item::Value(Value::Array(_)) => Err(ExpandError::InArray),
            _ => Err(ExpandError::NotInlineTable),
        }
    }
}

/// Recursively expands an inline table and its inline subtables into regular
/// tables.
///
/// Values, key identity, and table hierarchy are preserved. Dotted literal
/// keys (e.g. `"a.b"`) are kept as single keys and are not split into
/// multi-level keys.
fn expand_inline_table(table: InlineTable) -> Table {
    // Preserve the inline table's prefix and suffix decor (e.g. leading or
    // trailing comments) so they can be moved to the table header and the
    // last value of the table body.
    let inline_prefix: Option<RawString> = table.decor().prefix().cloned();
    let inline_suffix: Option<RawString> = table.decor().suffix().cloned();
    let mut result = Table::with_pairs(table.items);

    // Preserve the inline table's prefix decor as the table's prefix decor so
    // that leading comments are placed before the table header. Only preserve
    // decor that contains newlines (i.e. comments); plain whitespace (e.g. the
    // space after `=`) is not needed in the table header context.
    if let Some(prefix) = inline_prefix {
        if let Some(prefix_str) = prefix.as_str() {
            if prefix_str.contains('\n') {
                result.decor_mut().set_prefix(prefix);
            }
        }
    }

    // Recursively expand inline subtables and adjust decor for the table body
    // context.
    let mut last_value_index: Option<usize> = None;
    for (i, (key, item)) in result.items.iter_mut2().enumerate() {
        // Clear the key's suffix decor so the table body defaults are used (no
        // leftover inline-table spacing). Preserve the prefix decor only if it
        // contains comments so that leading comments are kept.
        let prefix = key.leaf_decor().prefix().cloned();
        key.leaf_decor_mut().clear();
        if let Some(prefix) = prefix {
            let preserve = prefix
                .as_str()
                .is_some_and(|s| s.contains('#') || s.contains('\n'));
            if preserve {
                key.leaf_decor_mut().set_prefix(prefix);
            }
        }
        key.dotted_decor_mut().clear();
        // Recursively expand inline subtables.
        if let Item::Value(Value::InlineTable(sub)) = item {
            let sub = std::mem::take(sub);
            let expanded = expand_inline_table(sub);
            *item = Item::Table(expanded);
        }
        if item.is_value() {
            last_value_index = Some(i);
        }
    }

    // Move the inline table's suffix decor to the last value's suffix so that
    // trailing comments are preserved. If the suffix is only whitespace (e.g.
    // a space before the next closing brace) or absent, clear the last value's
    // suffix (the whitespace before the closing brace).
    if let Some(index) = last_value_index {
        if let Some((_, item)) = result.items.get_index_mut(index) {
            if let Item::Value(value) = item {
                let preserve_suffix = inline_suffix
                    .as_ref()
                    .is_some_and(|s| s.as_str().is_some_and(|s| s.contains('#')));
                if preserve_suffix {
                    if let Some(suffix) = inline_suffix {
                        value.decor_mut().set_suffix(suffix);
                    }
                } else {
                    value.decor_mut().set_suffix(RawString::default());
                }
            }
        }
    }

    result
}
