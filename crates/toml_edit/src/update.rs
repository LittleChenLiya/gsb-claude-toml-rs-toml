//! Format-preserving integer updates.
//!
//! This module provides [`IntegerUpdate`], an explicit tool for changing integer
//! values in a TOML document while preserving the original lexeme's radix, sign
//! convention, and underscore grouping style. When the new value cannot legally
//! be expressed in the original radix (e.g. a negative value with a hexadecimal
//! radix), the update falls back to decimal and reports the fallback.
//!
//! All targets in a batch are validated for existence and type before any
//! mutation is applied, so a rejected batch leaves the document untouched.

use std::fmt;

use crate::key::Key;
use crate::repr::Repr;
use crate::{DocumentMut, Item, TomlError, Value};

/// A request to update a single integer in a TOML document, located by its full
/// dotted key path.
///
/// # Examples
///
/// ```rust
/// # #[cfg(feature = "parse")] {
/// use toml_edit::IntegerUpdate;
///
/// let update = IntegerUpdate::new("limit", 256).unwrap();
/// assert_eq!(update.value(), 256);
///
/// let nested = IntegerUpdate::new("a.b.c", 42).unwrap();
/// let quoted = IntegerUpdate::new(r#""q"."r""#, 7).unwrap();
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct IntegerUpdate {
    path: Vec<Key>,
    value: i64,
}

impl IntegerUpdate {
    /// Create an update from a dotted key path and a new value.
    ///
    /// The path uses TOML key syntax: bare keys, quoted keys, and dotted keys
    /// are all supported (e.g. `limit`, `a.b.c`, `"q"."r"`).
    pub fn new(path: &str, value: i64) -> Result<Self, TomlError> {
        let path = Key::parse(path)?;
        Ok(Self { path, value })
    }

    /// The parsed key path.
    pub fn path(&self) -> &[Key] {
        &self.path
    }

    /// The new value to assign.
    pub fn value(&self) -> i64 {
        self.value
    }
}

/// The radix (base) of a TOML integer representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IntegerRadix {
    /// Decimal (base 10).
    #[default]
    Decimal,
    /// Hexadecimal (base 16), prefixed with `0x`.
    Hexadecimal,
    /// Octal (base 8), prefixed with `0o`.
    Octal,
    /// Binary (base 2), prefixed with `0b`.
    Binary,
}

impl IntegerRadix {
    /// The TOML prefix for this radix, or `""` for decimal.
    pub fn prefix(&self) -> &'static str {
        match self {
            Self::Decimal => "",
            Self::Hexadecimal => "0x",
            Self::Octal => "0o",
            Self::Binary => "0b",
        }
    }

    /// A human-readable description of the radix.
    pub fn description(&self) -> &'static str {
        match self {
            Self::Decimal => "decimal",
            Self::Hexadecimal => "hexadecimal",
            Self::Octal => "octal",
            Self::Binary => "binary",
        }
    }
}

impl fmt::Display for IntegerRadix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.description())
    }
}

/// The outcome of applying a batch of integer updates.
#[derive(Debug, Clone)]
pub struct UpdateReport {
    outcomes: Vec<UpdateOutcome>,
}

impl UpdateReport {
    /// The outcomes of the updates, in the order they were applied.
    pub fn outcomes(&self) -> &[UpdateOutcome] {
        &self.outcomes
    }

    /// Returns `true` if any update fell back from its original radix to decimal.
    pub fn fell_back(&self) -> bool {
        self.outcomes.iter().any(|o| o.fell_back)
    }
}

/// The outcome of a single integer update.
#[derive(Debug, Clone)]
pub struct UpdateOutcome {
    path: Vec<Key>,
    value: i64,
    radix: IntegerRadix,
    fell_back: bool,
}

impl UpdateOutcome {
    /// The key path that was updated.
    pub fn path(&self) -> &[Key] {
        &self.path
    }

    /// The value that was assigned.
    pub fn value(&self) -> i64 {
        self.value
    }

    /// The radix used in the final representation.
    pub fn radix(&self) -> IntegerRadix {
        self.radix
    }

    /// Whether the representation fell back to decimal.
    pub fn fell_back(&self) -> bool {
        self.fell_back
    }
}

