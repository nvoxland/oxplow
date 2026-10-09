//! A generated source tree: Rust, TypeScript and Python files of 100 to 300
//! lines, with a few blocks shared verbatim between files so the
//! duplicate-block detector has work to do.

use std::collections::HashMap;
use std::fmt::Write;

use super::Rng;

/// The size the containment measurements use: a realistic repository slice.
pub const DEFAULT_FILES: usize = 120;

const SEED: u64 = 0x7ee5;
const AREAS: &[&str] = &[
    "collectors",
    "commands",
    "metrics",
    "stores",
    "runtime",
    "hooks",
];
const VERBS: &[&str] = &[
    "collect",
    "resolve",
    "snapshot",
    "render",
    "dispatch",
    "reconcile",
    "decode",
    "persist",
    "observe",
    "project",
    "merge",
    "audit",
];
const NOUNS: &[&str] = &[
    "entry", "record", "window", "batch", "ledger", "segment", "cursor", "manifest", "digest",
    "bucket",
];

#[derive(Clone, Copy, PartialEq)]
enum Lang {
    Rust,
    TypeScript,
    Python,
}

const LANGS: [Lang; 3] = [Lang::Rust, Lang::TypeScript, Lang::Python];

/// `files` source files by path, in Rust, TypeScript and Python in turn.
/// A fifth of the files carry a block shared with other files.
pub fn corpus(files: usize) -> HashMap<String, String> {
    let mut rng = Rng::new(SEED);
    let shared: Vec<Vec<String>> = LANGS
        .iter()
        .map(|lang| {
            let mut pool_rng = Rng::new(SEED ^ 0xb10c);
            (0..5)
                .map(|i| pair(*lang, &mut pool_rng, 1000 + i))
                .collect()
        })
        .collect();
    let mut tree = HashMap::with_capacity(files);
    for i in 0..files {
        let lang = LANGS[i % LANGS.len()];
        let area = AREAS[(i / LANGS.len()) % AREAS.len()];
        let path = match lang {
            Lang::Rust => format!("crates/unit-{}/src/{area}/m{i:03}.rs", i % 7),
            Lang::TypeScript => format!("web/src/{area}/m{i:03}.ts"),
            Lang::Python => format!("tools/{area}/m{i:03}.py"),
        };
        let target = rng.range(100, 300);
        let mut text = header(lang, i);
        let pool = &shared[i % LANGS.len()];
        let mut serial = 0;
        while text.lines().count() < target {
            if rng.chance(20) {
                text.push_str(rng.pick(pool));
            } else {
                text.push_str(&function(lang, &mut rng, serial));
            }
            text.push('\n');
            serial += 1;
        }
        tree.insert(path, text);
    }
    tree
}

fn header(lang: Lang, i: usize) -> String {
    match lang {
        Lang::Rust => {
            format!("//! Unit {i}: generated corpus module.\n\nuse std::collections::HashMap;\n\n")
        }
        Lang::TypeScript => format!(
            "// Unit {i}: generated corpus module.\nimport {{ Entry }} from \"./entry\";\n\n"
        ),
        Lang::Python => format!(
            "\"\"\"Unit {i}: generated corpus module.\"\"\"\nfrom typing import Dict, List\n\n\n"
        ),
    }
}

/// Two functions, the unit a shared block is made of.
fn pair(lang: Lang, rng: &mut Rng, serial: usize) -> String {
    let mut out = function(lang, rng, serial);
    out.push('\n');
    out.push_str(&function(lang, rng, serial + 1));
    out
}

fn name(rng: &mut Rng, serial: usize) -> String {
    format!("{}_{}_{serial}", rng.pick(VERBS), rng.pick(NOUNS))
}

