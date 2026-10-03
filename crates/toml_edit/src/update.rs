//! Explicit, format-preserving updates of integer values
//!
//! Robots reconfiguring a TOML document usually only need to touch a handful
//! of integer fields.  [`update_integers`] locates each target by its full
//! dotted key path and replaces *only* the integer lexical representation,
//! keeping the original base (decimal, hexadecimal, octal, binary), sign
//! convention, digit grouping and all surrounding whitespace and comments.
//! It never falls back to full-text replacement or generic reformatting.
//!
//! Every target is checked for existence and type before anything is
//! modified: if a single path is missing or does not hold an integer, the
//! document is left untouched and an [`UpdateError`] describing the offending
//! target is returned.
//!
//! # Example
//!
//! ```rust
//! # #[cfg(feature = "parse")] {
//! # #[cfg(feature = "display")] {
//! use toml_edit::{DocumentMut, IntegerUpdate, update_document_integers};
//!
//! let before = r#"limit = 0x00_FF # keep this comment
//! name = "worker"
//! "#;
//! let mut doc = before.parse::<DocumentMut>().unwrap();
//!
//! let report = update_document_integers(
//!     &mut doc,
//!     [IntegerUpdate::new(["limit"], 256)],
//! )
//! .unwrap();
//!
//! // The value keeps its hexadecimal base and the comment stays in place.
//! assert_eq!(
//!     doc.to_string(),
//!     "limit = 0x01_00 # keep this comment\nname = \"worker\"\n"
//! );
//! assert_eq!(report.fallbacks().count(), 0);
//! # }
//! # }
//! ```
//!
//! A non-decimal base cannot represent a negative value (TOML disallows a
//! sign before `0x`/`0o`/`0b`).  Such an update is legal but is rendered as a
//! signed decimal integer and is reported through
//! [`UpdateReport::fallbacks`] so callers can react to the base change.

use std::error::Error;
use std::fmt;

use crate::repr::Formatted;
use crate::table::TableLike;
use crate::Item;
use crate::Repr;
use crate::Table;
use crate::Value;

/// A single integer to write into a document.
///
/// Created with [`IntegerUpdate::new`]; the `path` is the full key path from
/// the table (or document root) passed to [`update_integers`].  Each path
/// component is one table key, so `["a", "b"]` navigates `a.b` while
/// `["a.b"]` selects a key literally named `"a.b"`.
#[derive(Clone, Debug)]
pub struct IntegerUpdate {
    path: Vec<String>,
    value: i64,
}

impl IntegerUpdate {
    /// Create an update setting `path` to `value`.
    pub fn new<I, S>(path: I, value: i64) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let path = path.into_iter().map(Into::into).collect();
        Self { path, value }
    }

    /// The full key path of the target integer.
    pub fn path(&self) -> &[String] {
        &self.path
    }

    /// The new integer value.
    pub fn value(&self) -> i64 {
        self.value
    }
}

/// Why a particular [`IntegerUpdate`] could not be applied.
///
/// Returned inside [`UpdateError`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateTargetError {
    /// No key with this path exists.
    ///
    /// This is also reported when an intermediate path component is missing
    /// or does not name a table.
    NotFound,
    /// The target exists but does not hold an integer.
    ///
    /// The string is the TOML type name of the value actually found, e.g.
    /// `"string"`, `"array"` or `"table"`.
    WrongType(String),
}

/// Error returned by [`update_integers`] when a target is missing or has the
/// wrong type.
///
/// Validation runs before any mutation, so this error guarantees that the
/// document is unchanged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateError {
    /// Path of the first target that failed validation.
    pub path: Vec<String>,
    /// What went wrong with `path`.
    pub kind: UpdateTargetError,
}

impl UpdateError {
    fn new(path: &[String], kind: UpdateTargetError) -> Self {
        Self {
            path: path.to_vec(),
            kind,
        }
    }
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let path = self.path.join(".");
        match &self.kind {
            UpdateTargetError::NotFound => {
                write!(f, "integer target `{path}` does not exist")
            }
            UpdateTargetError::WrongType(actual) => {
                write!(f, "target `{path}` is a {actual}, not an integer")
            }
        }
    }
}

impl Error for UpdateError {}

/// One target whose original base could not legally be kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateFallback {
    /// The original integer used a non-decimal base (`0x`/`0o`/`0b`), but the
    /// new value is negative.  Signed radix literals are invalid TOML, so the
    /// value was rendered as a signed decimal integer instead.
    NegativeForRadix,
}

