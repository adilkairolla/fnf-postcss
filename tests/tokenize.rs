//! Port of the upstream `test/tokenize.test.js`.
//!
//! Token positions are byte offsets here and UTF-16 offsets in JS; every case
//! below is ASCII, so the expected numbers are unchanged from upstream.

use postcss::{Input, Token, Tokenizer, TokenizerOptions};

/// A token in the `[type, content, start?, end?]` shape upstream asserts on.
#[derive(Debug, PartialEq, Eq)]
struct Expected(&'static str, &'static str, Option<usize>, Option<usize>);

fn actual(token: Token<'_>) -> (String, String, Option<usize>, Option<usize>) {
    (
        token.kind.as_str().to_string(),
        token.content.to_string(),
        token.start,
        token.end,
    )
}

fn tokenize(
    css: &str,
    opts: TokenizerOptions,
) -> Vec<(String, String, Option<usize>, Option<usize>)> {
    let input = Input::from_css(css);
    let mut tokenizer = Tokenizer::new(&input, opts);
    let mut tokens = Vec::new();
    while !tokenizer.end_of_file() {
        let token = tokenizer
            .next_token(false)
            .expect("tokenizer error")
            .expect("token");
        tokens.push(actual(token));
    }
    tokens
}

fn run(css: &str, expected: &[Expected]) {
    run_with(css, expected, TokenizerOptions::default());
}

fn run_with(css: &str, expected: &[Expected], opts: TokenizerOptions) {
    let expected: Vec<_> = expected
        .iter()
        .map(|e| (e.0.to_string(), e.1.to_string(), e.2, e.3))
        .collect();
    assert_eq!(tokenize(css, opts), expected, "tokenizing {:?}", css);
}

/// Shorthand for a token with a start and an end.
fn t(kind: &'static str, content: &'static str, start: usize, end: usize) -> Expected {
    Expected(kind, content, Some(start), Some(end))
}

/// Shorthand for a control character, which carries only a start.
fn c(kind: &'static str, content: &'static str, start: usize) -> Expected {
    Expected(kind, content, Some(start), None)
}

/// Shorthand for a space token, which carries no positions.
fn s(content: &'static str) -> Expected {
    Expected("space", content, None, None)
}

#[test]
fn tokenizes_empty_file() {
    run("", &[]);
}

#[test]
fn tokenizes_space() {
    run("\r\n \u{c}\t", &[s("\r\n \u{c}\t")]);
}

#[test]
fn tokenizes_word() {
    run("ab", &[t("word", "ab", 0, 1)]);
}

#[test]
fn splits_word_by_bang() {
    run("aa!bb", &[t("word", "aa", 0, 1), t("word", "!bb", 2, 4)]);
}

#[test]
fn changes_lines_in_spaces() {
    run(
        "a \n b",
        &[t("word", "a", 0, 0), s(" \n "), t("word", "b", 4, 4)],
    );
}

#[test]
fn tokenizes_control_chars() {
    run(
        "{:;}",
        &[
            c("{", "{", 0),
            c(":", ":", 1),
            c(";", ";", 2),
            c("}", "}", 3),
        ],
    );
}

