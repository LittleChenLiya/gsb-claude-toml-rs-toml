use std::iter::FromIterator;
use std::mem;

use crate::repr::Decor;
use crate::value::{DEFAULT_LEADING_VALUE_DECOR, DEFAULT_VALUE_DECOR};
use crate::{Item, RawString, Value};

/// A TOML [`Value`] that contains a sequence of [`Value`]s
#[derive(Debug, Default, Clone)]
pub struct Array {
    // `trailing` represents whitespaces, newlines
    // and comments in an empty array or after the trailing comma
    trailing: RawString,
    trailing_comma: bool,
    // prefix before `[` and suffix after `]`
    decor: Decor,
    pub(crate) span: Option<std::ops::Range<usize>>,
    // always Vec<Item::Value>
    pub(crate) values: Vec<Item>,
}

/// Options for [`Array::remove_with`]
///
/// # Examples
///
/// ```rust
/// let options = toml_edit::RemoveOptions::new().preserve_comments(true);
/// ```
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RemoveOptions {
    preserve_comments: bool,
}

impl RemoveOptions {
    /// Create options with the default behavior: a removed value's comments are removed
    /// along with it
    pub const fn new() -> Self {
        Self {
            preserve_comments: false,
        }
    }

    /// Keep a removed value's standalone comments by moving them to the next value, or to
    /// the end of the array when removing the last value
    ///
    /// Inline comments on the removed value's own line are always removed along with the
    /// value.
    pub const fn preserve_comments(mut self, yes: bool) -> Self {
        self.preserve_comments = yes;
        self
    }
}

/// An owned iterator type over [`Array`]'s [`Value`]s
pub type ArrayIntoIter = Box<dyn Iterator<Item = Value>>;
/// An iterator type over [`Array`]'s [`Value`]s
pub type ArrayIter<'a> = Box<dyn Iterator<Item = &'a Value> + 'a>;
/// An iterator type over [`Array`]'s [`Value`]s
pub type ArrayIterMut<'a> = Box<dyn Iterator<Item = &'a mut Value> + 'a>;

/// Constructors
///
/// See also `FromIterator`
impl Array {
    /// Create an empty `Array`
    ///
    /// # Examples
    ///
    /// ```rust
    /// let mut arr = toml_edit::Array::new();
    /// ```
    pub fn new() -> Self {
        Default::default()
    }

    pub(crate) fn with_vec(values: Vec<Item>) -> Self {
        Self {
            values,
            ..Default::default()
        }
    }
}

/// Formatting
impl Array {
    /// Auto formats the array.
    pub fn fmt(&mut self) {
        decorate_array(self);
    }

    /// Set whether the array will use a trailing comma
    pub fn set_trailing_comma(&mut self, yes: bool) {
        self.trailing_comma = yes;
    }

    /// Whether the array will use a trailing comma
    pub fn trailing_comma(&self) -> bool {
        self.trailing_comma
    }

    /// Set whitespace after last element
    pub fn set_trailing(&mut self, trailing: impl Into<RawString>) {
        self.trailing = trailing.into();
    }

    /// Whitespace after last element
    pub fn trailing(&self) -> &RawString {
        &self.trailing
    }

    /// Returns the surrounding whitespace
    pub fn decor_mut(&mut self) -> &mut Decor {
        &mut self.decor
    }

    /// Returns the surrounding whitespace
    pub fn decor(&self) -> &Decor {
        &self.decor
    }

    /// The location within the original document
    ///
    /// This generally requires a [`Document`][crate::Document].
    pub fn span(&self) -> Option<std::ops::Range<usize>> {
        self.span.clone()
    }

    pub(crate) fn despan(&mut self, input: &str) {
        self.span = None;
        self.decor.despan(input);
        self.trailing.despan(input);
        for value in &mut self.values {
            value.despan(input);
        }
    }
}

