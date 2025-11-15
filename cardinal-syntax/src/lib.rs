//! # Cardinal's Everything-like Syntax Parser
//!
//! `cardinal-syntax` turns raw Everything-style queries into a structured AST so
//! the rest of Cardinal can reason about filters, boolean logic, and phrases
//! without duplicating the parsing rules from the original Windows tool. Any
//! example shown in that manual should be accepted by [`parse_query`].
//!
//! ## Example
//! ```
//! use cardinal_syntax::{parse_query, Expr, Term};
//!
//! let query = parse_query("folder: dm:pastmonth ext:docx report").unwrap();
//! if let Expr::And(parts) = query.expr {
//!     // `folder:` (no argument), `dm:pastmonth`, `ext:docx`, and the bare word.
//!     assert_eq!(parts.len(), 4);
//!     assert!(matches!(parts[3], Expr::Term(Term::Word(ref word)) if word == "report"));
//! }
//! ```

use std::fmt;

/// Parses an Everything-like query string into a structured expression tree.
pub fn parse_query(input: &str) -> Result<Query, ParseError> {
    if let Some(result) = try_parse_regex_query(input) {
        return result;
    }

    Parser::new(input).parse()
}

fn try_parse_regex_query(input: &str) -> Option<Result<Query, ParseError>> {
    let trimmed = input.trim_start();
    const PREFIX: &str = "regex:";
    if trimmed.len() < PREFIX.len() {
        return None;
    }
    if !trimmed[..PREFIX.len()].eq_ignore_ascii_case(PREFIX) {
        return None;
    }

    let offset = input.len() - trimmed.len();
    let pattern_start = offset + PREFIX.len();
    let pattern = trimmed[PREFIX.len()..].trim();
    if pattern.is_empty() {
        return Some(Err(ParseError {
            message: "regex: requires a pattern".into(),
            position: pattern_start,
        }));
    }

    Some(Ok(Query {
        expr: Expr::Term(Term::Regex(pattern.to_string())),
    }))
}

/// User input normalized into a single expression tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    pub expr: Expr,
}

impl Query {
    pub fn is_empty(&self) -> bool {
        matches!(self.expr, Expr::Empty)
    }
}

/// Logical structure for Everything queries.
///
/// The `And`/`Or` variants store flat vectors instead of binary trees so
/// consumers can iterate terms without rebalancing. `Empty` exists so parsing
/// helpers can return a sentinel when a group contains whitespace or is
/// mid-construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    /// Returned when a query (or sub query) only contains whitespace.
    ///
    /// ```
    /// use cardinal_syntax::{parse_query, Expr};
    /// assert!(matches!(parse_query("   ").unwrap().expr, Expr::Empty));
    /// ```
    Empty,
    /// Wraps a [`Term`] so it can participate in boolean expressions.
    ///
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term};
    /// let expr = parse_query("report").unwrap().expr;
    /// assert!(matches!(expr, Expr::Term(Term::Word(word)) if word == "report"));
    /// ```
    Term(Term),
    /// Logical negation. Multiple NOT prefixes collapse into a single node.
    ///
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term};
    /// let expr = parse_query("!temp").unwrap().expr;
    /// if let Expr::Not(inner) = expr {
    ///     assert!(matches!(&*inner, Expr::Term(Term::Word(word)) if word == "temp"));
    /// }
    /// ```
    Not(Box<Expr>),
    /// Conjunction (implicit via whitespace in Everything syntax).
    ///
    /// ```
    /// use cardinal_syntax::{parse_query, Expr};
    /// let Expr::And(parts) = parse_query("foo bar").unwrap().expr else { panic!() };
    /// assert_eq!(parts.len(), 2);
    /// ```
    And(Vec<Expr>),
    /// Disjunction triggered by the `|` operator.
    ///
    /// ```
    /// use cardinal_syntax::{parse_query, Expr};
    /// let Expr::Or(parts) = parse_query("foo|bar").unwrap().expr else { panic!() };
    /// assert_eq!(parts.len(), 2);
    /// ```
    Or(Vec<Expr>),
}

/// A leaf expression that Everything understands without further boolean
/// structure. Filters are kept separate from raw words so higher layers can
/// translate them into structured lookups or validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Term {
    /// A bare word or wildcard token (e.g., `report`, `*.mp3`).
    ///
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term};
    /// let Expr::Term(Term::Word(word)) = parse_query("*.mp3").unwrap().expr else { panic!() };
    /// assert_eq!(word, "*.mp3");
    /// ```
    Word(String),
    /// Quoted phrase such as `"summer holiday"`.
    ///
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term};
    /// let Expr::Term(Term::Phrase(phrase)) = parse_query("\"summer holiday\"").unwrap().expr else { panic!() };
    /// assert_eq!(phrase, "summer holiday");
    /// ```
    Phrase(String),
    /// `name:argument` style filters (`size:>1GB`, `folder:` ...).
    ///
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, FilterKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("size:>1GB").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.kind, FilterKind::Size));
    /// ```
    Filter(Filter),
    /// Regex-prefixed query (`regex:...`). Everything treats the rest of the
    /// string as a regular expression.
    ///
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term};
    /// let Expr::Term(Term::Regex(pattern)) = parse_query("regex:^Report").unwrap().expr else { panic!() };
    /// assert_eq!(pattern, "^Report");
    /// ```
    Regex(String),
}

/// `name:argument` style filters Everything exposes (e.g. `size:>1gb`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filter {
    pub kind: FilterKind,
    pub argument: Option<FilterArgument>,
}

