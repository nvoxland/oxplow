//! How well one search entry matches a query: its **tier**, decided from
//! the entry's name and title alone, and the order inside a tier.
//!
//! The search index finds candidates two ways (`search_store`): by name
//! (`search_name`, a trigram index: a substring of a name) and by text
//! (`search_fts`, BM25 over title and body). Every candidate is then
//! ranked here, whatever found it, so a page, a command, a file and a task
//! compare on one scale:
//!
//! | Tier | Match |
//! |---|---|
//! | 0 | the query is the name, the title or the id |
//! | 1 | the name starts with the query |
//! | 2 | each word of the query starts a word of the name |
//! | 3 | … or is in the name (3 characters or more) |
//! | 4 | … or starts a word of the title (a file's folders) |
//! | 5 | the text matched only in the body |
//!
//! A word is split at anything but a letter or digit and at a camelCase
//! hump, and a query word may run on across words (`openoverlay` starts
//! `Open` in `QuickOpenOverlay`). There is no subsequence matching: a
//! query's letters scattered across a path (`d`…`e`…`v`) is not a match.

use std::cmp::Ordering;

/// The shortest query word matched inside a name rather than at a word's
/// start: below it, "in the name" matches nearly everything.
pub const MIN_SUBSTRING: usize = 3;

/// The best tier: an exact match.
pub const EXACT: u8 = 0;
/// The tier of a match in the body alone.
pub const BODY: u8 = 5;

/// One candidate as the ranking sees it.
#[derive(Debug, Clone, Copy)]
pub struct Candidate<'a> {
    /// What a person types for it: a file's name, a page's or a task's
    /// title, a command's label.
    pub name: &'a str,
    /// Its title: a file's path, a command's group and label.
    pub title: &'a str,
    /// Its id (`tsk12`), when its URI has one.
    pub id: Option<&'a str>,
    /// Its BM25 score when the text index matched it (lower is better).
    pub bm25: Option<f64>,
}

/// The query's words, lowercased.
pub fn query_words(query: &str) -> Vec<String> {
    query.split_whitespace().map(str::to_lowercase).collect()
}

/// The candidate's tier for `query`, or none when it doesn't match.
pub fn tier(query: &str, c: &Candidate) -> Option<u8> {
    let q = query.trim().to_lowercase();
    let words = query_words(&q);
    if words.is_empty() {
        return None;
    }
    let name = c.name.to_lowercase();
    let title = c.title.to_lowercase();
    if name == q || title == q || c.id.is_some_and(|id| id.to_lowercase() == q) {
        return Some(EXACT);
    }
    if name.starts_with(&q) {
        return Some(1);
    }
    let name_words = Words::of(c.name);
    if words.iter().all(|w| name_words.started_by(w)) {
        return Some(2);
    }
    let in_name = |w: &String| {
        name_words.started_by(w)
            || (w.chars().count() >= MIN_SUBSTRING && name.contains(w.as_str()))
    };
    if words.iter().all(in_name) {
        return Some(3);
    }
    let title_words = Words::of(c.title);
    if words
        .iter()
        .all(|w| in_name(w) || title_words.started_by(w))
    {
        return Some(4);
    }
    c.bm25.map(|_| BODY)
}

/// The order of two ranked candidates: by tier; inside the body tier by
/// BM25; else the shorter name, then the shorter title, first.
pub fn compare(a: (u8, &Candidate), b: (u8, &Candidate)) -> Ordering {
    let bm25 = |t: u8, c: &Candidate| {
        if t == BODY {
            c.bm25.unwrap_or(0.0)
        } else {
            0.0
        }
    };
    a.0.cmp(&b.0)
        .then_with(|| bm25(a.0, a.1).total_cmp(&bm25(b.0, b.1)))
        .then_with(|| a.1.name.chars().count().cmp(&b.1.name.chars().count()))
        .then_with(|| a.1.title.chars().count().cmp(&b.1.title.chars().count()))
}

/// A text as its words run together (`quickopenoverlaytsx`), with where
/// each word starts in it.
struct Words {
    joined: String,
    starts: Vec<usize>,
}