impl Array {
    /// Returns an iterator over all values.
    pub fn iter(&self) -> ArrayIter<'_> {
        Box::new(self.iter_values())
    }

    pub(crate) fn iter_values(&self) -> impl Iterator<Item = &Value> {
        self.values.iter().filter_map(Item::as_value)
    }

    /// Returns an iterator over all values.
    pub fn iter_mut(&mut self) -> ArrayIterMut<'_> {
        Box::new(self.values.iter_mut().filter_map(Item::as_value_mut))
    }

    /// Returns the length of the underlying Vec.
    ///
    /// In some rare cases, placeholder elements will exist.  For a more accurate count, call
    /// `a.iter().count()`
    ///
    /// # Examples
    ///
    /// ```rust
    /// let mut arr = toml_edit::Array::new();
    /// arr.push(1);
    /// arr.push("foo");
    /// assert_eq!(arr.len(), 2);
    /// ```
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Return true if `self.len() == 0`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let mut arr = toml_edit::Array::new();
    /// assert!(arr.is_empty());
    ///
    /// arr.push(1);
    /// arr.push("foo");
    /// assert!(! arr.is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Clears the array, removing all values. Keeps the allocated memory for reuse.
    pub fn clear(&mut self) {
        self.values.clear();
    }

    /// Returns a reference to the value at the given index, or `None` if the index is out of
    /// bounds.
    pub fn get(&self, index: usize) -> Option<&Value> {
        self.values.get(index).and_then(Item::as_value)
    }

    /// Returns a reference to the value at the given index, or `None` if the index is out of
    /// bounds.
    pub fn get_mut(&mut self, index: usize) -> Option<&mut Value> {
        self.values.get_mut(index).and_then(Item::as_value_mut)
    }

    /// Appends a new value to the end of the array, applying default formatting to it.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let mut arr = toml_edit::Array::new();
    /// arr.push(1);
    /// arr.push("foo");
    /// ```
    pub fn push<V: Into<Value>>(&mut self, v: V) {
        self.values.push(Item::Value(v.into()));
    }

    /// Appends a new, already formatted value to the end of the array.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # #[cfg(feature = "parse")] {
    /// let formatted_value = "'literal'".parse::<toml_edit::Value>().unwrap();
    /// let mut arr = toml_edit::Array::new();
    /// arr.push_formatted(formatted_value);
    /// # }
    /// ```
    pub fn push_formatted(&mut self, v: Value) {
        self.values.push(Item::Value(v));
    }

    /// Inserts an element at the given position within the array, applying default formatting to
    /// it and shifting all values after it to the right.
    ///
    /// # Panics
    ///
    /// Panics if `index > len`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let mut arr = toml_edit::Array::new();
    /// arr.push(1);
    /// arr.push("foo");
    ///
    /// arr.insert(0, "start");
    /// ```
    pub fn insert<V: Into<Value>>(&mut self, index: usize, v: V) {
        self.values.insert(index, Item::Value(v.into()));
    }

    /// Inserts an already formatted value at the given position within the array, shifting all
    /// values after it to the right.
    ///
    /// # Panics
    ///
    /// Panics if `index > len`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # #[cfg(feature = "parse")] {
    /// let mut arr = toml_edit::Array::new();
    /// arr.push(1);
    /// arr.push("foo");
    ///
    /// let formatted_value = "'start'".parse::<toml_edit::Value>().unwrap();
    /// arr.insert_formatted(0, formatted_value);
    /// # }
    /// ```
    pub fn insert_formatted(&mut self, index: usize, v: Value) {
        self.values.insert(index, Item::Value(v));
    }

    /// Replaces the element at the given position within the array, preserving existing formatting.
    ///
    /// # Panics
    ///
    /// Panics if `index >= len`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let mut arr = toml_edit::Array::new();
    /// arr.push(1);
    /// arr.push("foo");
    ///
    /// arr.replace(0, "start");
    /// ```
    pub fn replace<V: Into<Value>>(&mut self, index: usize, v: V) -> Value {
        // Read the existing value's decor and preserve it.
        let existing_decor = self
            .get(index)
            .unwrap_or_else(|| panic!("index {} out of bounds (len = {})", index, self.len()))
            .decor();
        let mut value = v.into();
        *value.decor_mut() = existing_decor.clone();
        self.replace_formatted(index, value)
    }

    /// Replaces the element at the given position within the array with an already formatted value.
    ///
    /// # Panics
    ///
    /// Panics if `index >= len`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # #[cfg(feature = "parse")] {
    /// let mut arr = toml_edit::Array::new();
    /// arr.push(1);
    /// arr.push("foo");
    ///
    /// let formatted_value = "'start'".parse::<toml_edit::Value>().unwrap();
    /// arr.replace_formatted(0, formatted_value);
    /// # }
    /// ```
    pub fn replace_formatted(&mut self, index: usize, v: Value) -> Value {
        match mem::replace(&mut self.values[index], Item::Value(v)) {
            Item::Value(old_value) => old_value,
            x => panic!("non-value item {x:?} in an array"),
        }
    }

    /// Removes the value at the given index.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let mut arr = toml_edit::Array::new();
    /// arr.push(1);
    /// arr.push("foo");
    ///
    /// arr.remove(0);
    /// assert_eq!(arr.len(), 1);
    /// ```
    pub fn remove(&mut self, index: usize) -> Value {
        let removed = self.values.remove(index);
        match removed {
            Item::Value(v) => v,
            x => panic!("non-value item {x:?} in an array"),
        }
    }

    /// Removes the value at the given index, controlling how its comments are handled with
    /// [`RemoveOptions`].
    ///
    /// Unlike [`Array::remove`], an out-of-bounds `index` returns `None` and leaves the
    /// array untouched.
    ///
    /// With the default options this behaves like [`Array::remove`].  With
    /// [`RemoveOptions::preserve_comments`], comments are re-attributed as follows:
    ///
    /// - Standalone comments on the lines above the removed value move to the next value,
    ///   or to the end of the array when removing the last value.
    /// - An inline comment on the removed value's own line is removed along with the
    ///   value, whether it was written before or after the separating comma.
    /// - An inline comment sharing a line with the *previous* value stays with that value.
    ///
    /// All other values keep their existing formatting.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # #[cfg(feature = "parse")] {
    /// let mut value = "[\n    # about one\n    1, # inline one\n    # about two\n    2, # inline two\n]"
    ///     .parse::<toml_edit::Value>()
    ///     .unwrap();
    /// let array = value.as_array_mut().unwrap();
    ///
    /// let removed = array
    ///     .remove_with(1, toml_edit::RemoveOptions::new().preserve_comments(true))
    ///     .unwrap();
    /// assert_eq!(removed.as_integer(), Some(2));
    /// assert_eq!(array.to_string(), "[\n    # about one\n    1, # inline one\n    # about two\n]");
    ///
    /// // Out-of-bounds indices leave the array untouched
    /// assert!(array.remove_with(10, toml_edit::RemoveOptions::new()).is_none());
    /// # }
    /// ```
    pub fn remove_with(&mut self, index: usize, options: RemoveOptions) -> Option<Value> {
        let item = self.values.get(index)?;
        let value = match item {
            Item::Value(v) => v,
            x => panic!("non-value item {x:?} in an array"),
        };
        let mut comments = String::new();
        if options.preserve_comments {
            if let Some(prefix) = value.decor().prefix().and_then(RawString::as_str) {
                comments = extract_preservable_comments(prefix);
            }
        }
        let removed = match self.values.remove(index) {
            Item::Value(v) => v,
            x => panic!("non-value item {x:?} in an array"),
        };
        if options.preserve_comments {
            // The text after the removed value's comma belongs to the removed value's
            // line: an inline comment there is removed along with the value.
            strip_inline_comment_head(self.following_decor_mut(index));
            if !comments.is_empty() {
                // Detached comments move to the next value, or to the end of the array
                // when the last value was removed.
                match self.following_decor_mut(index) {
                    FollowingDecor::Prefix(decor) => {
                        let mut prefix = comments;
                        if let Some(existing) = decor.prefix().and_then(RawString::as_str) {
                            trim_redundant_newline(&mut prefix, existing);
                            prefix.push_str(existing);
                        }
                        decor.set_prefix(prefix);
                    }
                    FollowingDecor::Trailing(trailing) => {
                        let mut new_trailing = comments;
                        if let Some(existing) = trailing.as_str() {
                            trim_redundant_newline(&mut new_trailing, existing);
                            new_trailing.push_str(existing);
                        }
                        *trailing = RawString::from(new_trailing);
                    }
                }
            }
        }
        Some(removed)
    }

    /// Retains only the values specified by the `keep` predicate.
    ///
    /// In other words, remove all values for which `keep(&value)` returns `false`.
    ///
    /// This method operates in place, visiting each element exactly once in the
    /// original order, and preserves the order of the retained elements.
    pub fn retain<F>(&mut self, mut keep: F)
    where
        F: FnMut(&Value) -> bool,
    {
        self.values
            .retain(|item| item.as_value().map(&mut keep).unwrap_or(false));
    }

    /// Sorts the slice with a comparator function.
    ///
    /// This sort is stable (i.e., does not reorder equal elements) and *O*(*n* \* log(*n*)) worst-case.
    ///
    /// The comparator function must define a total ordering for the elements in the slice. If
    /// the ordering is not total, the order of the elements is unspecified. An order is a
    /// total order if it is (for all `a`, `b` and `c`):
    ///
    /// * total and antisymmetric: exactly one of `a < b`, `a == b` or `a > b` is true, and
    /// * transitive, `a < b` and `b < c` implies `a < c`. The same must hold for both `==` and `>`.
    ///
    /// For example, while [`f64`] doesn't implement [`Ord`] because `NaN != NaN`, we can use
    /// `partial_cmp` as our sort function when we know the slice doesn't contain a `NaN`.
    #[inline]
    pub fn sort_by<F>(&mut self, mut compare: F)
    where
        F: FnMut(&Value, &Value) -> std::cmp::Ordering,
    {
        self.values.sort_by(move |lhs, rhs| {
            let lhs = lhs.as_value();
            let rhs = rhs.as_value();
            match (lhs, rhs) {
                (None, None) => std::cmp::Ordering::Equal,
                (Some(_), None) => std::cmp::Ordering::Greater,
                (None, Some(_)) => std::cmp::Ordering::Less,
                (Some(lhs), Some(rhs)) => compare(lhs, rhs),
            }
        });
    }

    /// Sorts the array with a key extraction function.
    ///
    /// This sort is stable (i.e., does not reorder equal elements) and *O*(*m* \* *n* \* log(*n*))
    /// worst-case, where the key function is *O*(*m*).
    #[inline]
    pub fn sort_by_key<K, F>(&mut self, mut f: F)
    where
        F: FnMut(&Value) -> K,
        K: Ord,
    {
        #[allow(clippy::manual_map)] // needed for lifetimes
        self.values.sort_by_key(move |item| {
            if let Some(value) = item.as_value() {
                Some(f(value))
            } else {
                None
            }
        });
    }
}