/// Strongly-typed view over Everything's built-in filters. Custom macros fall
/// back to [`FilterKind::Custom`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterKind {
    /// Only match files (`file:`).
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, FilterKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("file:report").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.kind, FilterKind::File));
    /// ```
    File,
    /// Only match folders (`folder:`).
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, FilterKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("folder:Projects").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.kind, FilterKind::Folder));
    /// ```
    Folder,
    /// Extension filter (`ext:`).
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, FilterKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("ext:txt").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.kind, FilterKind::Ext));
    /// ```
    Ext,
    /// File type categories (`type:` such as `type:picture`).
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, FilterKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("type:picture").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.kind, FilterKind::Type));
    /// ```
    Type,
    /// Audio macro (`audio:`).
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, FilterKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("audio:").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.kind, FilterKind::Audio));
    /// ```
    Audio,
    /// Video macro (`video:`).
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, FilterKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("video:").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.kind, FilterKind::Video));
    /// ```
    Video,
    /// Document macro (`doc:`).
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, FilterKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("doc:").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.kind, FilterKind::Doc));
    /// ```
    Doc,
    /// Executable macro (`exe:`).
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, FilterKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("exe:").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.kind, FilterKind::Exe));
    /// ```
    Exe,
    /// Size comparisons or ranges (`size:`).
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, FilterKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("size:>1GB").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.kind, FilterKind::Size));
    /// ```
    Size,
    /// Date modified (`dm:` / `datemodified:`).
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, FilterKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("dm:today").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.kind, FilterKind::DateModified));
    /// ```
    DateModified,
    /// Date created (`dc:` / `datecreated:`).
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, FilterKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("dc:thisweek").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.kind, FilterKind::DateCreated));
    /// ```
    DateCreated,
    /// Date accessed (`da:` / `dateaccessed:`).
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, FilterKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("da:yesterday").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.kind, FilterKind::DateAccessed));
    /// ```
    DateAccessed,
    /// Date run (`dr:` / `daterun:`).
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, FilterKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("dr:today").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.kind, FilterKind::DateRun));
    /// ```
    DateRun,
    /// Restrict to direct children of a folder (`parent:`).
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, FilterKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("parent:/Users").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.kind, FilterKind::Parent));
    /// ```
    Parent,
    /// Restrict to descendants of a folder (`infolder:`).
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, FilterKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("infolder:/Users/demo").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.kind, FilterKind::InFolder));
    /// ```
    InFolder,
    /// Limit to the folder itself (`nosubfolders:`).
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, FilterKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("nosubfolders:/Users/demo").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.kind, FilterKind::NoSubfolders));
    /// ```
    NoSubfolders,
    /// Require a folder containing matching children (`child:`).
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, FilterKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("child:*.mp3").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.kind, FilterKind::Child));
    /// ```
    Child,
    /// Match file-system attributes (`attrib:`).
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, FilterKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("attrib:H").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.kind, FilterKind::Attribute));
    /// ```
    Attribute,
    /// Attribute duplicate detection (`attribdupe:`).
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, FilterKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("attribdupe:").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.kind, FilterKind::AttributeDuplicate));
    /// ```
    AttributeDuplicate,
    /// Date-modified duplicate detection (`dmdupe:`).
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, FilterKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("dmdupe:").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.kind, FilterKind::DateModifiedDuplicate));
    /// ```
    DateModifiedDuplicate,
    /// Name duplicate detection (`dupe:`).
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, FilterKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("dupe:").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.kind, FilterKind::Duplicate));
    /// ```
    Duplicate,
    /// Duplicate detection ignoring extensions (`namepartdupe:`).
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, FilterKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("namepartdupe:").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.kind, FilterKind::NamePartDuplicate));
    /// ```
    NamePartDuplicate,
    /// Duplicate detection by size (`sizedupe:`).
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, FilterKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("sizedupe:").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.kind, FilterKind::SizeDuplicate));
    /// ```
    SizeDuplicate,
    /// Audio metadata—artist (`artist:`).
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, FilterKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("artist:Daft").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.kind, FilterKind::Artist));
    /// ```
    Artist,
    /// Audio metadata—album (`album:`).
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, FilterKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("album:Discovery").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.kind, FilterKind::Album));
    /// ```
    Album,
    /// Audio metadata—title (`title:`).
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, FilterKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("title:OneMoreTime").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.kind, FilterKind::Title));
    /// ```
    Title,
    /// Audio metadata—genre (`genre:`).
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, FilterKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("genre:house").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.kind, FilterKind::Genre));
    /// ```
    Genre,
    /// Audio metadata—year (`year:`).
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, FilterKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("year:2024").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.kind, FilterKind::Year));
    /// ```
    Year,
    /// Audio metadata—track number (`track:`).
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, FilterKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("track:01").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.kind, FilterKind::Track));
    /// ```
    Track,
    /// Audio metadata—comment (`comment:`).
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, FilterKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("comment:live").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.kind, FilterKind::Comment));
    /// ```
    Comment,
    /// Image width comparisons (`width:`).
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, FilterKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("width:>4000").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.kind, FilterKind::Width));
    /// ```
    Width,
    /// Image height comparisons (`height:`).
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, FilterKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("height:<=2000").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.kind, FilterKind::Height));
    /// ```
    Height,
    /// Combined dimensions (`dimensions:`).
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, FilterKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("dimensions:1920x1080").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.kind, FilterKind::Dimensions));
    /// ```
    Dimensions,
    /// Orientation filter (`orientation:`).
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, FilterKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("orientation:horizontal").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.kind, FilterKind::Orientation));
    /// ```
    Orientation,
    /// Bit depth filter (`bitdepth:`).
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, FilterKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("bitdepth:24").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.kind, FilterKind::BitDepth));
    /// ```
    BitDepth,
    /// Case-sensitive search toggle (`case:`).
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, FilterKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("case:ABC").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.kind, FilterKind::CaseSensitive));
    /// ```
    CaseSensitive,
    /// Content search (`content:`).
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, FilterKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("content:error").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.kind, FilterKind::Content));
    /// ```
    Content,
    /// Temporarily disable whole filename matching (`nowholefilename:`).
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, FilterKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("nowholefilename:report").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.kind, FilterKind::NoWholeFilename));
    /// ```
    NoWholeFilename,
    /// User-defined macro or unrecognized filter name.
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, FilterKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("proj:").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.kind, FilterKind::Custom(name) if name == "proj"));
    /// ```
    Custom(String),
}

