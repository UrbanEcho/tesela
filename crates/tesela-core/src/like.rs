//! Precompiled SQL `LIKE` patterns.
//!
//! `LIKE` is a *semantic* operator — the spec declares it, the IR carries it —
//! so its matching rules live here rather than in whichever backend happens to
//! evaluate a query. A backend that pushes `LIKE` down to SQL gets SQL's
//! semantics; one that evaluates it in Tesela gets these; and the two agree
//! because there is only one implementation to disagree with.

use std::fmt;
use std::sync::Arc;

/// Why a `LIKE` pattern could not be compiled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LikeError {
    /// The pattern ends with an escape character that escapes nothing.
    TrailingEscape,
}

impl fmt::Display for LikeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TrailingEscape => "LIKE pattern ends with a dangling escape character",
        })
    }
}

impl std::error::Error for LikeError {}

/// One token of a compiled `LIKE` pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    /// A run of literal characters.
    Literal(Box<str>),
    /// `_` — exactly one character.
    AnyOne,
    /// `%` — any run of characters, including none.
    AnyRun,
}

/// The shape of a compiled pattern, when it is cheap enough to answer outright.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Shape {
    /// No wildcards at all: an equality check.
    Exact(Box<str>),
    /// A single trailing `%`: a prefix check that `str::starts_with` answers
    /// without the general matcher.
    Prefix(Box<str>),
    /// A leading and trailing `%`: a substring check.
    Contains(Box<str>),
    /// Anything else: the general wildcard matcher.
    Pattern,
}

/// A compiled `LIKE` pattern.
///
/// Compiling once per query instead of re-splitting the pattern per record is
/// most of the win, but the correctness win matters more. The previous
/// implementation split the pattern on `%` and searched for each piece in
/// turn, which:
///
/// * was **unanchored**, so `abc%` matched `xabc` (the pieces are searched for
///   anywhere, never pinned to the start);
/// * ignored `_` entirely, so `a_c` matched `abc` and `a12345c` alike; and
/// * had no escape syntax, so a literal `%` could not be expressed at all.
///
/// This matcher is anchored at both ends, matching SQL semantics, honours `_`
/// as exactly one character, and treats `\` as an escape for `%`, `_` and `\`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LikePlan {
    pattern: Arc<str>,
    segments: Vec<Segment>,
    shape: Shape,
}

impl LikePlan {
    /// Compile a `LIKE` pattern.
    pub fn new(pattern: &str) -> Result<Self, LikeError> {
        let mut segments: Vec<Segment> = Vec::new();
        let mut literal = String::new();
        let mut characters = pattern.chars();

        while let Some(character) = characters.next() {
            match character {
                '\\' => match characters.next() {
                    Some(escaped) => literal.push(escaped),
                    None => return Err(LikeError::TrailingEscape),
                },
                '%' => {
                    push_literal(&mut segments, &mut literal);
                    segments.push(Segment::AnyRun);
                }
                '_' => {
                    push_literal(&mut segments, &mut literal);
                    segments.push(Segment::AnyOne);
                }
                other => literal.push(other),
            }
        }
        push_literal(&mut segments, &mut literal);

        Ok(Self {
            pattern: Arc::from(pattern),
            shape: shape_of(&segments),
            segments,
        })
    }

    /// Whether the text matches.
    ///
    /// The pattern is anchored at both ends: `abc%` matches `abcdef` but not
    /// `xabcdef`.
    #[must_use]
    pub fn matches(&self, text: &str) -> bool {
        match &self.shape {
            Shape::Exact(literal) => text.as_bytes() == literal.as_bytes(),
            Shape::Prefix(prefix) => text.starts_with(prefix.as_ref()),
            Shape::Contains(needle) => text.contains(needle.as_ref()),
            Shape::Pattern => wildcard_match(&self.segments, text),
        }
    }

    /// The original pattern text.
    #[must_use]
    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    /// Whether the pattern contains any wildcard.
    #[must_use]
    pub fn has_wildcards(&self) -> bool {
        !matches!(self.shape, Shape::Exact(_))
    }
}

impl fmt::Display for LikePlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.pattern)
    }
}

fn push_literal(segments: &mut Vec<Segment>, literal: &mut String) {
    if !literal.is_empty() {
        segments.push(Segment::Literal(literal.as_str().into()));
        literal.clear();
    }
}

/// Classify a token list into one of the fast shapes.
///
/// The classification is by *shape*, not by wildcard count: `%abc%` carries two
/// `%` and is still a substring test. `literal%` and `%literal%` are common
/// enough to be worth their own path, and `str::starts_with` / `str::contains`
/// are vectorised where the general matcher is not.
fn shape_of(segments: &[Segment]) -> Shape {
    match segments {
        [] => Shape::Exact(Box::from("")),
        [Segment::Literal(literal)] => Shape::Exact(literal.clone()),
        [Segment::Literal(prefix), Segment::AnyRun] => Shape::Prefix(prefix.clone()),
        [Segment::AnyRun, Segment::Literal(needle), Segment::AnyRun] => {
            Shape::Contains(needle.clone())
        }
        _ => Shape::Pattern,
    }
}

