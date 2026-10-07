use std::borrow::Cow;
use std::str::FromStr;

#[cfg(feature = "display")]
use toml_writer::ToTomlKey as _;

use crate::repr::{Decor, Repr};

/// For Key/[`Value`][crate::Value] pairs under a [`Table`][crate::Table] header or inside an
/// [`InlineTable`][crate::InlineTable]
///
/// # Examples
///
/// ```notrust
/// [dependencies."nom"]
/// version = "5.0"
/// 'literal key' = "nonsense"
/// "basic string key" = 42
/// ```
///
/// There are 3 types of keys:
///
/// 1. Bare keys (`version` and `dependencies`)
///
/// 2. Basic quoted keys (`"basic string key"` and `"nom"`)
///
/// 3. Literal quoted keys (`'literal key'`)
///
/// For details see [toml spec](https://github.com/toml-lang/toml/#keyvalue-pair).
///
/// To parse a key use `FromStr` trait implementation: `"string".parse::<Key>()`.
#[derive(Debug)]
pub struct Key {
    key: String,
    pub(crate) repr: Option<Repr>,
    pub(crate) leaf_decor: Decor,
    pub(crate) dotted_decor: Decor,
}

impl Key {
    /// Create a new table key
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            repr: None,
            leaf_decor: Default::default(),
            dotted_decor: Default::default(),
        }
    }

    /// Parse a TOML key expression
    ///
    /// Unlike `"".parse<Key>()`, this supports dotted keys.
    #[cfg(feature = "parse")]
    pub fn parse(repr: &str) -> Result<Vec<Self>, crate::TomlError> {
        Self::try_parse_path(repr)
    }

    pub(crate) fn with_repr_unchecked(mut self, repr: Repr) -> Self {
        self.repr = Some(repr);
        self
    }

    /// While creating the `Key`, add `Decor` to it for the line entry
    pub fn with_leaf_decor(mut self, decor: Decor) -> Self {
        self.leaf_decor = decor;
        self
    }

    /// While creating the `Key`, add `Decor` to it for between dots
    pub fn with_dotted_decor(mut self, decor: Decor) -> Self {
        self.dotted_decor = decor;
        self
    }

    /// Access a mutable proxy for the `Key`.
    pub fn as_mut(&mut self) -> KeyMut<'_> {
        KeyMut { key: self }
    }

    /// Returns the parsed key value.
    pub fn get(&self) -> &str {
        &self.key
    }

    /// Returns key raw representation, if available.
    pub fn as_repr(&self) -> Option<&Repr> {
        self.repr.as_ref()
    }

    /// Returns the default raw representation.
    #[cfg(feature = "display")]
    pub fn default_repr(&self) -> Repr {
        let output = toml_writer::TomlKeyBuilder::new(&self.key)
            .as_default()
            .to_toml_key();
        Repr::new_unchecked(output)
    }

    /// Returns a raw representation.
    #[cfg(feature = "display")]
    pub fn display_repr(&self) -> Cow<'_, str> {
        self.as_repr()
            .and_then(|r| r.as_raw().as_str())
            .map(Cow::Borrowed)
            .unwrap_or_else(|| Cow::Owned(self.default_repr().into_raw().into_string().unwrap()))
    }

    /// Returns the surrounding whitespace for the line entry
    pub fn leaf_decor_mut(&mut self) -> &mut Decor {
        &mut self.leaf_decor
    }

    /// Returns the surrounding whitespace for between dots
    pub fn dotted_decor_mut(&mut self) -> &mut Decor {
        &mut self.dotted_decor
    }

    /// Returns the surrounding whitespace for the line entry
    pub fn leaf_decor(&self) -> &Decor {
        &self.leaf_decor
    }

    /// Returns the surrounding whitespace for between dots
    pub fn dotted_decor(&self) -> &Decor {
        &self.dotted_decor
    }

    /// The location within the original document
    ///
    /// This generally requires a [`Document`][crate::Document].
    pub fn span(&self) -> Option<std::ops::Range<usize>> {
        self.repr.as_ref().and_then(|r| r.span())
    }

    pub(crate) fn despan(&mut self, input: &str) {
        self.leaf_decor.despan(input);
        self.dotted_decor.despan(input);
        if let Some(repr) = &mut self.repr {
            repr.despan(input);
        }
    }

    /// Rename the key, preserving the existing quote style when the new name
    /// can be represented with it
    ///
    /// The surrounding whitespace ([`Decor`]) is left untouched.
    ///
    /// - A bare key stays bare when the new name is a valid bare key
    /// - A literal (`'...'`) key stays literal unless the new name cannot be
    ///   represented in a literal string
    /// - A basic (`"..."`) key stays basic
    ///
    /// When the existing style cannot represent the new name, this falls back
    /// to the [default representation][Key::default_repr].
    ///
    /// # Examples
    ///
    /// ```rust
    /// # #[cfg(feature = "parse")] {
    /// let mut key = "'literal key'".parse::<toml_edit::Key>().unwrap();
    /// key.rename("renamed");
    /// assert_eq!(key.to_string(), "'renamed'");
    ///
    /// let mut key = "'literal key'".parse::<toml_edit::Key>().unwrap();
    /// key.rename("it's");
    /// assert_eq!(key.to_string(), "\"it's\"");
    /// # }
    /// ```
    #[cfg(feature = "display")]
    pub fn rename(&mut self, new_name: &str) {
        let builder = toml_writer::TomlKeyBuilder::new(new_name);
        let raw = match self.repr.as_ref().and_then(|r| r.as_raw().as_str()) {
            Some(raw) if raw.starts_with('\'') => builder.as_literal().map(|k| k.to_toml_key()),
            Some(raw) if raw.starts_with('"') => Some(builder.as_basic().to_toml_key()),
            Some(_) => builder.as_unquoted().map(|k| k.to_toml_key()),
            None => None,
        };
        if self.repr.is_some() {
            let raw = raw.unwrap_or_else(|| builder.as_default().to_toml_key());
            self.repr = Some(Repr::new_unchecked(raw));
        }
        self.key.clear();
        self.key.push_str(new_name);
    }

    /// Auto formats the key.
    pub fn fmt(&mut self) {
        self.repr = None;
        self.leaf_decor.clear();
        self.dotted_decor.clear();
    }

    #[cfg(feature = "parse")]
    fn try_parse_simple(s: &str) -> Result<Self, crate::TomlError> {
        let source = toml_parser::Source::new(s);
        let mut sink = crate::error::TomlSink::<Option<_>>::new(source);
        let mut key = crate::parser::parse_key(source, &mut sink);
        if let Some(err) = sink.into_inner() {
            Err(err)
        } else {
            key.despan(s);
            Ok(key)
        }
    }

    #[cfg(feature = "parse")]
    fn try_parse_path(s: &str) -> Result<Vec<Self>, crate::TomlError> {
        let source = toml_parser::Source::new(s);
        let mut sink = crate::error::TomlSink::<Option<_>>::new(source);
        let mut keys = crate::parser::parse_key_path(source, &mut sink);
        if let Some(err) = sink.into_inner() {
            Err(err)
        } else {
            for key in &mut keys {
                key.despan(s);
            }
            Ok(keys)
        }
    }
}