impl FilterKind {
    fn from_name(name: &str) -> Self {
        let lower = name.to_ascii_lowercase();
        match lower.as_str() {
            "file" => FilterKind::File,
            "folder" => FilterKind::Folder,
            "ext" => FilterKind::Ext,
            "type" => FilterKind::Type,
            "audio" => FilterKind::Audio,
            "video" => FilterKind::Video,
            "doc" => FilterKind::Doc,
            "exe" => FilterKind::Exe,
            "size" => FilterKind::Size,
            "dm" | "datemodified" => FilterKind::DateModified,
            "dc" | "datecreated" => FilterKind::DateCreated,
            "da" | "dateaccessed" => FilterKind::DateAccessed,
            "dr" | "daterun" => FilterKind::DateRun,
            "parent" => FilterKind::Parent,
            "infolder" => FilterKind::InFolder,
            "nosubfolders" => FilterKind::NoSubfolders,
            "child" => FilterKind::Child,
            "attrib" => FilterKind::Attribute,
            "attribdupe" => FilterKind::AttributeDuplicate,
            "dmdupe" => FilterKind::DateModifiedDuplicate,
            "dupe" => FilterKind::Duplicate,
            "namepartdupe" => FilterKind::NamePartDuplicate,
            "sizedupe" => FilterKind::SizeDuplicate,
            "artist" => FilterKind::Artist,
            "album" => FilterKind::Album,
            "title" => FilterKind::Title,
            "genre" => FilterKind::Genre,
            "year" => FilterKind::Year,
            "track" => FilterKind::Track,
            "comment" => FilterKind::Comment,
            "width" => FilterKind::Width,
            "height" => FilterKind::Height,
            "dimensions" => FilterKind::Dimensions,
            "orientation" => FilterKind::Orientation,
            "bitdepth" => FilterKind::BitDepth,
            "case" => FilterKind::CaseSensitive,
            "content" => FilterKind::Content,
            "nowholefilename" => FilterKind::NoWholeFilename,
            _ => FilterKind::Custom(name.to_string()),
        }
    }
}

/// Captures both the raw string and the heuristically detected shape so a
/// consumer can distinguish between e.g. a list (`ext:jpg;png`) and a comparison
/// (`size:>1GB`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterArgument {
    pub raw: String,
    pub kind: ArgumentKind,
}

/// Common syntactic patterns supported by Everything filters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArgumentKind {
    /// Plain argument with no additional structure.
    ///
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, ArgumentKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("folder:Projects").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.argument.unwrap().kind, ArgumentKind::Bare));
    /// ```
    Bare,
    /// Double quoted value (Everything keeps the text verbatim).
    ///
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, ArgumentKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("parent:\"/Users/demo\"").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.argument.unwrap().kind, ArgumentKind::Phrase));
    /// ```
    Phrase,
    /// Semicolon-delimited list such as `ext:jpg;png`.
    ///
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, ArgumentKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("ext:jpg;png").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.argument.unwrap().kind, ArgumentKind::List(values) if values == ["jpg", "png"]));
    /// ```
    List(Vec<String>),
    /// Numeric/date range (dotted or hyphenated).
    ///
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, ArgumentKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("size:1mb..10mb").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.argument.unwrap().kind, ArgumentKind::Range(_)));
    /// ```
    Range(RangeValue),
    /// Comparison like `>1GB`, `<=4000`, etc.
    ///
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, ArgumentKind};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("size:>1GB").unwrap().expr else { panic!() };
    /// assert!(matches!(filter.argument.unwrap().kind, ArgumentKind::Comparison(_)));
    /// ```
    Comparison(ComparisonValue),
}

/// Represents `start..end` or `start-end` ranges. Empty endpoints are allowed
/// for open ranges (Everything treats `..10mb` as `<=10mb`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeValue {
    pub start: Option<String>,
    pub end: Option<String>,
    pub separator: RangeSeparator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RangeSeparator {
    /// Range expressed with `..` such as `size:1..10`.
    ///
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, ArgumentKind, RangeSeparator};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("size:1..10").unwrap().expr else { panic!() };
    /// let ArgumentKind::Range(range) = filter.argument.unwrap().kind else { panic!() };
    /// assert!(matches!(range.separator, RangeSeparator::Dots));
    /// ```
    Dots,
    /// Range expressed with `-` (dates such as `dc:2024/1/1-2024/12/31`).
    ///
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, ArgumentKind, RangeSeparator};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("dc:2024/01/01-2024/12/31").unwrap().expr else { panic!() };
    /// let ArgumentKind::Range(range) = filter.argument.unwrap().kind else { panic!() };
    /// assert!(matches!(range.separator, RangeSeparator::Hyphen));
    /// ```
    Hyphen,
}

/// `size:>1GB` style comparisons.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComparisonValue {
    pub op: ComparisonOp,
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparisonOp {
    /// `< value` comparison.
    ///
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, ArgumentKind, ComparisonOp};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("size:<10mb").unwrap().expr else { panic!() };
    /// let ArgumentKind::Comparison(value) = filter.argument.unwrap().kind else { panic!() };
    /// assert!(matches!(value.op, ComparisonOp::Lt));
    /// ```
    Lt,
    /// `<= value` comparison.
    ///
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, ArgumentKind, ComparisonOp};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("width:<=4000").unwrap().expr else { panic!() };
    /// let ArgumentKind::Comparison(value) = filter.argument.unwrap().kind else { panic!() };
    /// assert!(matches!(value.op, ComparisonOp::Lte));
    /// ```
    Lte,
    /// `> value` comparison.
    ///
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, ArgumentKind, ComparisonOp};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("size:>1GB").unwrap().expr else { panic!() };
    /// let ArgumentKind::Comparison(value) = filter.argument.unwrap().kind else { panic!() };
    /// assert!(matches!(value.op, ComparisonOp::Gt));
    /// ```
    Gt,
    /// `>= value` comparison.
    ///
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, ArgumentKind, ComparisonOp};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("size:>=1GB").unwrap().expr else { panic!() };
    /// let ArgumentKind::Comparison(value) = filter.argument.unwrap().kind else { panic!() };
    /// assert!(matches!(value.op, ComparisonOp::Gte));
    /// ```
    Gte,
    /// `= value` comparison.
    ///
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, ArgumentKind, ComparisonOp};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("size:=10mb").unwrap().expr else { panic!() };
    /// let ArgumentKind::Comparison(value) = filter.argument.unwrap().kind else { panic!() };
    /// assert!(matches!(value.op, ComparisonOp::Eq));
    /// ```
    Eq,
    /// `!= value` comparison.
    ///
    /// ```
    /// use cardinal_syntax::{parse_query, Expr, Term, ArgumentKind, ComparisonOp};
    /// let Expr::Term(Term::Filter(filter)) = parse_query("size:!=10mb").unwrap().expr else { panic!() };
    /// let ArgumentKind::Comparison(value) = filter.argument.unwrap().kind else { panic!() };
    /// assert!(matches!(value.op, ComparisonOp::Ne));
    /// ```
    Ne,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub message: String,
    pub position: usize,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (at byte {})", self.message, self.position)
    }
}