#[cfg(feature = "display")]
impl std::fmt::Display for Array {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::encode::encode_array(self, f, None, ("", ""))
    }
}

impl<V: Into<Value>> Extend<V> for Array {
    fn extend<T: IntoIterator<Item = V>>(&mut self, iter: T) {
        for value in iter {
            self.push_formatted(value.into());
        }
    }
}

impl<V: Into<Value>> FromIterator<V> for Array {
    fn from_iter<I>(iter: I) -> Self
    where
        I: IntoIterator<Item = V>,
    {
        let v = iter.into_iter().map(|a| Item::Value(a.into()));
        Self {
            values: v.collect(),
            ..Default::default()
        }
    }
}

impl IntoIterator for Array {
    type Item = Value;
    type IntoIter = ArrayIntoIter;

    fn into_iter(self) -> Self::IntoIter {
        Box::new(
            self.values
                .into_iter()
                .filter(|v| v.is_value())
                .map(|v| v.into_value().unwrap()),
        )
    }
}

impl<'s> IntoIterator for &'s Array {
    type Item = &'s Value;
    type IntoIter = ArrayIter<'s>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

fn decorate_array(array: &mut Array) {
    for (i, value) in array
        .values
        .iter_mut()
        .filter_map(Item::as_value_mut)
        .enumerate()
    {
        // [value1, value2, value3]
        if i == 0 {
            value.decorate(DEFAULT_LEADING_VALUE_DECOR.0, DEFAULT_LEADING_VALUE_DECOR.1);
        } else {
            value.decorate(DEFAULT_VALUE_DECOR.0, DEFAULT_VALUE_DECOR.1);
        }
    }
    // Since everything is now on the same line, remove trailing commas and whitespace.
    array.set_trailing_comma(false);
    array.set_trailing("");
}

/// The decor immediately after a removed array value: the next value's prefix, or the
/// array's trailing when the last value was removed
enum FollowingDecor<'a> {
    Prefix(&'a mut Decor),
    Trailing(&'a mut RawString),
}

impl Array {
    fn following_decor_mut(&mut self, index: usize) -> FollowingDecor<'_> {
        match self.values.get_mut(index).and_then(Item::as_value_mut) {
            Some(next) => FollowingDecor::Prefix(next.decor_mut()),
            None => FollowingDecor::Trailing(&mut self.trailing),
        }
    }
}

/// Extract the comments to keep from a removed value's prefix.
///
/// The result, when non-empty, is newline-terminated so it can be placed directly in front
/// of the next value's prefix or the array's trailing.
fn extract_preservable_comments(prefix: &str) -> String {
    let newline = if prefix.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut comments = String::new();
    let mut rest = prefix;
    // The text up to the first newline shares a line with the previous value (or the
    // opening bracket): a comment there belongs to that line and stays in place.
    if let Some(end) = rest.find('\n') {
        let head = &rest[..end];
        if head.contains('#') {
            comments.push_str(head);
            comments.push('\n');
        }
        rest = &rest[end + 1..];
    }
    // Each later newline-terminated line that holds a comment is a standalone comment for
    // the removed value and moves with it.  The final, unterminated fragment is the
    // whitespace directly in front of the value and is dropped.
    while let Some(end) = rest.find('\n') {
        let line = &rest[..end];
        rest = &rest[end + 1..];
        if line.contains('#') {
            if comments.is_empty() {
                comments.push_str(newline);
            }
            comments.push_str(line);
            comments.push('\n');
        }
    }
    comments
}

/// Remove an inline comment (the text up to the first newline, when it holds a comment)
/// from the decor following a removed value, as that comment shared the removed value's
/// line.
fn strip_inline_comment_head(following: FollowingDecor<'_>) {
    let stripped = {
        let raw = match &following {
            FollowingDecor::Prefix(decor) => decor.prefix(),
            FollowingDecor::Trailing(trailing) => Some(&**trailing),
        };
        let Some(Some(text)) = raw.map(RawString::as_str) else {
            return;
        };
        let Some(newline) = text.find('\n') else {
            return;
        };
        let head = &text[..newline];
        if !head.contains('#') {
            return;
        }
        text[newline..].to_string()
    };
    match following {
        FollowingDecor::Prefix(decor) => decor.set_prefix(stripped),
        FollowingDecor::Trailing(trailing) => *trailing = RawString::from(stripped),
    }
}

/// Avoid a blank line when newline-terminated comments are placed in front of text that
/// already starts with a newline.
fn trim_redundant_newline(comments: &mut String, following: &str) {
    if !comments.ends_with('\n') {
        return;
    }
    if following.starts_with('\n') {
        comments.pop();
    } else if following.starts_with("\r\n") {
        comments.pop();
        if comments.ends_with('\r') {
            comments.pop();
        }
    }
}