impl Clone for Key {
    #[inline(never)]
    fn clone(&self) -> Self {
        Self {
            key: self.key.clone(),
            repr: self.repr.clone(),
            leaf_decor: self.leaf_decor.clone(),
            dotted_decor: self.dotted_decor.clone(),
        }
    }
}

impl std::ops::Deref for Key {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.get()
    }
}

impl std::borrow::Borrow<str> for Key {
    #[inline]
    fn borrow(&self) -> &str {
        self.get()
    }
}

impl std::hash::Hash for Key {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.get().hash(state);
    }
}

impl Ord for Key {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.get().cmp(other.get())
    }
}

impl PartialOrd for Key {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Eq for Key {}

impl PartialEq for Key {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        PartialEq::eq(self.get(), other.get())
    }
}

impl PartialEq<str> for Key {
    #[inline]
    fn eq(&self, other: &str) -> bool {
        PartialEq::eq(self.get(), other)
    }
}

impl PartialEq<&str> for Key {
    #[inline]
    fn eq(&self, other: &&str) -> bool {
        PartialEq::eq(self.get(), *other)
    }
}

impl PartialEq<String> for Key {
    #[inline]
    fn eq(&self, other: &String) -> bool {
        PartialEq::eq(self.get(), other.as_str())
    }
}

#[cfg(feature = "display")]
impl std::fmt::Display for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::encode::encode_key(self, f, None)
    }
}

#[cfg(feature = "parse")]
impl FromStr for Key {
    type Err = crate::TomlError;

    /// Tries to parse a key from a &str,
    /// if fails, tries as basic quoted key (surrounds with "")
    /// and then literal quoted key (surrounds with '')
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_parse_simple(s)
    }
}

impl<'b> From<&'b str> for Key {
    fn from(s: &'b str) -> Self {
        Self::new(s)
    }
}

impl<'b> From<&'b String> for Key {
    fn from(s: &'b String) -> Self {
        Self::new(s)
    }
}

impl From<String> for Key {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

#[doc(hidden)]
impl From<Key> for String {
    fn from(key: Key) -> Self {
        key.key
    }
}

/// A mutable reference to a [`Key`]'s formatting
#[derive(Debug, Eq, PartialEq, PartialOrd, Ord, Hash)]
pub struct KeyMut<'k> {
    key: &'k mut Key,
}