impl std::error::Error for ParseError {}

/// Hand-rolled recursive-descent parser because the language is tiny and we
/// need to model whitespace, booleans, and filters exactly the way Everything
/// does. Keeping the parser explicit makes it easy to line up code with the
/// textual manual.
struct Parser<'a> {
    input: &'a str,
    pos: usize,
    group_stack: Vec<char>,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input,
            pos: 0,
            group_stack: Vec::new(),
        }
    }

    fn parse(mut self) -> Result<Query, ParseError> {
        let expr = self.parse_and()?;
        self.skip_ws();
        if !self.eof() {
            return Err(self.error("unexpected trailing characters"));
        }
        Ok(Query { expr })
    }

    // AND has the lowest precedence and is implicit between whitespace-delimited
    // terms. We accumulate a Vec instead of nesting binary nodes so callers get
    // a normalized structure regardless of how many terms are chained.
    fn parse_and(&mut self) -> Result<Expr, ParseError> {
        let mut parts = Vec::new();
        let mut pending_keyword_and = false;
        loop {
            self.skip_ws();
            if self.consume_keyword("AND") {
                if parts.is_empty() {
                    parts.push(Expr::Empty);
                }
                pending_keyword_and = true;
                continue;
            }
            if self.eof() || self.is_at_group_close() {
                if pending_keyword_and {
                    parts.push(Expr::Empty);
                }
                break;
            }
            let expr = self.parse_or()?;
            if matches!(expr, Expr::Empty) {
                break;
            }
            parts.push(expr);
            if pending_keyword_and {
                pending_keyword_and = false;
            }
        }

        match parts.len() {
            0 => Ok(Expr::Empty),
            1 => Ok(parts.remove(0)),
            _ => Ok(Expr::And(parts)),
        }
    }

    // OR has higher precedence than AND but still associates left-to-right via
    // the vector accumulator, mirroring how Everything evaluates `|`.
    fn parse_or(&mut self) -> Result<Expr, ParseError> {
        let mut parts = Vec::new();
        loop {
            self.skip_ws();
            let operand_is_empty =
                self.peek_char() == Some('|') || self.eof() || self.is_at_group_close();
            if operand_is_empty {
                parts.push(Expr::Empty);
            } else {
                parts.push(self.parse_not()?);
            }

            self.skip_ws();
            let matched = if self.peek_char() == Some('|') {
                self.advance_char();
                true
            } else {
                self.consume_keyword("OR")
            };

            if !matched {
                break;
            }
        }

        if parts.len() == 1 {
            Ok(parts.remove(0))
        } else {
            Ok(Expr::Or(parts))
        }
    }

    // NOT binds tighter than OR/AND and Everything allows chains like
    // `!!!foo`, so we count prefixes and only wrap once if the parity is odd.
    fn parse_not(&mut self) -> Result<Expr, ParseError> {
        let mut negations = 0;
        loop {
            self.skip_ws();
            if self.peek_char() == Some('!') {
                self.advance_char();
                negations += 1;
                continue;
            }
            if self.consume_keyword("NOT") {
                negations += 1;
                continue;
            }
            break;
        }

        let mut expr = self.parse_primary()?;
        if negations % 2 == 1 {
            expr = Expr::Not(Box::new(expr));
        }
        Ok(expr)
    }

    // Primary expressions cover grouped subqueries, quoted phrases, regex, and
    // bare tokens/filters. Everything does not require escape sequences inside
    // quoted strings, so we treat backslashes literally.
    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        self.skip_ws();
        if self.eof() {
            return Ok(Expr::Empty);
        }

        match self.peek_char().unwrap() {
            '<' => self.parse_group('>'),
            '(' => self.parse_group(')'),
            '>' | ')' => Err(self.error("unexpected closing delimiter")),
            '"' => {
                let text = self.parse_phrase_string()?;
                Ok(Expr::Term(Term::Phrase(text)))
            }
            _ => {
                let term = self.parse_word_like()?;
                Ok(Expr::Term(term))
            }
        }
    }

    fn parse_group(&mut self, closing: char) -> Result<Expr, ParseError> {
        self.advance_char(); // consume opening token
        self.group_stack.push(closing);
        let expr = self.parse_and()?;
        self.group_stack.pop();
        self.skip_ws();
        if self.peek_char() == Some(closing) {
            self.advance_char();
            Ok(expr)
        } else {
            Err(self.error(format!("expected '{closing}'")))
        }
    }

    // Parses bare tokens with optional `name:` filters. Everything allows
    // filters to appear anywhere, so this routine needs to stop as soon as we
    // see `:` to avoid consuming the argument.
    fn parse_word_like(&mut self) -> Result<Term, ParseError> {
        let start = self.pos;
        let mut seen = false;
        while let Some(ch) = self.peek_char() {
            if ch == ':' && seen {
                let name = &self.input[start..self.pos];
                if is_valid_filter_name(name) {
                    self.advance_char();
                    return self.parse_filter_term(name.to_string());
                }
            }

            if is_term_breaker(ch) {
                break;
            }
            seen = true;
            self.advance_char();
        }

        if start == self.pos {
            return Err(self.error("expected term"));
        }

        let text = self.input[start..self.pos].to_string();
        Ok(Term::Word(text))
    }

    // After seeing `name:`, decide whether this is the regex prefix (which
    // switches the entire query into regex mode) or a normal filter.
    fn parse_filter_term(&mut self, name: String) -> Result<Term, ParseError> {
        if name.eq_ignore_ascii_case("regex") {
            let pattern = self.input[self.pos..].trim();
            if pattern.is_empty() {
                return Err(self.error("regex: requires a pattern"));
            }
            self.pos = self.input.len();
            return Ok(Term::Regex(pattern.to_string()));
        }

        let kind = FilterKind::from_name(&name);
        let argument = self.parse_filter_argument(&kind)?;
        Ok(Term::Filter(Filter { kind, argument }))
    }

    // Extracts the argument immediately following `name:`. This function is
    // deliberately permissive because Everything accepts many free-form tokens
    // (dates, macros, etc.), so we only stop once whitespace or another control
    // character appears.
    fn parse_filter_argument(
        &mut self,
        kind: &FilterKind,
    ) -> Result<Option<FilterArgument>, ParseError> {
        self.skip_ws();
        if self.eof() {
            return Ok(None);
        }

        if self.starts_with_filter_token() {
            return Ok(None);
        }

        if self.is_at_group_close() || matches!(self.peek_char(), Some('|')) {
            return Ok(None);
        }

        if self.peek_char() == Some('"') {
            let text = self.parse_phrase_string()?;
            let kind = ArgumentKind::Phrase;
            return Ok(Some(FilterArgument { raw: text, kind }));
        }

        let start = self.pos;
        let mut buffer = String::new();
        while let Some(ch) = self.peek_char() {
            if ch.is_whitespace() || ch == '|' {
                break;
            }

            if self.current_closer_is(ch) {
                break;
            }

            if (ch == '<' || ch == '>') && buffer.is_empty() {
                buffer.push(ch);
                self.advance_char();
                continue;
            }

            if (ch == '<' || ch == '>') && !buffer.is_empty() {
                break;
            }

            buffer.push(ch);
            self.advance_char();
        }

        if buffer.is_empty() {
            self.pos = start;
            return Ok(None);
        }

        let argument_kind = classify_argument(kind, &buffer, false);
        Ok(Some(FilterArgument {
            raw: buffer,
            kind: argument_kind,
        }))
    }

    // Everything supports literal double-quoted phrases without escape syntax.
    // We still surface a parse error if the closing quote is missing so callers
    // can provide useful feedback.
    fn parse_phrase_string(&mut self) -> Result<String, ParseError> {
        let quote_pos = self.pos;
        self.advance_char(); // opening quote
        let mut result = String::new();
        while let Some(ch) = self.peek_char() {
            self.advance_char();
            if ch == '"' {
                return Ok(result);
            }
            result.push(ch);
        }

        Err(ParseError {
            message: "missing closing quote".into(),
            position: quote_pos,
        })
    }

    fn skip_ws(&mut self) {
        while let Some(ch) = self.peek_char() {
            if ch.is_whitespace() {
                self.advance_char();
            } else {
                break;
            }
        }
    }

    fn consume_keyword(&mut self, keyword: &str) -> bool {
        let rest = self.remaining();
        if rest.len() < keyword.len() || !rest.is_char_boundary(keyword.len()) {
            return false;
        }
        let candidate = &rest[..keyword.len()];
        if !candidate.eq_ignore_ascii_case(keyword) {
            return false;
        }
        if let Some(next) = rest[keyword.len()..].chars().next() {
            if !is_keyword_boundary_char(next) {
                return false;
            }
        }
        self.pos += keyword.len();
        true
    }

    fn current_closer_is(&self, ch: char) -> bool {
        matches!(self.group_stack.last(), Some(&closer) if closer == ch)
    }

    fn is_at_group_close(&self) -> bool {
        matches!((self.group_stack.last(), self.peek_char()), (Some(&closer), Some(ch)) if closer == ch)
    }

    fn remaining(&self) -> &'a str {
        &self.input[self.pos..]
    }

    fn peek_char(&self) -> Option<char> {
        self.remaining().chars().next()
    }

    fn advance_char(&mut self) {
        if let Some(ch) = self.peek_char() {
            self.pos += ch.len_utf8();
        }
    }

    fn eof(&self) -> bool {
        self.pos >= self.input.len()
    }

    fn error(&self, message: impl Into<String>) -> ParseError {
        ParseError {
            message: message.into(),
            position: self.pos,
        }
    }

    // Determines whether the cursor is sitting at the next `name:` token so
    // filters such as `video: size:>1gb` don't treat the second filter as the
    // first one's argument.
    fn starts_with_filter_token(&self) -> bool {
        let rest = self.remaining();
        for (idx, ch) in rest.char_indices() {
            if ch.is_whitespace() || is_term_breaker(ch) {
                break;
            }
            if ch == ':' {
                if idx == 0 {
                    return false;
                }
                if !rest.is_char_boundary(idx) {
                    return false;
                }
                let name = &rest[..idx];
                if !is_valid_filter_name(name) {
                    return false;
                }
                return true;
            }
        }
        false
    }
}