/// An error that prevents a batch of integer updates from being applied.
#[derive(Debug, Clone)]
pub enum UpdateError {
    /// A key path could not be parsed as TOML key syntax.
    InvalidPath {
        /// The path string that failed to parse.
        path: String,
        /// The underlying parse error.
        source: TomlError,
    },
    /// A target key does not exist in the document.
    MissingKey {
        /// The path that was not found.
        path: Vec<Key>,
    },
    /// A target value is not an integer.
    NotAnInteger {
        /// The path that was found.
        path: Vec<Key>,
        /// The type name of the value that was found.
        found: String,
    },
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPath { path, source } => {
                write!(f, "invalid key path `{path}`: {source}")
            }
            Self::MissingKey { path } => {
                write!(
                    f,
                    "missing key `{}`",
                    path.iter().map(|k| k.get()).collect::<Vec<_>>().join(".")
                )
            }
            Self::NotAnInteger { path, found } => {
                write!(
                    f,
                    "expected integer at `{}`, found {found}",
                    path.iter().map(|k| k.get()).collect::<Vec<_>>().join(".")
                )
            }
        }
    }
}

impl std::error::Error for UpdateError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidPath { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl DocumentMut {
    /// Update a batch of integers atomically.
    ///
    /// All targets are first checked for existence and type. If any target is
    /// missing or not an integer, the entire batch is rejected and the document
    /// is left unchanged. Otherwise, all updates are applied.
    ///
    /// Each integer is reformatted to preserve its original radix, sign
    /// convention, and grouping style where possible. If the new value cannot
    /// be represented in the original radix (e.g. a negative value with a
    /// hexadecimal radix), the update falls back to decimal and reports it.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # #[cfg(all(feature = "parse", feature = "display"))] {
    /// use toml_edit::{DocumentMut, IntegerUpdate};
    ///
    /// let mut doc = "limit = 0x00_FF # comment\n".parse::<DocumentMut>().unwrap();
    /// let updates = vec![IntegerUpdate::new("limit", 256).unwrap()];
    /// let report = doc.update_integers(&updates).unwrap();
    /// assert_eq!(doc["limit"].as_integer(), Some(256));
    /// assert!(!report.fell_back());
    /// // The hexadecimal radix and the comment are preserved.
    /// assert!(doc.to_string().starts_with("limit = 0x01_00"));
    /// assert!(doc.to_string().contains("# comment"));
    /// # }
    /// ```
    pub fn update_integers(
        &mut self,
        updates: &[IntegerUpdate],
    ) -> Result<UpdateReport, UpdateError> {
        // Phase 1: validate all targets (existence + type). No mutation.
        for update in updates {
            let item = self.walk_path(&update.path)?;
            let value = item.as_value().ok_or_else(|| UpdateError::NotAnInteger {
                path: update.path.clone(),
                found: item.type_name().to_owned(),
            })?;
            if !value.is_integer() {
                return Err(UpdateError::NotAnInteger {
                    path: update.path.clone(),
                    found: value.type_name().to_owned(),
                });
            }
        }
        // Phase 2: apply all updates.
        let mut outcomes = Vec::with_capacity(updates.len());
        for update in updates {
            let item = self.walk_path_mut(&update.path)?;
            let value = item
                .as_value_mut()
                .expect("validated as an integer in phase 1");
            let outcome = apply_integer_update(value, update);
            outcomes.push(outcome);
        }
        Ok(UpdateReport { outcomes })
    }

    /// Update a single integer.
    ///
    /// This is a convenience wrapper around [`update_integers`](Self::update_integers)
    /// for a single target.
    pub fn update_integer(
        &mut self,
        path: &str,
        value: i64,
    ) -> Result<UpdateOutcome, UpdateError> {
        let update = IntegerUpdate::new(path, value).map_err(|source| UpdateError::InvalidPath {
            path: path.to_owned(),
            source,
        })?;
        let mut report = self.update_integers(&[update])?;
        Ok(report.outcomes.pop().expect("one update"))
    }

    fn walk_path(&self, path: &[Key]) -> Result<&Item, UpdateError> {
        let mut item = &self.root;
        for key in path {
            item = item
                .get(key.get())
                .ok_or_else(|| UpdateError::MissingKey {
                    path: path.to_vec(),
                })?;
        }
        Ok(item)
    }