impl KeyMut<'_> {
    /// Returns the parsed key value.
    pub fn get(&self) -> &str {
        self.key.get()
    }

    /// Returns the raw representation, if available.
    pub fn as_repr(&self) -> Option<&Repr> {
        self.key.as_repr()
    }

    /// Returns the default raw representation.
    #[cfg(feature = "display")]
    pub fn default_repr(&self) -> Repr {
        self.key.default_repr()
    }

    /// Returns a raw representation.
    #[cfg(feature = "display")]
    pub fn display_repr(&self) -> Cow<'_, str> {
        self.key.display_repr()
    }

    /// Returns the surrounding whitespace for the line entry
    pub fn leaf_decor_mut(&mut self) -> &mut Decor {
        self.key.leaf_decor_mut()
    }

    /// Returns the surrounding whitespace for between dots
    pub fn dotted_decor_mut(&mut self) -> &mut Decor {
        self.key.dotted_decor_mut()
    }

    /// Returns the surrounding whitespace for the line entry
    pub fn leaf_decor(&self) -> &Decor {
        self.key.leaf_decor()
    }

    /// Returns the surrounding whitespace for between dots
    pub fn dotted_decor(&self) -> &Decor {
        self.key.dotted_decor()
    }

    /// Auto formats the key.
    pub fn fmt(&mut self) {
        self.key.fmt();
    }
}

impl std::ops::Deref for KeyMut<'_> {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.get()
    }
}

impl PartialEq<str> for KeyMut<'_> {
    #[inline]
    fn eq(&self, other: &str) -> bool {
        PartialEq::eq(self.get(), other)
    }
}

impl<'s> PartialEq<&'s str> for KeyMut<'s> {
    #[inline]
    fn eq(&self, other: &&str) -> bool {
        PartialEq::eq(self.get(), *other)
    }
}

impl PartialEq<String> for KeyMut<'_> {
    #[inline]
    fn eq(&self, other: &String) -> bool {
        PartialEq::eq(self.get(), other.as_str())
    }
}

#[cfg(feature = "display")]
impl std::fmt::Display for KeyMut<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.key, f)
    }
}

#[test]
#[cfg(feature = "parse")]
#[cfg(feature = "display")]
fn string_roundtrip() {
    Key::new("hello").to_string().parse::<Key>().unwrap();
}

#[test]
#[cfg(feature = "parse")]
#[cfg(feature = "display")]
fn original_interface_regression() {
    // `Key::new` keys have no repr and display with the default representation
    let key = Key::new("bare");
    assert!(key.as_repr().is_none());
    assert_eq!(key.display_repr(), "bare");
    let key = Key::new("not bare");
    assert_eq!(key.display_repr(), "\"not bare\"");

    // parsed keys keep their original representation
    let key = "'literal key'".parse::<Key>().unwrap();
    assert_eq!(key.get(), "literal key");
    assert_eq!(key.display_repr(), "'literal key'");
    let key = "\"basic key\"".parse::<Key>().unwrap();
    assert_eq!(key.display_repr(), "\"basic key\"");
}

#[test]
#[cfg(feature = "parse")]
#[cfg(feature = "display")]
fn rename_bare_key() {
    let mut key = "bare".parse::<Key>().unwrap();
    key.rename("renamed");
    assert_eq!(key.get(), "renamed");
    assert_eq!(key.to_string(), "renamed");

    // a new name that cannot be bare switches to the default representation
    let mut key = "bare".parse::<Key>().unwrap();
    key.rename("not bare");
    assert_eq!(key.to_string(), "\"not bare\"");
}

#[test]
#[cfg(feature = "parse")]
#[cfg(feature = "display")]
fn rename_literal_key() {
    let mut key = "'literal key'".parse::<Key>().unwrap();
    key.rename("renamed key");
    assert_eq!(key.get(), "renamed key");
    assert_eq!(key.to_string(), "'renamed key'");

    // a literal key cannot contain a single quote, switch quote style
    let mut key = "'literal key'".parse::<Key>().unwrap();
    key.rename("it's");
    assert_eq!(key.get(), "it's");
    assert_eq!(key.to_string(), "\"it's\"");
}

#[test]
#[cfg(feature = "parse")]
#[cfg(feature = "display")]
fn rename_basic_key() {
    let mut key = "\"basic key\"".parse::<Key>().unwrap();
    key.rename("renamed");
    assert_eq!(key.get(), "renamed");
    assert_eq!(key.to_string(), "\"renamed\"");
}

#[test]
#[cfg(feature = "display")]
fn rename_key_without_repr() {
    let mut key = Key::new("bare");
    key.rename("renamed");
    assert!(key.as_repr().is_none());
    assert_eq!(key.to_string(), "renamed");

    let mut key = Key::new("bare");
    key.rename("not bare");
    assert_eq!(key.to_string(), "\"not bare\"");
}