/// Outcome of a successful call to [`update_integers`].
///
/// The report lists the targets whose requested base or grouping conventions
/// could not be followed and had to fall back to another legal lexical form.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UpdateReport {
    fallbacks: Vec<(Vec<String>, UpdateFallback)>,
}

impl UpdateReport {
    /// Iterate over `(path, fallback)` pairs for targets whose preferred
    /// representation could not be used.
    pub fn fallbacks(&self) -> impl Iterator<Item = (&[String], &UpdateFallback)> {
        self.fallbacks
            .iter()
            .map(|(path, fallback)| (path.as_slice(), fallback))
    }
}

/// Update several integer values in one atomic, format-preserving pass.
///
/// Each [`IntegerUpdate`] is located by its full key path.  Every target must
/// exist and hold an integer; if any target is missing or has another type,
/// no update (including earlier ones in the iterator) is applied and an
/// [`UpdateError`] is returned.
///
/// When possible the new value keeps the original integer base, sign
/// convention and digit grouping; only changed values are rewritten.  If the
/// original base cannot represent the new value (e.g. a hexadecimal integer
/// updated to a negative number), a legal representation is used instead and
/// the deviation is recorded in the returned [`UpdateReport`].  Whitespace,
/// comments and every other value in the document are preserved verbatim.
pub fn update_integers<I>(
    table: &mut dyn TableLike,
    updates: I,
) -> Result<UpdateReport, UpdateError>
where
    I: IntoIterator<Item = IntegerUpdate>,
{
    let updates: Vec<IntegerUpdate> = updates.into_iter().collect();

    // Validate everything first so the document is never partially mutated.
    for update in &updates {
        let path = &update.path;
        if path.is_empty() {
            return Err(UpdateError::new(path, UpdateTargetError::NotFound));
        }
        let item = resolve(table, path)
            .map_err(|depth| UpdateError::new(&path[..=depth], UpdateTargetError::NotFound))?;
        match item {
            Item::Value(Value::Integer(_)) => {}
            other => {
                return Err(UpdateError::new(
                    path,
                    UpdateTargetError::WrongType(other.type_name().to_owned()),
                ));
            }
        }
    }

    let mut report = UpdateReport::default();
    for update in updates {
        // Resolution was just validated against these same paths.
        let item = resolve_mut(table, &update.path).expect("validated integer path should resolve");
        let integer = match item {
            Item::Value(Value::Integer(integer)) => integer,
            _ => unreachable!("validated target should be an integer"),
        };
        if let Some(fallback) = integer.update_integer(update.value) {
            report.fallbacks.push((update.path, fallback));
        }
    }
    Ok(report)
}

/// Update integers on a mutable [`Table`].
///
/// This is a typed shorthand for [`update_integers`]; use the latter when
/// working with a generic [`TableLike`] such as an inline table.
pub fn update_table_integers<I>(table: &mut Table, updates: I) -> Result<UpdateReport, UpdateError>
where
    I: IntoIterator<Item = IntegerUpdate>,
{
    update_integers(table, updates)
}

/// Update integers at the root of a [`DocumentMut`](crate::DocumentMut).
///
/// This is a shorthand for [`update_integers`] on the document root table.
pub fn update_document_integers<I>(
    doc: &mut crate::DocumentMut,
    updates: I,
) -> Result<UpdateReport, UpdateError>
where
    I: IntoIterator<Item = IntegerUpdate>,
{
    update_integers(doc.as_table_mut(), updates)
}

/// `Err(depth)` means the component at `depth` could not be reached.
fn resolve<'a>(table: &'a dyn TableLike, path: &[String]) -> Result<&'a Item, usize> {
    let mut current: &dyn TableLike = table;
    let last = path.len() - 1;
    for (depth, key) in path.iter().enumerate() {
        let item = current.get(key).ok_or(depth)?;
        if depth == last {
            return Ok(item);
        }
        current = item.as_table_like().ok_or(depth)?;
    }
    unreachable!("non-empty path always returns or errors")
}

