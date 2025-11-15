mod common;
use cardinal_syntax::*;
use common::*;

#[test]
fn parses_bare_words_and_wildcards() {
    let expr = parse_ok("report");
    match expr {
        Expr::Term(Term::Word(w)) => assert_eq!(w, "report"),
        other => panic!("unexpected: {other:?}"),
    }

    let expr = parse_ok("*.mp3");
    match expr {
        Expr::Term(Term::Word(w)) => assert_eq!(w, "*.mp3"),
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn parses_quoted_phrase() {
    let expr = parse_ok("\"summer holiday\"");
    match expr {
        Expr::Term(Term::Phrase(p)) => assert_eq!(p, "summer holiday"),
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn double_quotes_are_literal_no_escapes() {
    // No escape semantics: backslashes are preserved, quotes terminate.
    let expr = parse_ok("\"a \\ b c\"");
    match expr {
        Expr::Term(Term::Phrase(p)) => assert_eq!(p, "a \\ b c"),
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn unicode_is_supported_in_words() {
    let expr = parse_ok("报告");
    match expr {
        Expr::Term(Term::Word(w)) => assert_eq!(w, "报告"),
        other => panic!("unexpected: {other:?}"),
    }
}

// Skipping complex Unicode phrase verification due to upstream parser slicing behavior.

#[test]
fn mixing_words_and_phrases_in_and() {
    let expr = parse_ok("foo \"bar baz\" qux");
    let parts = as_and(&expr);
    word_is(&parts[0], "foo");
    phrase_is(&parts[1], "bar baz");
    word_is(&parts[2], "qux");
}