    fn walk_path_mut(&mut self, path: &[Key]) -> Result<&mut Item, UpdateError> {
        let mut item = &mut self.root;
        for key in path {
            item = item
                .get_mut(key.get())
                .ok_or_else(|| UpdateError::MissingKey {
                    path: path.to_vec(),
                })?;
        }
        Ok(item)
    }
}

fn apply_integer_update(value: &mut Value, update: &IntegerUpdate) -> UpdateOutcome {
    let formatted = value
        .as_integer_mut()
        .expect("validated as an integer in phase 1");
    let old_repr = formatted.as_repr().map(|r| r.as_raw());
    let style = old_repr
        .and_then(|r| r.as_str())
        .map(analyze_integer)
        .unwrap_or_default();
    let (new_repr, fell_back) = encode_integer(update.value, &style);
    let radix = if fell_back {
        IntegerRadix::Decimal
    } else {
        style.radix
    };
    formatted.set_repr_unchecked(Repr::new_unchecked(new_repr));
    formatted.set_value(update.value);
    UpdateOutcome {
        path: update.path.clone(),
        value: update.value,
        radix,
        fell_back,
    }
}

/// The sign convention of an integer representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Sign {
    #[default]
    None,
    Plus,
    Minus,
}

/// The analyzed style of an integer representation.
#[derive(Debug, Clone, Default)]
struct IntegerStyle {
    radix: IntegerRadix,
    sign: Sign,
    /// The number of digits in the original representation (excluding
    /// underscores), used to preserve width when possible.
    digit_count: usize,
    /// The number of digits per underscore group, if grouping is present.
    group_size: Option<usize>,
    /// For hexadecimal, whether the digits are uppercase. `None` for
    /// non-hexadecimal or when there are no A-F digits.
    hex_upper: Option<bool>,
}

/// Analyze a raw integer representation to determine its radix, sign, grouping
/// style, and width.
fn analyze_integer(raw: &str) -> IntegerStyle {
    let bytes = raw.as_bytes();
    let mut i = 0;
    let mut sign = Sign::None;
    if let Some(&b) = bytes.first() {
        if b == b'+' {
            sign = Sign::Plus;
            i = 1;
        } else if b == b'-' {
            sign = Sign::Minus;
            i = 1;
        }
    }
    let rest = &raw[i..];
    let (radix, digits_start) = if rest.len() >= 2 {
        match &rest[..2] {
            "0x" | "0X" => (IntegerRadix::Hexadecimal, 2),
            "0o" | "0O" => (IntegerRadix::Octal, 2),
            "0b" | "0B" => (IntegerRadix::Binary, 2),
            _ => (IntegerRadix::Decimal, 0),
        }
    } else {
        (IntegerRadix::Decimal, 0)
    };
    let digits = &rest[digits_start..];
    let digit_count = digits.chars().filter(|c| *c != '_').count();
    let group_size = digits.rfind('_').map(|pos| digits.len() - pos - 1);
    let hex_upper = if radix == IntegerRadix::Hexadecimal {
        let mut has_upper = false;
        let mut has_lower = false;
        for c in digits.chars() {
            match c {
                'A'..='F' => has_upper = true,
                'a'..='f' => has_lower = true,
                _ => {}
            }
        }
        Some(has_upper && !has_lower)
    } else {
        None
    };
    IntegerStyle {
        radix,
        sign,
        digit_count,
        group_size,
        hex_upper,
    }
}