fn resolve_mut<'a>(table: &'a mut dyn TableLike, path: &[String]) -> Result<&'a mut Item, usize> {
    let mut current: &'a mut dyn TableLike = table;
    let last = path.len() - 1;
    for (depth, key) in path.iter().enumerate() {
        if depth == last {
            return current.get_mut(key).ok_or(depth);
        }
        let item = current.get_mut(key).ok_or(depth)?;
        current = item.as_table_like_mut().ok_or(depth)?;
    }
    unreachable!("non-empty path always returns or errors")
}

/// A parsed integer lexical style: base, sign, digit case and grouping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct IntegerStyle {
    base: Base,
    explicit_plus: bool,
    uppercase_digits: bool,
    /// Size of the right-most underscore-separated group when grouping was
    /// used (e.g. `0x00_FF` -> 2, `1_000` -> 3).
    group_width: Option<usize>,
    /// Total number of digits in the original literal.
    min_digits: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Base {
    Dec,
    Hex,
    Oct,
    Bin,
}

impl Base {
    fn radix(self) -> u32 {
        match self {
            Self::Dec => 10,
            Self::Hex => 16,
            Self::Oct => 8,
            Self::Bin => 2,
        }
    }

    fn prefix(self) -> &'static str {
        match self {
            Self::Dec => "",
            Self::Hex => "0x",
            Self::Oct => "0o",
            Self::Bin => "0b",
        }
    }
}

/// Extract the reusable lexical style of an integer literal.
fn parse_style(raw: &str) -> Option<IntegerStyle> {
    let bytes = raw.as_bytes();
    let mut idx = 0;
    let mut explicit_plus = false;
    match bytes.first()? {
        b'+' => {
            explicit_plus = true;
            idx += 1;
        }
        b'-' => {
            // A leading sign is consumed; negative non-decimal literals
            // are invalid TOML and would not have parsed.
            idx += 1;
        }
        _ => {}
    }

    let mut base = Base::Dec;
    // Radix prefixes are lowercase-only in TOML; hex *digits* may be uppercase.
    if bytes.len() >= idx + 2 && bytes[idx] == b'0' {
        match bytes[idx + 1] {
            b'x' => base = Base::Hex,
            b'o' => base = Base::Oct,
            b'b' => base = Base::Bin,
            _ => {}
        }
        if !matches!(base, Base::Dec) {
            idx += 2;
        }
    }

    let digits = raw.get(idx..)?;
    let chunks: Vec<&str> = digits.split('_').collect();
    let mut uppercase_digits = false;
    for chunk in &chunks {
        if chunk.is_empty() {
            return None;
        }
        for c in chunk.chars() {
            if !c.is_ascii_hexdigit() {
                return None;
            }
            if c.is_ascii_uppercase() {
                uppercase_digits = true;
            }
        }
    }
    // Every digit must be legal for the detected base.
    let valid_digit = |c: char| match base {
        Base::Dec => c.is_ascii_digit(),
        Base::Hex => c.is_ascii_hexdigit(),
        Base::Oct => ('0'..='7').contains(&c),
        Base::Bin => c == '0' || c == '1',
    };
    if !digits.chars().filter(|&c| c != '_').all(valid_digit) {
        return None;
    }
    // Decimal integers are a single `0` or start with a non-zero digit.
    if base == Base::Dec {
        let first = *chunks.first()?;
        if first.len() > 1 && first.starts_with('0') {
            return None;
        }
    }
    // Re-parse defensively: only preserve a style that decodes as an integer.
    let compact: String = digits.chars().filter(|&c| c != '_').collect();
    i64::from_str_radix(&compact, base.radix()).ok()?;

    let group_width = if chunks.len() > 1 {
        Some(chunks.last()?.len())
    } else {
        None
    };
    let min_digits = chunks.iter().map(|c| c.len()).sum();

    Some(IntegerStyle {
        base,
        explicit_plus,
        uppercase_digits,
        group_width,
        min_digits,
    })
}

/// Render `value` in the given style.  `None` means the original base cannot
/// legally carry this value and the caller must fall back to decimal.
fn render(style: IntegerStyle, value: i64) -> Option<String> {
    match style.base {
        Base::Dec => Some(render_decimal(style, value)),
        base => {
            if value < 0 {
                // Neither `-0x1` nor `0x-1` is valid TOML.
                None
            } else {
                Some(render_radix(style, base, value))
            }
        }
    }
}