impl Words {
    fn of(text: &str) -> Self {
        let mut joined = String::new();
        let mut starts = Vec::new();
        let mut prev: Option<char> = None;
        let chars: Vec<char> = text.chars().collect();
        for (i, &ch) in chars.iter().enumerate() {
            if !ch.is_alphanumeric() {
                prev = None;
                continue;
            }
            let next = chars.get(i + 1).copied();
            let starts_word = match prev {
                None => true,
                // fooBar
                Some(p) if p.is_lowercase() && ch.is_uppercase() => true,
                // HTTPServer: the S of Server
                Some(p) if p.is_uppercase() && ch.is_uppercase() => {
                    next.is_some_and(char::is_lowercase)
                }
                // v2, 2x
                Some(p) => p.is_ascii_digit() != ch.is_ascii_digit(),
            };
            if starts_word {
                starts.push(joined.len());
            }
            joined.extend(ch.to_lowercase());
            prev = Some(ch);
        }
        Self { joined, starts }
    }

    /// Whether `word` (lowercase) starts one of the words, running on
    /// across the words after it.
    fn started_by(&self, word: &str) -> bool {
        let word: String = word.chars().filter(|c| c.is_alphanumeric()).collect();
        if word.is_empty() {
            return false;
        }
        self.starts
            .iter()
            .any(|&at| self.joined[at..].starts_with(&word))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(name: &str, title: &str) -> Candidate<'static> {
        Candidate {
            name: Box::leak(name.to_string().into_boxed_str()),
            title: Box::leak(title.to_string().into_boxed_str()),
            id: None,
            bm25: None,
        }
    }

    fn file(path: &str) -> Candidate<'static> {
        named(path.rsplit('/').next().unwrap(), path)
    }

    #[test]
    fn an_exact_name_title_or_id_is_tier_zero() {
        assert_eq!(tier("dev.md", &file("DEV.md")), Some(0));
        assert_eq!(tier("Work", &named("Work", "Work")), Some(0));
        let task = Candidate {
            id: Some("tsk12"),
            ..named("Fix the launcher", "Fix the launcher")
        };
        assert_eq!(tier("tsk12", &task), Some(0));
    }

    #[test]
    fn a_name_that_starts_with_the_query_is_tier_one() {
        assert_eq!(tier("dev", &file("DEV.md")), Some(1));
        assert_eq!(tier("markdownview", &file("src/MarkdownView.tsx")), Some(1));
        assert_eq!(
            tier("new str", &named("New Stream", "Stream New Stream")),
            Some(1)
        );
    }

    #[test]
    fn each_word_starting_a_word_of_the_name_is_tier_two() {
        assert_eq!(tier("view", &file("src/MarkdownView.tsx")), Some(2));
        assert_eq!(tier("openoverlay", &file("QuickOpenOverlay.tsx")), Some(2));
        assert_eq!(
            tier("stream new", &named("New Stream", "New Stream")),
            Some(2)
        );
        assert_eq!(tier("server", &named("HTTPServer", "HTTPServer")), Some(2));
    }

    #[test]
    fn a_word_inside_the_name_is_tier_three_from_three_characters() {
        assert_eq!(tier("down", &file("src/MarkdownView.tsx")), Some(3));
        assert_eq!(tier("ow", &file("src/MarkdownView.tsx")), None);
    }

    #[test]
    fn a_word_starting_a_folder_is_tier_four() {
        assert_eq!(
            tier(
                "desktop markdown",
                &file("apps/desktop/src/MarkdownView.tsx")
            ),
            Some(4)
        );
    }

    #[test]
    fn letters_scattered_across_a_path_are_no_match() {
        assert_eq!(
            tier(
                "dev",
                &file("apps/desktop/public/fonts/InterVariable.woff2")
            ),
            None
        );
    }

    #[test]
    fn a_body_only_match_is_tier_five() {
        let hit = Candidate {
            bm25: Some(-3.0),
            ..named("Fix the launcher", "Fix the launcher")
        };
        assert_eq!(tier("ranking", &hit), Some(BODY));
    }

    #[test]
    fn inside_a_tier_the_shorter_name_comes_first_and_bodies_go_by_bm25() {
        let short = file("DEV.md");
        let long = file("dev-notes.md");
        assert_eq!(compare((1, &long), (1, &short)), Ordering::Greater);
        let better = Candidate {
            bm25: Some(-5.0),
            ..named("zzzzzz", "zzzzzz")
        };
        let worse = Candidate {
            bm25: Some(-1.0),
            ..named("a", "a")
        };
        assert_eq!(compare((BODY, &better), (BODY, &worse)), Ordering::Less);
        assert_eq!(compare((2, &long), (BODY, &better)), Ordering::Less);
    }
}