/// Encode `value` as a TOML integer representation, preserving `style` where
/// legal. Returns the raw representation and whether it fell back to decimal.
fn encode_integer(value: i64, style: &IntegerStyle) -> (String, bool) {
    let mut fell_back = false;
    let (radix, sign, group_size, digit_count) = match style.radix {
        IntegerRadix::Decimal => (
            style.radix,
            style.sign,
            style.group_size,
            style.digit_count,
        ),
        IntegerRadix::Hexadecimal | IntegerRadix::Octal | IntegerRadix::Binary => {
            if value < 0 {
                // Non-decimal radices cannot represent negative values; fall
                // back to decimal with no grouping.
                fell_back = true;
                (IntegerRadix::Decimal, Sign::Minus, None, 0)
            } else {
                (
                    style.radix,
                    Sign::None,
                    style.group_size,
                    style.digit_count,
                )
            }
        }
    };

    let abs_value = value.unsigned_abs();
    let (sign_str, mut digits) = match radix {
        IntegerRadix::Decimal => {
            let sign = resolve_decimal_sign(value, sign);
            let sign_str = match sign {
                Sign::None => "",
                Sign::Plus => "+",
                Sign::Minus => "-",
            };
            (sign_str, abs_value.to_string())
        }
        IntegerRadix::Hexadecimal => {
            let case = if style.hex_upper.unwrap_or(false) {
                HexCase::Upper
            } else {
                HexCase::Lower
            };
            ("", format_radix(abs_value, 16, case))
        }
        IntegerRadix::Octal => ("", format_radix(abs_value, 8, HexCase::Lower)),
        IntegerRadix::Binary => ("", format_radix(abs_value, 2, HexCase::Lower)),
    };

    // Preserve the original width for non-decimal radices by padding with
    // leading zeros.
    if radix != IntegerRadix::Decimal && digits.len() < digit_count {
        let padding = digit_count - digits.len();
        let mut padded = String::with_capacity(digit_count);
        for _ in 0..padding {
            padded.push('0');
        }
        padded.push_str(&digits);
        digits = padded;
    }

    // Apply the grouping style.
    if let Some(group_size) = group_size {
        digits = group_digits(&digits, group_size);
    }

    let prefix = radix.prefix();
    (format!("{sign_str}{prefix}{digits}"), fell_back)
}