fn is_term_breaker(ch: char) -> bool {
    ch.is_whitespace() || matches!(ch, '|' | '<' | '>' | '(' | ')' | '!')
}

fn is_keyword_boundary_char(ch: char) -> bool {
    ch.is_whitespace()
        || matches!(
            ch,
            '|' | '<' | '>' | '(' | ')' | '!' | ':' | '"' | '\\' | '/' | ';' | ',' | '.'
        )
        || matches!(ch, '[' | ']' | '{' | '}' | '#')
        || ch == '\0'
}

fn is_valid_filter_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
}

/// Lightweight heuristic classification so downstream code can handle the most
/// common filter syntaxes without writing custom parsers.
fn classify_argument(kind: &FilterKind, raw: &str, quoted: bool) -> ArgumentKind {
    if quoted {
        return ArgumentKind::Phrase;
    }

    if let Some(list) = try_parse_list(raw) {
        return ArgumentKind::List(list);
    }

    if let Some(comparison) = try_parse_comparison(raw) {
        return ArgumentKind::Comparison(comparison);
    }

    if let Some(range) = try_parse_range(kind, raw) {
        return ArgumentKind::Range(range);
    }

    ArgumentKind::Bare
}

/// Splits `foo;bar;baz` style extension lists.
fn try_parse_list(raw: &str) -> Option<Vec<String>> {
    if !raw.contains(';') {
        return None;
    }

    let parts: Vec<String> = raw
        .split(';')
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .map(|p| p.to_string())
        .collect();

    if parts.len() > 1 { Some(parts) } else { None }
}

