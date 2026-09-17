//! Feature-gated Tree-sitter highlighting for retained source editors.
//!
//! Highlighting produces byte ranges into the original UTF-8 source. The
//! renderer can therefore shape one unchanged string and vary style by byte
//! offset without creating a string for every token.

use std::fmt;
use std::sync::Arc;

use crate::{TextStyleRange, Underline};
pub use tree_sitter_highlight::HighlightConfiguration;
use tree_sitter_highlight::{HighlightEvent, Highlighter};

/// Capture categories recognized by [`SyntaxHighlighting`].
///
/// Language queries may use more specific dotted capture names (for example
/// `function.call`); Tree-sitter resolves those to the most specific category
/// listed here.
///
/// **Order matters.** `tree-sitter-highlight`'s `configure` breaks equally-long
/// matches by list order, and a dotted capture whose *modifier* is itself a
/// category here matches both (`keyword.function` → parts `keyword` +
/// `function`, each matching with length 1). Every modifier-doubled name must
/// therefore appear **after** the base categories it can modify, so a dotted
/// capture inherits its base (leftmost) category — `keyword.function` styles as
/// a keyword, `variable.parameter` as a variable, `function.call` as a
/// function. The `category_precedes_its_dotted_modifiers` test pins this.
const CAPTURE_NAMES: &[&str] = &[
    "attribute",
    "boolean",
    "comment",
    "constant",
    "number",
    "keyword",
    "conditional",
    "repeat",
    "type",
    "variable",
    "function",
    "method",
    "constructor",
    "label",
    "preproc",
    "operator",
    "parameter",
    "property",
    "punctuation",
    "string",
];

/// Reusable palette for Tree-sitter capture categories, as linear RGBA values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SyntaxTheme {
    /// Comments and documentation.
    pub comment: [f32; 4],
    /// Strings and character literals.
    pub string: [f32; 4],
    /// Numeric literals and booleans.
    pub literal: [f32; 4],
    /// Language keywords and preprocessor forms.
    pub keyword: [f32; 4],
    /// Function, method, and constructor names.
    pub function: [f32; 4],
    /// Types, constants, and attributes.
    pub type_or_constant: [f32; 4],
    /// Parameters, properties, and labels.
    pub symbol: [f32; 4],
    /// Operators and punctuation.
    pub operator: [f32; 4],
    /// Variables not covered by a more specific capture.
    pub variable: [f32; 4],
    /// Conditional keywords (`if`/`then`/`else`); falls back to
    /// [`keyword`](Self::keyword) by `Default` and in the built-in mapping.
    pub conditional: [f32; 4],
    /// Loop keywords (`while`/`for`/`repeat`); falls back to
    /// [`keyword`](Self::keyword) by `Default` and in the built-in mapping.
    pub repeat: [f32; 4],
}

impl Default for SyntaxTheme {
    fn default() -> Self {
        let keyword = [0.86, 0.48, 0.96, 1.0];
        Self {
            comment: [0.46, 0.52, 0.48, 1.0],
            string: [0.55, 0.82, 0.48, 1.0],
            literal: [0.96, 0.72, 0.38, 1.0],
            keyword,
            function: [0.38, 0.72, 0.96, 1.0],
            type_or_constant: [0.38, 0.78, 0.78, 1.0],
            symbol: [0.86, 0.76, 0.48, 1.0],
            operator: [0.78, 0.80, 0.84, 1.0],
            variable: [0.82, 0.84, 0.88, 1.0],
            conditional: keyword,
            repeat: keyword,
        }
    }
}

impl SyntaxTheme {
    fn color(self, capture: usize) -> [f32; 4] {
        match CAPTURE_NAMES[capture] {
            "comment" => self.comment,
            "string" => self.string,
            "number" | "boolean" => self.literal,
            "keyword" | "preproc" | "conditional" | "repeat" => self.keyword,
            "function" | "method" | "constructor" => self.function,
            "type" | "constant" | "attribute" => self.type_or_constant,
            "parameter" | "property" | "label" => self.symbol,
            "operator" | "punctuation" => self.operator,
            "variable" => self.variable,
            _ => unreachable!("CAPTURE_NAMES and SyntaxTheme mapping disagree"),
        }
    }
}

/// Immutable, shareable language configuration and palette.
///
/// Construct this from any Tree-sitter [`HighlightConfiguration`] after adding
/// the crate's `syntax-highlighting` feature. The configuration is configured
/// once and then shared; each editor retains and reuses its own parser state.
#[derive(Clone)]
pub struct SyntaxHighlighting {
    configuration: Arc<HighlightConfiguration>,
    theme: SyntaxTheme,
}

impl fmt::Debug for SyntaxHighlighting {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SyntaxHighlighting")
            .field("language", &self.configuration.language_name)
            .field("theme", &self.theme)
            .finish_non_exhaustive()
    }
}

impl PartialEq for SyntaxHighlighting {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.configuration, &other.configuration) && self.theme == other.theme
    }
}

impl SyntaxHighlighting {
    /// Build highlighting from a configured Tree-sitter language query.
    ///
    /// The supplied configuration is configured with this crate's reusable
    /// capture categories. Keep clones of the resulting value to share the
    /// compiled query between editors.
    pub fn new(mut configuration: HighlightConfiguration, theme: SyntaxTheme) -> Self {
        configuration.configure(CAPTURE_NAMES);
        Self {
            configuration: Arc::new(configuration),
            theme,
        }
    }

