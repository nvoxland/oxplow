//! How much a file's tests check, and how many they skip — counted from its
//! text so both sides of a change can be compared (the oxplow-bundled Tests
//! Weakened lens, via `v_change_test_file`). A heuristic across languages,
//! not a parser: it counts calls that look like assertions and markers that
//! skip a test.

/// Assertion and skip counts for one version of a file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TestSignals {
    pub assertions: i64,
    pub skips: i64,
}

/// Count assertions and skip markers in `content`. Line comments (`//`,
/// and `#` lines that aren't `#[attribute]`s) are ignored, so commenting an
/// assertion out lowers the count.
pub fn count(content: &str) -> TestSignals {
    let mut out = TestSignals::default();
    for raw in content.lines() {
        let line = code_part(raw);
        if line.contains("#[ignore") {
            out.skips += 1;
        }
        for marker in ["@Disabled", "@Ignore"] {
            out.skips += line.matches(marker).count() as i64;
        }
        let tokens = tokens(line);
        for (i, &(start, tok)) in tokens.iter().enumerate() {
            let before = line[..start].chars().next_back();
            let after = line[start + tok.len()..].chars().next();
            let dotted = before == Some('.');
            let receiver = |name: &str| {
                dotted
                    && i > 0
                    && tokens[i - 1].1 == name
                    && tokens[i - 1].0 + name.len() + 1 == start
            };
            if is_assert_word(tok)
                || (tok == "expect" && after == Some('('))
                || (receiver("t") && matches!(tok, "Error" | "Errorf" | "Fatal" | "Fatalf"))
                || (tok == "require"
                    && after == Some('.')
                    && line[start + tok.len() + 1..]
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_uppercase()))
            {
                out.assertions += 1;
            } else if (dotted
                && matches!(tok, "skip" | "skipif" | "skipIf" | "skipUnless" | "todo"))
                || (receiver("t") && matches!(tok, "Skip" | "Skipf" | "SkipNow"))
                || (matches!(tok, "xit" | "xdescribe" | "xtest") && after == Some('('))
            {
                out.skips += 1;
            }
        }
    }
    out
}

/// The line without a trailing `//` comment; empty for a `#` comment line
/// (Python, shell) — but not for a `#[attribute]` or `#!`.
fn code_part(line: &str) -> &str {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') && !trimmed.starts_with("#[") && !trimmed.starts_with("#!") {
        return "";
    }
    match line.find("//") {
        Some(i) => &line[..i],
        None => line,
    }
}

/// Identifier tokens with their byte offsets.
fn tokens(line: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in line.char_indices() {
        let word = c.is_ascii_alphanumeric() || c == '_';
        match (word, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                out.push((s, &line[s..i]));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push((s, &line[s..]));
    }
    out
}

/// `assert`, `assert_eq`, `assertEqual`, `debug_assert`, … but not words
/// that merely contain it (`reassertion`).
fn is_assert_word(tok: &str) -> bool {
    tok.starts_with("assert") || tok.split('_').any(|part| part == "assert")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(src: &str) -> (i64, i64) {
        let s = count(src);
        (s.assertions, s.skips)
    }

    #[test]
    fn rust_assertions_and_ignored_tests() {
        let src = "#[test]\nfn a() {\n    assert_eq!(1, 1);\n    assert!(true);\n    debug_assert!(x);\n}\n#[test]\n#[ignore]\nfn b() { assert_ne!(1, 2) }\n";
        assert_eq!(c(src), (4, 1));
    }

    #[test]
    fn js_expectations_and_skips() {
        let src = "it('a', () => { expect(x).toBe(1); expect(y).toEqual(2); });\nit.skip('b', () => {});\nxit('c', () => {});\ndescribe.skip('d', () => {});\ntest.todo('e');\n";
        assert_eq!(c(src), (2, 4));
    }

    #[test]
    fn python_go_and_java() {
        let py = "def test_a():\n    assert x == 1\n    self.assertEqual(a, b)\n\n@pytest.mark.skip(reason='later')\ndef test_b():\n    pytest.skip('no')\n";
        assert_eq!(c(py), (2, 2));
        let go = "func TestA(t *testing.T) {\n\tif x != 1 { t.Errorf(\"bad\") }\n\tt.Fatal(\"x\")\n\tt.Skip(\"later\")\n\trequire.Equal(t, 1, x)\n}\n";
        assert_eq!(c(go), (3, 1));
        let java = "@Test void a() { assertEquals(1, x); assertThat(y).isTrue(); }\n@Disabled @Test void b() {}\n";
        assert_eq!(c(java), (2, 1));
    }

    #[test]
    fn words_that_merely_contain_assert_or_skip_do_not_count() {
        assert_eq!(
            c("let reassertion = skipper(); // not an assertion\n"),
            (0, 0)
        );
    }

    #[test]
    fn commented_out_assertions_do_not_count() {
        // Commenting an assertion out is a weakening, so it must drop the count.
        assert_eq!(c("// assert!(x);\n    # assert x == 1\n"), (0, 0));
        // `#[ignore]` is an attribute, not a comment.
        assert_eq!(c("#[ignore]\n"), (0, 1));
    }
}