#[test]
fn escapes_control_symbols() {
    run(
        r#"\(\{\"\@\\"""#,
        &[
            t("word", r"\(", 0, 1),
            t("word", r"\{", 2, 3),
            t("word", r#"\""#, 4, 5),
            t("word", r"\@", 6, 7),
            t("word", r"\\", 8, 9),
            t("string", r#""""#, 10, 11),
        ],
    );
}

#[test]
fn escapes_backslash() {
    run(r"\\\\{", &[t("word", r"\\\\", 0, 3), c("{", "{", 4)]);
}

#[test]
fn tokenizes_simple_brackets() {
    run("(ab)", &[t("brackets", "(ab)", 0, 3)]);
}

#[test]
fn tokenizes_square_brackets() {
    run(
        "a[bc]",
        &[
            t("word", "a", 0, 0),
            c("[", "[", 1),
            t("word", "bc", 2, 3),
            c("]", "]", 4),
        ],
    );
}

#[test]
fn tokenizes_complicated_brackets() {
    run(
        "(())(\"\")(/**/)(\\\\)(\n)(",
        &[
            c("(", "(", 0),
            c("(", "(", 1),
            c(")", ")", 2),
            c(")", ")", 3),
            c("(", "(", 4),
            t("string", "\"\"", 5, 6),
            c(")", ")", 7),
            c("(", "(", 8),
            t("comment", "/**/", 9, 12),
            c(")", ")", 13),
            c("(", "(", 14),
            t("word", r"\\", 15, 16),
            c(")", ")", 17),
            c("(", "(", 18),
            s("\n"),
            c(")", ")", 20),
            c("(", "(", 21),
        ],
    );
}

#[test]
fn tokenizes_string() {
    run(
        r#"'"'"\"""#,
        &[t("string", "'\"'", 0, 2), t("string", r#""\"""#, 3, 6)],
    );
}

#[test]
fn tokenizes_escaped_string() {
    run(r#""\\""#, &[t("string", r#""\\""#, 0, 3)]);
}

#[test]
fn changes_lines_in_strings() {
    run(
        "\"\n\n\"\"\n\n\"",
        &[t("string", "\"\n\n\"", 0, 3), t("string", "\"\n\n\"", 4, 7)],
    );
}

#[test]
fn tokenizes_at_word() {
    run("@word ", &[t("at-word", "@word", 0, 4), s(" ")]);
}

#[test]
fn tokenizes_at_word_end() {
    run(
        "@one{@two()@three\"\"@four;",
        &[
            t("at-word", "@one", 0, 3),
            c("{", "{", 4),
            t("at-word", "@two", 5, 8),
            t("brackets", "()", 9, 10),
            t("at-word", "@three", 11, 16),
            t("string", "\"\"", 17, 18),
            t("at-word", "@four", 19, 23),
            c(";", ";", 24),
        ],
    );
}

#[test]
fn tokenizes_urls() {
    run(
        r"url(/*\))",
        &[t("word", "url", 0, 2), t("brackets", r"(/*\))", 3, 8)],
    );
}

#[test]
fn tokenizes_quoted_urls() {
    run(
        "url(\")\")",
        &[
            t("word", "url", 0, 2),
            c("(", "(", 3),
            t("string", "\")\"", 4, 6),
            c(")", ")", 7),
        ],
    );
}

#[test]
fn tokenizes_at_symbol() {
    run("@", &[t("at-word", "@", 0, 0)]);
}

#[test]
fn tokenizes_comment() {
    run("/* a\nb */", &[t("comment", "/* a\nb */", 0, 8)]);
}

#[test]
fn changes_lines_in_comments() {
    run(
        "a/* \n */b",
        &[
            t("word", "a", 0, 0),
            t("comment", "/* \n */", 1, 7),
            t("word", "b", 8, 8),
        ],
    );
}

#[test]
fn supports_line_feed() {
    run(
        "a\u{c}b",
        &[t("word", "a", 0, 0), s("\u{c}"), t("word", "b", 2, 2)],
    );
}

#[test]
fn supports_carriage_return() {
    run(
        "a\rb\r\nc",
        &[
            t("word", "a", 0, 0),
            s("\r"),
            t("word", "b", 2, 2),
            s("\r\n"),
            t("word", "c", 5, 5),
        ],
    );
}

#[test]
fn tokenizes_css() {
    let css =
        "a {\n  content: \"a\";\n  width: calc(1px;)\n  }\n/* small screen */\n@media screen {}";
    run(
        css,
        &[
            t("word", "a", 0, 0),
            s(" "),
            c("{", "{", 2),
            s("\n  "),
            t("word", "content", 6, 12),
            c(":", ":", 13),
            s(" "),
            t("string", "\"a\"", 15, 17),
            c(";", ";", 18),
            s("\n  "),
            t("word", "width", 22, 26),
            c(":", ":", 27),
            s(" "),
            t("word", "calc", 29, 32),
            t("brackets", "(1px;)", 33, 38),
            s("\n  "),
            c("}", "}", 42),
            s("\n"),
            t("comment", "/* small screen */", 44, 61),
            s("\n"),
            t("at-word", "@media", 63, 68),
            s(" "),
            t("word", "screen", 70, 75),
            s(" "),
            c("{", "{", 77),
            c("}", "}", 78),
        ],
    );
}

#[test]
fn errors_on_unclosed_string() {
    let input = Input::from_css(" \"");
    let mut tokenizer = Tokenizer::new(&input, TokenizerOptions::default());
    tokenizer.next_token(false).unwrap();
    let error = tokenizer.next_token(false).unwrap_err();
    assert_eq!(error.reason, "Unclosed string");
    assert_eq!((error.line, error.column), (Some(1), Some(2)));
}

#[test]
fn errors_on_unclosed_comment() {
    let input = Input::from_css(" /*");
    let mut tokenizer = Tokenizer::new(&input, TokenizerOptions::default());
    tokenizer.next_token(false).unwrap();
    let error = tokenizer.next_token(false).unwrap_err();
    assert_eq!(error.reason, "Unclosed comment");
    assert_eq!((error.line, error.column), (Some(1), Some(2)));
}

#[test]
fn errors_on_unclosed_url() {
    let input = Input::from_css("url(");
    let mut tokenizer = Tokenizer::new(&input, TokenizerOptions::default());
    tokenizer.next_token(false).unwrap();
    let error = tokenizer.next_token(false).unwrap_err();
    assert_eq!(error.reason, "Unclosed bracket");
    assert_eq!((error.line, error.column), (Some(1), Some(4)));
}

#[test]
fn ignores_unclosed_string_on_request() {
    run_with(
        " \"",
        &[s(" "), t("string", "\"", 1, 2)],
        TokenizerOptions {
            ignore_errors: true,
        },
    );
}

#[test]
fn ignores_unclosed_comment_on_request() {
    run_with(
        " /*",
        &[s(" "), t("comment", "/*", 1, 3)],
        TokenizerOptions {
            ignore_errors: true,
        },
    );
}

#[test]
fn ignores_unclosed_function_on_request() {
    run_with(
        "url(",
        &[t("word", "url", 0, 2), t("brackets", "(", 3, 3)],
        TokenizerOptions {
            ignore_errors: true,
        },
    );
}

#[test]
fn tokenizes_hexadecimal_escape() {
    run(
        r"\0a \09 \z ",
        &[
            t("word", r"\0a ", 0, 3),
            t("word", r"\09 ", 4, 7),
            t("word", r"\z", 8, 9),
            s(" "),
        ],
    );
}

#[test]
fn ignores_unclosed_per_token_request() {
    let input = Input::from_css("How's it going (");
    let mut tokenizer = Tokenizer::new(&input, TokenizerOptions::default());
    let mut tokens = Vec::new();
    while !tokenizer.end_of_file() {
        tokens.push(actual(tokenizer.next_token(true).unwrap().unwrap()));
    }

    let expected: Vec<_> = [
        t("word", "How", 0, 2),
        t("string", "'s", 3, 4),
        s(" "),
        t("word", "it", 6, 7),
        s(" "),
        t("word", "going", 9, 13),
        s(" "),
        c("(", "(", 15),
    ]
    .iter()
    .map(|e| (e.0.to_string(), e.1.to_string(), e.2, e.3))
    .collect();

    assert_eq!(tokens, expected);
}

#[test]
fn provides_correct_position() {
    let input = Input::from_css("Three tokens");
    let mut tokenizer = Tokenizer::new(&input, TokenizerOptions::default());
    assert_eq!(tokenizer.position(), 0);
    tokenizer.next_token(false).unwrap();
    assert_eq!(tokenizer.position(), 5);
    tokenizer.next_token(false).unwrap();
    assert_eq!(tokenizer.position(), 6);
    tokenizer.next_token(false).unwrap();
    assert_eq!(tokenizer.position(), 12);
    tokenizer.next_token(false).unwrap();
    assert_eq!(tokenizer.position(), 12);
}

#[test]
fn pushes_tokens_back() {
    let input = Input::from_css("a b");
    let mut tokenizer = Tokenizer::new(&input, TokenizerOptions::default());
    let first = tokenizer.next_token(false).unwrap().unwrap();
    tokenizer.back(first);
    assert!(!tokenizer.end_of_file());
    assert_eq!(actual(tokenizer.next_token(false).unwrap().unwrap()).1, "a");
}