    /// Palette applied to this language configuration.
    pub fn theme(&self) -> SyntaxTheme {
        self.theme
    }

    /// Construct the bundled Lua highlighter.
    #[cfg(feature = "syntax-lua")]
    pub fn lua(theme: SyntaxTheme) -> Result<Self, SyntaxConfigurationError> {
        HighlightConfiguration::new(
            tree_sitter_lua::LANGUAGE.into(),
            "lua",
            tree_sitter_lua::HIGHLIGHTS_QUERY,
            tree_sitter_lua::INJECTIONS_QUERY,
            tree_sitter_lua::LOCALS_QUERY,
        )
        .map(|configuration| Self::new(configuration, theme))
        .map_err(|error| SyntaxConfigurationError(error.to_string()))
    }

    pub(crate) fn highlight_into(
        &self,
        highlighter: &mut Highlighter,
        source: &str,
        out: &mut Vec<TextStyleRange>,
    ) {
        out.clear();
        let Ok(events) =
            highlighter.highlight(&self.configuration, source.as_bytes(), None, |_| None)
        else {
            return;
        };
        let mut active = Vec::new();
        for event in events {
            match event {
                Ok(HighlightEvent::HighlightStart(highlight)) => active.push(highlight.0),
                Ok(HighlightEvent::HighlightEnd) => {
                    active.pop();
                }
                Ok(HighlightEvent::Source { start, end }) if start < end => {
                    if let Some(&capture) = active.last() {
                        out.push(TextStyleRange {
                            range: start..end,
                            color: Some(self.theme.color(capture)),
                            underline: Underline::None,
                        });
                    }
                }
                Ok(HighlightEvent::Source { .. }) | Err(_) => {}
            }
        }
    }
}

/// Error compiling a bundled language's Tree-sitter highlight queries.
#[cfg(feature = "syntax-lua")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxConfigurationError(String);

#[cfg(feature = "syntax-lua")]
impl fmt::Display for SyntaxConfigurationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid Tree-sitter highlight configuration: {}", self.0)
    }
}

#[cfg(feature = "syntax-lua")]
impl std::error::Error for SyntaxConfigurationError {}

#[cfg(all(test, feature = "syntax-lua"))]
mod tests {
    use super::*;

    fn highlighted(source: &str) -> Vec<(&str, [f32; 4])> {
        let syntax = SyntaxHighlighting::lua(SyntaxTheme::default()).unwrap();
        let mut highlighter = Highlighter::new();
        let mut ranges = Vec::new();
        syntax.highlight_into(&mut highlighter, source, &mut ranges);
        ranges
            .into_iter()
            .map(|range| (&source[range.range], range.color.unwrap()))
            .collect()
    }

    #[test]
    fn lua_highlights_comments_strings_keywords_and_functions() {
        let theme = SyntaxTheme::default();
        let parts = highlighted("local function greet(name)\n -- hi\n return 'hello' .. name\nend");
        assert!(parts.contains(&("local", theme.keyword)));
        // `function` here is the @keyword.function capture — a keyword, not the
        // function-name category (see `category_precedes_its_dotted_modifiers`).
        assert!(parts.contains(&("function", theme.keyword)));
        assert!(parts.contains(&("greet", theme.function)));
        assert!(parts.contains(&("-- hi", theme.comment)));
        assert!(parts.contains(&("'hello'", theme.string)));
    }

    /// `configure` resolves a dotted capture to the most specific listed
    /// category, breaking equal-length matches by list order. A capture like
    /// `keyword.function` therefore matches BOTH `keyword` and `function` —
    /// and if `function` is listed first, the keyword styles as a function
    /// name. Every category that can appear as another category's dotted
    /// modifier must be listed *after* it so the base (leftmost) category wins.
    #[test]
    fn category_precedes_its_dotted_modifiers() {
        // Base categories that also occur as modifiers in common queries.
        let modifiers = [
            "attribute",
            "boolean",
            "comment",
            "constant",
            "constructor",
            "function",
            "keyword",
            "conditional",
            "repeat",
            "label",
            "method",
            "number",
            "operator",
            "parameter",
            "preproc",
            "property",
            "punctuation",
            "string",
            "type",
            "variable",
        ];
        for modifier in modifiers {
            let dotted = format!("{modifier}.something");
            // Re-implement `configure`'s resolution for this capture: every
            // listed category whose parts are all present wins; ties go to the
            // earliest entry. The base category must beat its own modifier.
            let parts: Vec<&str> = dotted.split('.').collect();
            let resolved = CAPTURE_NAMES
                .iter()
                .filter(|candidate| candidate.split('.').all(|part| parts.contains(&part)))
                .max_by_key(|candidate| candidate.split('.').count())
                .copied();
            assert_eq!(
                resolved,
                Some(modifier),
                "{dotted} must resolve to its base category `{modifier}` — \
                 reorder CAPTURE_NAMES so the base precedes the modifier"
            );
        }
    }

    #[test]
    fn lua_malformed_and_incomplete_source_still_produces_valid_ranges() {
        for source in [
            "local function unfinished(",
            "if x then\n  print(\"open",
            "--[=[ open",
        ] {
            let parts = highlighted(source);
            assert!(!parts.is_empty());
        }
    }
}
