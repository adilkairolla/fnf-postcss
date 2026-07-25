//! Splitting comma- and space-separated CSS values without breaking strings,
//! functions or escapes.
//!
//! Port of `lib/list.js`.

/// Splits on top-level commas.
///
/// ```
/// # use postcss::list;
/// assert_eq!(list::comma("black, linear-gradient(white, black)"),
///            vec!["black", "linear-gradient(white, black)"]);
/// ```
pub fn comma(string: &str) -> Vec<String> {
    split(string, &[','], true)
}

/// Splits on top-level whitespace.
///
/// ```
/// # use postcss::list;
/// assert_eq!(list::space("1px calc(10px + 1px) solid"),
///            vec!["1px", "calc(10px + 1px)", "solid"]);
/// ```
pub fn space(string: &str) -> Vec<String> {
    split(string, &[' ', '\n', '\t'], false)
}

/// Splits on any of `separators`, ignoring ones inside quotes, parentheses or
/// after a backslash.
///
/// With `last`, a trailing empty part is kept, which is what makes
/// `comma("a,")` return two items.
pub fn split(string: &str, separators: &[char], last: bool) -> Vec<String> {
    let mut array: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut split_here = false;

    let mut func = 0usize;
    let mut in_quote = false;
    let mut prev_quote = '\0';
    let mut escape = false;

    for letter in string.chars() {
        if escape {
            escape = false;
        } else if letter == '\\' {
            escape = true;
        } else if in_quote {
            if letter == prev_quote {
                in_quote = false;
            }
        } else if letter == '"' || letter == '\'' {
            in_quote = true;
            prev_quote = letter;
        } else if letter == '(' {
            func += 1;
        } else if letter == ')' {
            // An unbalanced `)` is ignored rather than going negative.
            func = func.saturating_sub(1);
        } else if func == 0 && separators.contains(&letter) {
            split_here = true;
        }

        if split_here {
            if !current.is_empty() {
                array.push(current.trim().to_string());
            }
            current = String::new();
            split_here = false;
        } else {
            current.push(letter);
        }
    }

    if last || !current.is_empty() {
        array.push(current.trim().to_string());
    }
    array
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_comma() {
        assert_eq!(comma("a, b"), vec!["a", "b"]);
        assert_eq!(comma("a,b"), vec!["a", "b"]);
        assert_eq!(comma(""), vec![""]);
        assert_eq!(comma("a,"), vec!["a", ""]);
        // A leading separator produces no empty first item, since the buffer is
        // still empty when the split happens.
        assert_eq!(comma(",a"), vec!["a"]);
        assert_eq!(comma("a, b(a, b)"), vec!["a", "b(a, b)"]);
        assert_eq!(comma(r#"a, "b,c""#), vec!["a", "\"b,c\""]);
        assert_eq!(comma(r#"a, 'b,c'"#), vec!["a", "'b,c'"]);
        assert_eq!(comma(r"a, b\,c"), vec!["a", r"b\,c"]);
    }

    #[test]
    fn splits_on_space() {
        assert_eq!(space("a b"), vec!["a", "b"]);
        assert_eq!(space("a\nb\tc"), vec!["a", "b", "c"]);
        assert_eq!(space("a b(a b)"), vec!["a", "b(a b)"]);
        assert_eq!(space(r#"a "b c""#), vec!["a", "\"b c\""]);
        assert_eq!(space(r"a b\ c"), vec!["a", r"b\ c"]);
    }

    #[test]
    fn ignores_unclosed_parens() {
        assert_eq!(space("border-radius: 10px / 20px"), vec!["border-radius:", "10px", "/", "20px"]);
        assert_eq!(comma("a, b)"), vec!["a", "b)"]);
    }
}