/// Detects `<, <=, >, >=, =, !=` prefixes.
fn try_parse_comparison(raw: &str) -> Option<ComparisonValue> {
    let operators = ["<=", ">=", "!=", "<", ">", "="];
    for op in operators {
        if let Some(value) = raw.strip_prefix(op) {
            let value = value.trim().to_string();
            if value.is_empty() {
                return None;
            }
            let op_enum = match op {
                "<" => ComparisonOp::Lt,
                "<=" => ComparisonOp::Lte,
                ">" => ComparisonOp::Gt,
                ">=" => ComparisonOp::Gte,
                "=" => ComparisonOp::Eq,
                "!=" => ComparisonOp::Ne,
                _ => continue,
            };
            return Some(ComparisonValue { op: op_enum, value });
        }
    }
    None
}

/// Recognizes either dotted ranges (`a..b`) or hyphenated date ranges depending
/// on the filter name.
fn try_parse_range(kind: &FilterKind, raw: &str) -> Option<RangeValue> {
    if let Some(range) = try_parse_dotted_range(raw) {
        return Some(range);
    }

    if allows_hyphen_range(kind) {
        if let Some(range) = try_parse_hyphen_range(raw) {
            return Some(range);
        }
    }

    None
}

/// Parses `start..end` expressions. Empty endpoints stay `None` to signal open
/// intervals.
fn try_parse_dotted_range(raw: &str) -> Option<RangeValue> {
    let separator = "..";
    let index = raw.find(separator)?;
    let start_raw = raw[..index].trim();
    let end_raw = raw[index + separator.len()..].trim();
    if start_raw.is_empty() && end_raw.is_empty() {
        return None;
    }
    if !has_digit(start_raw) && !has_digit(end_raw) {
        return None;
    }
    Some(RangeValue {
        start: if start_raw.is_empty() {
            None
        } else {
            Some(start_raw.to_string())
        },
        end: if end_raw.is_empty() {
            None
        } else {
            Some(end_raw.to_string())
        },
        separator: RangeSeparator::Dots,
    })
}

/// Hyphenated ranges are ambiguous with subtraction, so we only accept them
/// when both sides look date-like.
fn try_parse_hyphen_range(raw: &str) -> Option<RangeValue> {
    for (idx, ch) in raw.char_indices() {
        if ch != '-' {
            continue;
        }
        let left = raw[..idx].trim();
        let right = raw[idx + 1..].trim();
        if left.is_empty() || right.is_empty() {
            continue;
        }
        if looks_like_date_fragment(left) && looks_like_date_fragment(right) {
            return Some(RangeValue {
                start: Some(left.to_string()),
                end: Some(right.to_string()),
                separator: RangeSeparator::Hyphen,
            });
        }
    }
    None
}

/// Only date-related filters accept hyphenated ranges.
fn allows_hyphen_range(kind: &FilterKind) -> bool {
    matches!(
        kind,
        FilterKind::DateCreated
            | FilterKind::DateModified
            | FilterKind::DateAccessed
            | FilterKind::DateRun
    )
}

fn has_digit(value: &str) -> bool {
    value.chars().any(|c| c.is_ascii_digit())
}