fn render_decimal(style: IntegerStyle, value: i64) -> String {
    let (negative, magnitude) = if value < 0 {
        (true, (value as i128).unsigned_abs().to_string())
    } else {
        (false, (value as u64).to_string())
    };
    let digits = group_decimal(&magnitude, style.group_width.unwrap_or(3));
    let sign = if negative {
        "-"
    } else if style.explicit_plus {
        "+"
    } else {
        ""
    };
    format!("{sign}{digits}")
}

fn group_decimal(digits: &str, width: usize) -> String {
    if width == 0 || digits.len() <= width {
        return digits.to_owned();
    }
    let mut out = String::with_capacity(digits.len() + digits.len() / width);
    let first = digits.len() % width;
    let mut rest = digits;
    if first != 0 {
        out.push_str(&digits[..first]);
        rest = &digits[first..];
    }
    while !rest.is_empty() {
        if !out.is_empty() {
            out.push('_');
        }
        out.push_str(&rest[..width]);
        rest = &rest[width..];
    }
    out
}

fn render_radix(style: IntegerStyle, base: Base, value: i64) -> String {
    debug_assert!(value >= 0);
    let mut digits = radix_digits(value, base.radix());
    if style.uppercase_digits {
        digits.make_ascii_uppercase();
    }
    // Grouped literals keep their grouping; shorter values are zero-padded
    // to the original field width, aligned to a group multiple.
    let grouped = match style.group_width {
        Some(width) => pad_and_group(&digits, width, style.min_digits),
        None => digits,
    };
    format!("{prefix}{grouped}", prefix = base.prefix())
}

fn radix_digits(value: i64, radix: u32) -> String {
    if value == 0 {
        return "0".to_owned();
    }
    let mut value = value as u64;
    let mut out = String::new();
    while value != 0 {
        let rem = (value % radix as u64) as u8;
        let c = if rem < 10 {
            b'0' + rem
        } else {
            b'a' + (rem - 10)
        };
        out.push(c as char);
        value /= radix as u64;
    }
    out.chars().rev().collect()
}

/// Left-pad to a multiple of `width`, then insert underscores every `width`
/// digits from the right, reproducing fixed-width grouping such as `00_FF`.
fn pad_and_group(digits: &str, width: usize, min_digits: usize) -> String {
    if width == 0 {
        return digits.to_owned();
    }
    // The final field is at least the original width and a multiple of the
    // group width, so right-aligned groups stay intact.
    let target = min_digits.max(digits.len().div_ceil(width) * width);
    let pad = target.saturating_sub(digits.len());
    let mut padded = String::with_capacity(target);
    for _ in 0..pad {
        padded.push('0');
    }
    padded.push_str(digits);
    let mut out = String::new();
    for (pos, c) in padded.chars().enumerate() {
        if pos != 0 && pos % width == 0 {
            out.push('_');
        }
        out.push(c);
    }
    out
}

impl Formatted<i64> {
    /// Rewrite an integer value while preserving its lexical conventions.
    ///
    /// Returns `Some(fallback)` when the stored base could not legally carry
    /// `value` and another legal representation was used.
    pub(crate) fn update_integer(&mut self, value: i64) -> Option<UpdateFallback> {
        if *self.value() == value {
            return None;
        }
        let style = self
            .as_repr()
            .and_then(|repr| repr.as_raw().as_str())
            .and_then(parse_style);
        let (lexeme, fallback) = match style.and_then(|style| render(style, value)) {
            Some(lexeme) => (lexeme, None),
            None => {
                // Either there was no reusable style or its base cannot carry
                // the new value.  In the latter case keep decimal sign and
                // grouping conventions while dropping the radix prefix; a
                // fallback is only reported when a radix actually gave way.
                let (decimal, fallback) = match style {
                    Some(radix_style) if radix_style.base != Base::Dec => {
                        let decimal = IntegerStyle {
                            base: Base::Dec,
                            explicit_plus: radix_style.explicit_plus,
                            uppercase_digits: false,
                            group_width: radix_style.group_width,
                            min_digits: radix_style.min_digits,
                        };
                        (decimal, Some(UpdateFallback::NegativeForRadix))
                    }
                    _ => (
                        IntegerStyle {
                            base: Base::Dec,
                            explicit_plus: false,
                            uppercase_digits: false,
                            group_width: None,
                            min_digits: 1,
                        },
                        None,
                    ),
                };
                (render_decimal(decimal, value), fallback)
            }
        };
        self.set_value_unchecked(value);
        self.set_repr_unchecked(Repr::new_unchecked(lexeme));
        fallback
    }
}