/// Match a token list against text, anchored at both ends.
///
/// A greedy matcher that remembers the last `%` it passed: on a mismatch it
/// lets that `%` swallow one more character and retries from there. Every byte
/// is visited at most twice, so matching is linear in the text length even for
/// adversarial patterns — a recursive matcher would be exponential.
fn wildcard_match(segments: &[Segment], text: &str) -> bool {
    let mut segment_index = 0usize;
    let mut text_index = 0usize;
    let mut last_any_run: Option<usize> = None;
    let mut last_any_run_text = 0usize;

    loop {
        if let Some(segment) = segments.get(segment_index) {
            match segment {
                Segment::Literal(literal) => {
                    if text[text_index..].starts_with(literal.as_ref()) {
                        text_index += literal.len();
                        segment_index += 1;
                        continue;
                    }
                }
                Segment::AnyOne => {
                    if let Some(character) = text[text_index..].chars().next() {
                        text_index += character.len_utf8();
                        segment_index += 1;
                        continue;
                    }
                }
                Segment::AnyRun => {
                    last_any_run = Some(segment_index);
                    last_any_run_text = text_index;
                    segment_index += 1;
                    continue;
                }
            }
        } else if text_index == text.len() {
            return true;
        }

        // Mismatch, or pattern exhausted with text left over: widen the last `%`.
        match last_any_run {
            Some(any_run) => {
                let Some(character) = text[last_any_run_text..].chars().next() else {
                    return false;
                };
                last_any_run_text += character.len_utf8();
                text_index = last_any_run_text;
                segment_index = any_run + 1;
            }
            None => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches(pattern: &str, text: &str) -> bool {
        LikePlan::new(pattern)
            .expect("pattern should compile")
            .matches(text)
    }

    #[test]
    fn a_pattern_without_wildcards_is_an_exact_match() {
        assert!(matches("abc", "abc"));
        assert!(!matches("abc", "abcd"));
        assert!(!matches("abc", "xabc"));
    }

    #[test]
    fn a_trailing_wildcard_anchors_at_the_start() {
        // The regression this replaces: the old matcher split on `%` and
        // searched for the pieces anywhere, so `abc%` matched `xabc`.
        assert!(matches("abc%", "abcdef"));
        assert!(matches("abc%", "abc"));
        assert!(!matches("abc%", "xabc"));
        assert!(!matches("abc%", "xabcdef"));
    }

    #[test]
    fn a_leading_wildcard_anchors_at_the_end() {
        assert!(matches("%abc", "xyzabc"));
        assert!(!matches("%abc", "abcxyz"));
    }

    #[test]
    fn wildcards_on_both_sides_match_a_substring() {
        assert!(matches("%bc%", "abcd"));
        assert!(matches("%bc%", "zbc"));
        assert!(!matches("%bc%", "abdc"));
    }

    #[test]
    fn underscore_matches_exactly_one_character() {
        assert!(matches("a_c", "abc"));
        assert!(matches("a_c", "aéc"));
        assert!(!matches("a_c", "ac"));
        assert!(!matches("a_c", "abbc"));
    }

    #[test]
    fn underscore_counts_characters_not_bytes() {
        assert!(matches("é_", "éa"));
        assert!(!matches("é_", "é"));
    }

    #[test]
    fn a_lone_wildcard_matches_anything() {
        assert!(matches("%", ""));
        assert!(matches("%", "anything at all"));
    }

    #[test]
    fn escapes_make_wildcards_literal() {
        assert!(matches(r"100\%", "100%"));
        assert!(!matches(r"100\%", "100x"));
        assert!(matches(r"a\_c", "a_c"));
        assert!(!matches(r"a\_c", "abc"));
        assert!(matches(r"a\\b", r"a\b"));
    }

    #[test]
    fn a_dangling_escape_is_rejected() {
        assert_eq!(LikePlan::new("abc\\"), Err(LikeError::TrailingEscape));
    }

    #[test]
    fn repeated_wildcards_still_terminate() {
        // Exponential blow-up is the failure mode of a naive backtracking
        // matcher; this must finish instantly.
        let text = "a".repeat(64);
        // No `b` anywhere, so the mandatory trailing literal can never be met.
        assert!(!matches("%a%a%a%a%a%a%a%a%b%", &text));
        assert!(matches("%a%a%a%a%a%a%a%a%b%", &format!("{text}b")));
    }

    #[test]
    fn shapes_pick_the_cheap_path() {
        assert_eq!(
            LikePlan::new("abc").expect("compiles").shape,
            Shape::Exact("abc".into())
        );
        assert_eq!(
            LikePlan::new("abc%").expect("compiles").shape,
            Shape::Prefix("abc".into())
        );
        assert_eq!(
            LikePlan::new("%abc%").expect("compiles").shape,
            Shape::Contains("abc".into())
        );
        assert_eq!(
            LikePlan::new("%abc").expect("compiles").shape,
            Shape::Pattern
        );
        assert!(!LikePlan::new("abc").expect("compiles").has_wildcards());
        assert!(LikePlan::new("ab_").expect("compiles").has_wildcards());
    }

    #[test]
    fn an_empty_pattern_matches_only_the_empty_string() {
        assert!(matches("", ""));
        assert!(!matches("", "a"));
    }

    #[test]
    fn the_original_pattern_is_preserved_for_diagnostics() {
        let plan = LikePlan::new(r"a\%b").expect("compiles");
        assert_eq!(plan.pattern(), r"a\%b");
        assert_eq!(plan.to_string(), r"a\%b");
    }
}