/// One function of 10 to 20 lines, one of three shapes, with its own names
/// and constants.
fn function(lang: Lang, rng: &mut Rng, serial: usize) -> String {
    let name = name(rng, serial);
    let (m, k) = (rng.range(2, 13), rng.range(1, 90));
    let mut s = String::new();
    let shape = rng.range(0, 2);
    match (lang, shape) {
        (Lang::Rust, 0) => {
            let _ = write!(s, "pub fn {name}(input: &[u64], limit: usize) -> Result<Vec<u64>, String> {{\n    let mut out = Vec::with_capacity(limit);\n    for (i, value) in input.iter().enumerate() {{\n        if i >= limit {{\n            break;\n        }}\n        if *value % {m} == 0 {{\n            out.push(*value / {m});\n        }} else {{\n            out.push(*value + {k});\n        }}\n    }}\n    if out.is_empty() {{\n        return Err(format!(\"{name}: nothing to do\"));\n    }}\n    Ok(out)\n}}\n");
        }
        (Lang::Rust, 1) => {
            let _ = write!(s, "pub fn {name}(words: &[String]) -> HashMap<String, usize> {{\n    let mut counts = HashMap::new();\n    for word in words {{\n        let key = word.trim().to_lowercase();\n        if key.len() < {m} {{\n            continue;\n        }}\n        *counts.entry(key).or_insert(0) += {k};\n    }}\n    counts.retain(|_, n| *n > {m});\n    counts\n}}\n");
        }
        (Lang::Rust, _) => {
            let _ = write!(s, "pub struct {camel} {{\n    pub id: u64,\n    pub label: String,\n    weight: f64,\n}}\n\nimpl {camel} {{\n    pub fn new(id: u64, label: &str) -> Self {{\n        Self {{ id, label: label.to_string(), weight: {k} as f64 / {m} as f64 }}\n    }}\n\n    pub fn heavier_than(&self, other: &Self) -> bool {{\n        self.weight > other.weight\n    }}\n}}\n", camel = camel(&name));
        }
        (Lang::TypeScript, 0) => {
            let _ = write!(s, "export function {name}(input: number[], limit: number): number[] {{\n  const out: number[] = [];\n  for (let i = 0; i < input.length; i++) {{\n    if (i >= limit) {{\n      break;\n    }}\n    if (input[i] % {m} === 0) {{\n      out.push(input[i] / {m});\n    }} else {{\n      out.push(input[i] + {k});\n    }}\n  }}\n  if (out.length === 0) {{\n    throw new Error(\"{name}: nothing to do\");\n  }}\n  return out;\n}}\n");
        }
        (Lang::TypeScript, 1) => {
            let _ = write!(s, "export function {name}(words: string[]): Map<string, number> {{\n  const counts = new Map<string, number>();\n  for (const word of words) {{\n    const key = word.trim().toLowerCase();\n    if (key.length < {m}) {{\n      continue;\n    }}\n    counts.set(key, (counts.get(key) ?? 0) + {k});\n  }}\n  for (const [key, n] of counts) {{\n    if (n <= {m}) counts.delete(key);\n  }}\n  return counts;\n}}\n");
        }
        (Lang::TypeScript, _) => {
            let _ = write!(s, "export class {camel} {{\n  private weight: number;\n\n  constructor(public id: number, public label: string) {{\n    this.weight = {k} / {m};\n  }}\n\n  heavierThan(other: {camel}): boolean {{\n    return this.weight > other.weight;\n  }}\n}}\n", camel = camel(&name));
        }
        (Lang::Python, 0) => {
            let _ = write!(s, "def {name}(values: List[int], limit: int) -> List[int]:\n    out = []\n    for i, value in enumerate(values):\n        if i >= limit:\n            break\n        if value % {m} == 0:\n            out.append(value // {m})\n        else:\n            out.append(value + {k})\n    if not out:\n        raise ValueError(\"{name}: nothing to do\")\n    return out\n");
        }
        (Lang::Python, 1) => {
            let _ = write!(s, "def {name}(words: List[str]) -> Dict[str, int]:\n    counts: Dict[str, int] = {{}}\n    for word in words:\n        key = word.strip().lower()\n        if len(key) < {m}:\n            continue\n        counts[key] = counts.get(key, 0) + {k}\n    return {{key: n for key, n in counts.items() if n > {m}}}\n");
        }
        (Lang::Python, _) => {
            let _ = write!(s, "class {camel}:\n    def __init__(self, ident: int, label: str) -> None:\n        self.ident = ident\n        self.label = label\n        self.weight = {k} / {m}\n\n    def heavier_than(self, other: \"{camel}\") -> bool:\n        return self.weight > other.weight\n", camel = camel(&name));
        }
    }
    s
}

fn camel(snake: &str) -> String {
    snake
        .split('_')
        .map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect())
        })
        .collect()
}