/// Resolve the sign for a decimal representation based on the value's sign and
/// the original sign convention.
fn resolve_decimal_sign(value: i64, original: Sign) -> Sign {
    if value < 0 {
        Sign::Minus
    } else if value == 0 {
        original
    } else {
        match original {
            Sign::Plus => Sign::Plus,
            Sign::None | Sign::Minus => Sign::None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum HexCase {
    Upper,
    Lower,
}

fn format_radix(value: u64, radix: u32, case: HexCase) -> String {
    match radix {
        16 => match case {
            HexCase::Upper => format!("{value:X}"),
            HexCase::Lower => format!("{value:x}"),
        },
        8 => format!("{value:o}"),
        2 => format!("{value:b}"),
        _ => unreachable!("unsupported radix {radix}"),
    }
}

/// Group digits from the right with underscores every `group_size` digits.
fn group_digits(digits: &str, group_size: usize) -> String {
    if group_size == 0 || digits.len() <= group_size {
        return digits.to_owned();
    }
    let chars: Vec<char> = digits.chars().collect();
    let len = chars.len();
    let mut result = String::with_capacity(len + len / group_size);
    for (i, c) in chars.iter().enumerate() {
        if i > 0 && (len - i) % group_size == 0 {
            result.push('_');
        }
        result.push(*c);
    }
    result
}

#[cfg(test)]
#[cfg(feature = "parse")]
#[cfg(feature = "display")]
mod tests {
    use super::*;

    fn update(doc: &mut DocumentMut, path: &str, value: i64) -> UpdateReport {
        let updates = vec![IntegerUpdate::new(path, value).unwrap()];
        doc.update_integers(&updates).unwrap()
    }

    fn raw_value(doc: &DocumentMut, path: &str) -> String {
        let keys = Key::parse(path).unwrap();
        let mut item = doc.as_item();
        for key in &keys {
            item = item.get(key.get()).unwrap();
        }
        let value = item.as_value().unwrap();
        let formatted = match value {
            Value::Integer(f) => f,
            _ => panic!("not an integer"),
        };
        formatted
            .as_repr()
            .and_then(|r| r.as_raw().as_str())
            .expect("repr should be explicit after update")
            .to_owned()
    }

    fn reparse(doc: &DocumentMut, path: &str) -> i64 {
        let keys = Key::parse(path).unwrap();
        let mut item = doc.as_item();
        for key in &keys {
            item = item.get(key.get()).unwrap();
        }
        let value = item.as_value().unwrap();
        let formatted = match value {
            Value::Integer(f) => f,
            _ => panic!("not an integer"),
        };
        formatted
            .as_repr()
            .and_then(|r| r.as_raw().as_str())
            .expect("repr should be explicit after update")
            .parse::<Value>()
            .unwrap()
            .as_integer()
            .unwrap()
    }

    #[test]
    fn hex_preserves_radix_and_comment() {
        let mut doc = "limit = 0x00_FF # comment\n".parse::<DocumentMut>().unwrap();
        let report = update(&mut doc, "limit", 256);
        assert!(!report.fell_back());
        let outcome = &report.outcomes()[0];
        assert_eq!(outcome.radix(), IntegerRadix::Hexadecimal);
        assert!(!outcome.fell_back());
        assert_eq!(raw_value(&doc, "limit"), "0x01_00");
        assert!(doc.to_string().contains("# comment"));
        assert_eq!(reparse(&doc, "limit"), 256);
    }

    #[test]
    fn hex_lowercase_preserved() {
        let mut doc = "v = 0x00_ff\n".parse::<DocumentMut>().unwrap();
        update(&mut doc, "v", 256);
        assert_eq!(raw_value(&doc, "v"), "0x01_00");
    }

    #[test]
    fn negative_value_falls_back_to_decimal() {
        let mut doc = "v = 0xFF\n".parse::<DocumentMut>().unwrap();
        let report = update(&mut doc, "v", -1);
        assert!(report.fell_back());
        let outcome = &report.outcomes()[0];
        assert_eq!(outcome.radix(), IntegerRadix::Decimal);
        assert!(outcome.fell_back());
        assert_eq!(raw_value(&doc, "v"), "-1");
        assert_eq!(reparse(&doc, "v"), -1);
    }

    #[test]
    fn negative_value_with_underscore_falls_back_to_decimal() {
        let mut doc = "v = 0x00_FF\n".parse::<DocumentMut>().unwrap();
        let report = update(&mut doc, "v", -256);
        assert!(report.fell_back());
        assert_eq!(raw_value(&doc, "v"), "-256");
    }

    #[test]
    fn decimal_grouping_preserved() {
        let mut doc = "v = 1_000\n".parse::<DocumentMut>().unwrap();
        update(&mut doc, "v", 2000);
        assert_eq!(raw_value(&doc, "v"), "2_000");
    }

    #[test]
    fn decimal_sign_preserved() {
        let mut doc = "v = +42\n".parse::<DocumentMut>().unwrap();
        update(&mut doc, "v", 43);
        assert_eq!(raw_value(&doc, "v"), "+43");
    }

    #[test]
    fn negative_decimal_sign_preserved() {
        let mut doc = "v = -42\n".parse::<DocumentMut>().unwrap();
        update(&mut doc, "v", -43);
        assert_eq!(raw_value(&doc, "v"), "-43");
    }

    #[test]
    fn octal_preserved() {
        let mut doc = "v = 0o77\n".parse::<DocumentMut>().unwrap();
        update(&mut doc, "v", 64);
        assert_eq!(raw_value(&doc, "v"), "0o100");
    }

    #[test]
    fn binary_grouping_preserved() {
        let mut doc = "v = 0b1111_0000\n".parse::<DocumentMut>().unwrap();
        update(&mut doc, "v", 0b1111_0001);
        assert_eq!(raw_value(&doc, "v"), "0b1111_0001");
    }

    #[test]
    fn dotted_key_path() {
        let mut doc = "[a.b]\nc = 5\n".parse::<DocumentMut>().unwrap();
        update(&mut doc, "a.b.c", 42);
        assert_eq!(doc["a"]["b"]["c"].as_integer(), Some(42));
        assert_eq!(raw_value(&doc, "a.b.c"), "42");
    }

    #[test]
    fn dotted_key_path_inline() {
        let mut doc = "a = { b = { c = 5 } }\n".parse::<DocumentMut>().unwrap();
        update(&mut doc, "a.b.c", 42);
        assert_eq!(doc["a"]["b"]["c"].as_integer(), Some(42));
    }

    #[test]
    fn quoted_key_path() {
        let mut doc = "[q.\"r\"]\nv = 7\n".parse::<DocumentMut>().unwrap();
        update(&mut doc, "q.r.v", 9);
        assert_eq!(doc["q"]["r"]["v"].as_integer(), Some(9));
    }

    #[test]
    fn quoted_dotted_key_path() {
        let mut doc = "\"q\".\"r\" = 7\n".parse::<DocumentMut>().unwrap();
        update(&mut doc, r#""q"."r""#, 9);
        assert_eq!(doc["q"]["r"].as_integer(), Some(9));
    }

    #[test]
    fn i64_boundaries() {
        // `min` starts as hexadecimal, so updating to i64::MIN (negative) must
        // fall back to decimal. `max` starts as decimal and stays decimal.
        let mut doc = "min = 0x00\nmax = 0\n".parse::<DocumentMut>().unwrap();
        let updates = vec![
            IntegerUpdate::new("min", i64::MIN).unwrap(),
            IntegerUpdate::new("max", i64::MAX).unwrap(),
        ];
        let report = doc.update_integers(&updates).unwrap();
        assert_eq!(doc["min"].as_integer(), Some(i64::MIN));
        assert_eq!(doc["max"].as_integer(), Some(i64::MAX));
        assert!(report.outcomes()[0].fell_back());
        assert_eq!(raw_value(&doc, "min"), i64::MIN.to_string());
        assert!(!report.outcomes()[1].fell_back());
        assert_eq!(raw_value(&doc, "max"), i64::MAX.to_string());
        assert_eq!(reparse(&doc, "min"), i64::MIN);
        assert_eq!(reparse(&doc, "max"), i64::MAX);
    }

    #[test]
    fn i64_max_hex() {
        let mut doc = "v = 0x7FFF_FFFF\n".parse::<DocumentMut>().unwrap();
        update(&mut doc, "v", i64::MAX);
        assert_eq!(doc["v"].as_integer(), Some(i64::MAX));
        assert_eq!(raw_value(&doc, "v"), "0x7FFF_FFFF_FFFF_FFFF");
    }

    #[test]
    fn batch_rejected_when_one_target_is_string() {
        let mut doc = "a = 1\nb = \"hello\"\n".parse::<DocumentMut>().unwrap();
        let before = doc.to_string();
        let updates = vec![
            IntegerUpdate::new("a", 2).unwrap(),
            IntegerUpdate::new("b", 3).unwrap(),
        ];
        let err = doc.update_integers(&updates).unwrap_err();
        assert!(matches!(err, UpdateError::NotAnInteger { .. }));
        assert_eq!(doc.to_string(), before);
        assert_eq!(doc["a"].as_integer(), Some(1));
        assert_eq!(doc["b"].as_str(), Some("hello"));
    }

    #[test]
    fn batch_rejected_when_key_missing() {
        let mut doc = "a = 1\n".parse::<DocumentMut>().unwrap();
        let before = doc.to_string();
        let updates = vec![
            IntegerUpdate::new("a", 2).unwrap(),
            IntegerUpdate::new("b", 3).unwrap(),
        ];
        let err = doc.update_integers(&updates).unwrap_err();
        assert!(matches!(err, UpdateError::MissingKey { .. }));
        assert_eq!(doc.to_string(), before);
    }

    #[test]
    fn batch_applied_atomically() {
        let mut doc = "a = 1\nb = 2\n".parse::<DocumentMut>().unwrap();
        let updates = vec![
            IntegerUpdate::new("a", 10).unwrap(),
            IntegerUpdate::new("b", 20).unwrap(),
        ];
        let report = doc.update_integers(&updates).unwrap();
        assert_eq!(doc["a"].as_integer(), Some(10));
        assert_eq!(doc["b"].as_integer(), Some(20));
        assert_eq!(report.outcomes().len(), 2);
    }

    #[test]
    fn update_integer_convenience() {
        let mut doc = "v = 1\n".parse::<DocumentMut>().unwrap();
        let outcome = doc.update_integer("v", 2).unwrap();
        assert_eq!(outcome.value(), 2);
        assert_eq!(doc["v"].as_integer(), Some(2));
    }

    #[test]
    fn update_integer_invalid_path() {
        let mut doc = "v = 1\n".parse::<DocumentMut>().unwrap();
        let err = doc.update_integer("a..b", 2).unwrap_err();
        assert!(matches!(err, UpdateError::InvalidPath { .. }));
    }

    #[test]
    fn no_repr_uses_decimal() {
        let mut doc = DocumentMut::new();
        doc["v"] = crate::value(42i64);
        let report = update(&mut doc, "v", 43);
        assert!(!report.fell_back());
        assert_eq!(raw_value(&doc, "v"), "43");
    }

    #[test]
    fn comments_and_other_values_preserved() {
        let mut doc = "# top comment\na = 1 # inline\nb = \"string\"\nc = [1, 2, 3]\n"
            .parse::<DocumentMut>()
            .unwrap();
        update(&mut doc, "a", 2);
        assert!(doc.to_string().contains("# top comment"));
        assert!(doc.to_string().contains("# inline"));
        assert_eq!(doc["b"].as_str(), Some("string"));
        assert_eq!(doc["c"][0].as_integer(), Some(1));
        assert_eq!(doc["a"].as_integer(), Some(2));
    }
}