fn looks_like_date_fragment(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return false;
    }
    if !has_digit(trimmed) {
        return false;
    }
    let slash = trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains('.');
    let hyphen_count = trimmed.chars().filter(|&c| c == '-').count();
    slash || hyphen_count >= 2
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(text: &str) -> Expr {
        Expr::Term(Term::Word(text.to_string()))
    }

    #[test]
    fn parses_empty_query() {
        let query = parse_query("   ").unwrap();
        assert!(query.is_empty());
    }

    #[test]
    fn parses_boolean_logic_with_expected_precedence() {
        let query = parse_query("foo bar|\"baz qux\" !temp").unwrap();
        assert_eq!(
            query.expr,
            Expr::And(vec![
                word("foo"),
                Expr::Or(vec![
                    word("bar"),
                    Expr::Term(Term::Phrase("baz qux".into()))
                ]),
                Expr::Not(Box::new(word("temp"))),
            ])
        );
    }

    #[test]
    fn supports_textual_boolean_keywords() {
        let query = parse_query("foo AND bar").unwrap();
        assert_eq!(query.expr, Expr::And(vec![word("foo"), word("bar")]));

        let query = parse_query("foo OR bar").unwrap();
        assert_eq!(query.expr, Expr::Or(vec![word("foo"), word("bar")]));

        let query = parse_query("NOT temp").unwrap();
        assert_eq!(query.expr, Expr::Not(Box::new(word("temp"))));

        let query = parse_query("NOT/Users").unwrap();
        assert_eq!(
            query.expr,
            Expr::Not(Box::new(Expr::Term(Term::Word("/Users".into()))))
        );
    }

    #[test]
    fn parses_unix_style_paths() {
        let query = parse_query("/Users/demo/Documents report").unwrap();
        let Expr::And(parts) = query.expr else {
            panic!("expected conjunction");
        };
        assert_eq!(parts.len(), 2);

        let Expr::Term(Term::Word(path)) = &parts[0] else {
            panic!();
        };
        assert_eq!(path, "/Users/demo/Documents");
        assert_eq!(parts[1], word("report"));

        let query = parse_query("/Volumes/Data OR /Users").unwrap();
        let Expr::Or(options) = query.expr else {
            panic!("expected OR expression");
        };
        assert_eq!(options.len(), 2);
        assert!(matches!(
            &options[0],
            Expr::Term(Term::Word(path)) if path == "/Volumes/Data"
        ));
        assert!(matches!(
            &options[1],
            Expr::Term(Term::Word(path)) if path == "/Users"
        ));
    }

    #[test]
    fn parses_grouping_expression() {
        let query = parse_query("<D: | E:> *.mp3").unwrap();
        let Expr::And(parts) = query.expr else {
            panic!("expected conjunction");
        };
        assert_eq!(parts.len(), 2);

        let Expr::Or(choices) = &parts[0] else {
            panic!("expected OR group");
        };
        assert_eq!(choices.len(), 2);
        let mut names = Vec::new();
        for choice in choices {
            let Expr::Term(Term::Filter(filter)) = choice else {
                panic!("expected filter term");
            };
            assert!(filter.argument.is_none());
            let FilterKind::Custom(name) = &filter.kind else {
                panic!("expected drive-like custom filter");
            };
            names.push(name.as_str());
        }
        assert_eq!(names, ["D", "E"]);

        assert_eq!(parts[1], word("*.mp3"));
    }

    #[test]
    fn parses_parentheses_group() {
        let query = parse_query("(foo bar) baz").unwrap();
        let Expr::And(parts) = query.expr else {
            panic!("expected conjunction");
        };
        assert_eq!(
            parts,
            vec![Expr::And(vec![word("foo"), word("bar")]), word("baz")]
        );
    }

    #[test]
    fn parses_and_with_leading_empty_operand() {
        let query = parse_query("  AND foo").unwrap();
        assert_eq!(query.expr, Expr::And(vec![Expr::Empty, word("foo")]));
    }

    #[test]
    fn parses_and_with_trailing_empty_operand() {
        let query = parse_query("foo AND ").unwrap();
        assert_eq!(query.expr, Expr::And(vec![word("foo"), Expr::Empty]));
    }

    #[test]
    fn parses_nested_groups() {
        let query = parse_query("(foo <bar|baz>) qux").unwrap();
        let Expr::And(parts) = query.expr else {
            panic!("expected conjunction");
        };
        assert_eq!(parts.len(), 2);

        let Expr::And(group_parts) = &parts[0] else {
            panic!("expected grouped conjunction");
        };
        assert_eq!(group_parts.len(), 2);
        assert_eq!(group_parts[0], word("foo"));

        let Expr::Or(region) = &group_parts[1] else {
            panic!("expected OR group");
        };
        assert_eq!(region.len(), 2);
        assert_eq!(region[0], word("bar"));
        assert_eq!(region[1], word("baz"));
        assert_eq!(parts[1], word("qux"));
    }

    #[test]
    fn parses_or_with_trailing_empty_operand() {
        let query = parse_query("kksk | ").unwrap();
        assert_eq!(query.expr, Expr::Or(vec![word("kksk"), Expr::Empty]));
    }

    #[test]
    fn parses_or_with_only_empty_operands() {
        let query = parse_query(" | ").unwrap();
        assert_eq!(query.expr, Expr::Or(vec![Expr::Empty, Expr::Empty]));
    }

    #[test]
    fn parses_or_with_leading_empty_operand() {
        let query = parse_query("| foo").unwrap();
        assert_eq!(query.expr, Expr::Or(vec![Expr::Empty, word("foo")]));
    }

    #[test]
    fn parses_and_with_only_empty_operands() {
        let query = parse_query(" AND ").unwrap();
        assert_eq!(query.expr, Expr::And(vec![Expr::Empty, Expr::Empty]));
    }

    #[test]
    fn parses_or_with_consecutive_separators() {
        let query = parse_query("foo||bar").unwrap();
        assert_eq!(
            query.expr,
            Expr::Or(vec![word("foo"), Expr::Empty, word("bar")])
        );
    }

    #[test]
    fn parses_or_with_empty_operands_on_both_sides() {
        let query = parse_query("| foo |").unwrap();
        assert_eq!(
            query.expr,
            Expr::Or(vec![Expr::Empty, word("foo"), Expr::Empty])
        );
    }

    #[test]
    fn parses_filters_with_lists_and_ranges() {
        let query = parse_query("ext:txt;doc size:1mb..10mb").unwrap();
        let Expr::And(parts) = query.expr else {
            panic!("expected conjunction");
        };
        assert_eq!(parts.len(), 2);

        let Expr::Term(Term::Filter(ext)) = &parts[0] else {
            panic!("expected filter");
        };
        assert!(matches!(ext.kind, FilterKind::Ext));
        let ArgumentKind::List(values) = &ext.argument.as_ref().unwrap().kind else {
            panic!("expected list argument");
        };
        assert_eq!(values, &["txt", "doc"]);

        let Expr::Term(Term::Filter(size)) = &parts[1] else {
            panic!("expected filter");
        };
        let ArgumentKind::Range(range) = &size.argument.as_ref().unwrap().kind else {
            panic!("expected range argument");
        };
        assert_eq!(range.start.as_deref(), Some("1mb"));
        assert_eq!(range.end.as_deref(), Some("10mb"));
        assert!(matches!(range.separator, RangeSeparator::Dots));
    }

    #[test]
    fn parses_date_range_with_hyphen() {
        let query = parse_query("dc:2014/8/1-2014/8/31").unwrap();
        let Expr::Term(Term::Filter(filter)) = query.expr else {
            panic!("expected filter term");
        };
        let ArgumentKind::Range(range) = filter.argument.unwrap().kind else {
            panic!("expected range");
        };
        assert_eq!(range.start.as_deref(), Some("2014/8/1"));
        assert_eq!(range.end.as_deref(), Some("2014/8/31"));
        assert!(matches!(range.separator, RangeSeparator::Hyphen));
    }

    #[test]
    fn parses_comparison_arguments() {
        let query = parse_query("size:>1GB width:<=4000").unwrap();
        let Expr::And(parts) = query.expr else {
            panic!("expected AND expression");
        };
        let Expr::Term(Term::Filter(size)) = &parts[0] else {
            panic!();
        };
        let ArgumentKind::Comparison(ComparisonValue { op, value }) =
            &size.argument.as_ref().unwrap().kind
        else {
            panic!();
        };
        assert_eq!(*op, ComparisonOp::Gt);
        assert_eq!(value, "1GB");

        let Expr::Term(Term::Filter(width)) = &parts[1] else {
            panic!();
        };
        let ArgumentKind::Comparison(ComparisonValue { op, value }) =
            &width.argument.as_ref().unwrap().kind
        else {
            panic!();
        };
        assert_eq!(*op, ComparisonOp::Lte);
        assert_eq!(value, "4000");
    }

    #[test]
    fn parses_unc_paths_and_child_filter() {
        let query = parse_query("\\\\srv\\share child:*.mp3").unwrap();
        let Expr::And(parts) = query.expr else {
            panic!();
        };
        let Expr::Term(Term::Word(path)) = &parts[0] else {
            panic!();
        };
        assert_eq!(path, "\\\\srv\\share");

        let Expr::Term(Term::Filter(child)) = &parts[1] else {
            panic!();
        };
        assert!(matches!(child.kind, FilterKind::Child));
        assert_eq!(child.argument.as_ref().unwrap().raw, "*.mp3");
    }

    #[test]
    fn parses_regex_prefix() {
        let query = parse_query(" regex:[^\\x00-\\x7f] ").unwrap();
        assert_eq!(query.expr, Expr::Term(Term::Regex("[^\\x00-\\x7f]".into())));
    }

    #[test]
    fn reports_unmatched_groups() {
        let err = parse_query("<foo bar").unwrap_err();
        assert!(err.message.contains("expected '>'"));
    }

    #[derive(Debug)]
    struct DocExample {
        line: usize,
        query: &'static str,
    }

    fn assert_doc_examples(examples: &[DocExample]) {
        for example in examples {
            if let Err(err) = parse_query(example.query) {
                panic!(
                    "manual example at line {} failed to parse `{}`: {}",
                    example.line, example.query, err
                );
            }
        }
    }

    #[test]
    fn parses_all_manual_examples() {
        const EXAMPLES: &[DocExample] = &[
            DocExample {
                line: 9,
                query: "reports",
            },
            DocExample {
                line: 13,
                query: "D:",
            },
            DocExample {
                line: 13,
                query: r#"D:\Downloads\"#,
            },
            DocExample {
                line: 17,
                query: r#""C:\Program Files\""#,
            },
            DocExample {
                line: 19,
                query: r#"documents\"#,
            },
            DocExample {
                line: 27,
                query: r#""summer holiday""#,
            },
            DocExample {
                line: 33,
                query: "foo bar",
            },
            DocExample {
                line: 37,
                query: "foo|bar",
            },
            DocExample {
                line: 41,
                query: "!temp",
            },
            DocExample {
                line: 47,
                query: "<D: | E:> *.mp3",
            },
            DocExample {
                line: 57,
                query: r#""Program Files""#,
            },
            DocExample {
                line: 59,
                query: "foo AND bar",
            },
            DocExample {
                line: 61,
                query: "NOT temp",
            },
            DocExample {
                line: 81,
                query: "*.mp3",
            },
            DocExample {
                line: 81,
                query: "report-??.txt",
            },
            DocExample {
                line: 87,
                query: "a*b",
            },
            DocExample {
                line: 102,
                query: "file: 报告",
            },
            DocExample {
                line: 102,
                query: "folder: 项目",
            },
            DocExample {
                line: 107,
                query: "ext:txt;doc;docx",
            },
            DocExample {
                line: 114,
                query: "type:picture",
            },
            DocExample {
                line: 114,
                query: "type:video",
            },
            DocExample {
                line: 114,
                query: "type:document",
            },
            DocExample {
                line: 114,
                query: "audio:",
            },
            DocExample {
                line: 114,
                query: "video:",
            },
            DocExample {
                line: 114,
                query: "doc:",
            },
            DocExample {
                line: 114,
                query: "exe:",
            },
            DocExample {
                line: 123,
                query: "size:>1GB",
            },
            DocExample {
                line: 152,
                query: "dm:today",
            },
            DocExample {
                line: 152,
                query: "dc:thisweek",
            },
            DocExample {
                line: 152,
                query: "dc:2023-01-01..2023-12-31",
            },
            DocExample {
                line: 170,
                query: r"parent:C:\Windows",
            },
            DocExample {
                line: 174,
                query: r"D:\Music\",
            },
            DocExample {
                line: 174,
                query: r"D:\Music\ !child:*.mp3",
            },
            DocExample {
                line: 183,
                query: "attrib:H",
            },
            DocExample {
                line: 183,
                query: "attrib:R",
            },
            DocExample {
                line: 250,
                query: "dimensions:1920x1080",
            },
            DocExample {
                line: 250,
                query: "width:>4000",
            },
            DocExample {
                line: 276,
                query: "dupe: *.mp4",
            },
            DocExample {
                line: 276,
                query: "size:>1gb sizedupe:",
            },
            DocExample {
                line: 284,
                query: "folder: dm:pastmonth ext:docx report",
            },
            DocExample {
                line: 296,
                query: r"regex:^Report.*2025$",
            },
            DocExample {
                line: 332,
                query: r"regex:^(?i)Readme\.",
            },
            DocExample {
                line: 332,
                query: r"regex:[0-9]{4}",
            },
            DocExample {
                line: 332,
                query: r"regex:\.jpe?g$",
            },
            DocExample {
                line: 337,
                query: r"regex:error",
            },
            DocExample {
                line: 337,
                query: "*.log",
            },
            DocExample {
                line: 344,
                query: "ABC 123",
            },
            DocExample {
                line: 349,
                query: "ABC|123",
            },
            DocExample {
                line: 359,
                query: "case:ABC",
            },
            DocExample {
                line: 367,
                query: "D:|E: *.mp3",
            },
            DocExample {
                line: 370,
                query: "D: *.jpg|*.png",
            },
            DocExample {
                line: 373,
                query: "!.",
            },
            DocExample {
                line: 378,
                query: "file:",
            },
            DocExample {
                line: 378,
                query: "folder:",
            },
            DocExample {
                line: 390,
                query: "size:>1mb",
            },
            DocExample {
                line: 390,
                query: "size:1mb..10mb",
            },
            DocExample {
                line: 396,
                query: "dm:today",
            },
            DocExample {
                line: 399,
                query: "dc:thisweek",
            },
            DocExample {
                line: 402,
                query: "dc:1/8/2014-31/8/2014",
            },
            DocExample {
                line: 402,
                query: "dc:2014/8/1-2014/8/31",
            },
            DocExample {
                line: 407,
                query: r"D:\music\ !child:*.mp3",
            },
            DocExample {
                line: 412,
                query: r"regex:[^\x00-\x7f]",
            },
            DocExample {
                line: 428,
                query: "video: size:>1gb",
            },
            DocExample {
                line: 439,
                query: "audio: year:2024",
            },
            DocExample {
                line: 439,
                query: "musiclastyear:",
            },
            DocExample {
                line: 478,
                query: r"%TEMP%\*.log",
            },
            DocExample {
                line: 486,
                query: "ext:pdf report",
            },
            DocExample {
                line: 501,
                query: "Pictures vacation",
            },
            DocExample {
                line: 501,
                query: "report ext:docx",
            },
        ];

        assert_doc_examples(EXAMPLES);
    }
}
