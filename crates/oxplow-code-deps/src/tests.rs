use crate::{diff_edges, extract_imports, ImportKind};

fn modules(path: &str, source: &str) -> Vec<(ImportKind, String)> {
    extract_imports(&oxplow_code_metrics::for_tests::shipped(), path, source)
        .into_iter()
        .map(|e| (e.kind, e.module))
        .collect()
}

#[test]
fn rust_use_simple() {
    let src = r#"
use std::fs;
use std::collections::HashMap;
pub use crate::foo;
"#;
    let m = modules("a.rs", src);
    assert_eq!(m[0], (ImportKind::Use, "std::fs".into()));
    assert_eq!(m[1], (ImportKind::Use, "std::collections::HashMap".into()));
    assert_eq!(m[2], (ImportKind::Use, "crate::foo".into()));
}

#[test]
fn rust_use_grouped_and_alias() {
    let src = r#"
use std::{fs, io::{self, Read}};
use foo::bar as baz;
use foo::*;
extern crate serde;
"#;
    let m = modules("a.rs", src);
    let texts: Vec<&str> = m.iter().map(|(_, s)| s.as_str()).collect();
    assert!(texts.contains(&"std::fs"));
    assert!(texts.contains(&"std::io"));
    assert!(texts.contains(&"std::io::Read"));
    assert!(texts.contains(&"foo::bar"));
    assert!(texts.contains(&"foo"));
    assert!(texts.contains(&"serde"));
}

#[test]
fn typescript_imports() {
    let src = r#"
import { useState } from "react";
import * as fs from "node:fs";
import type { Foo } from "./foo";
import "./side-effect";
export { x } from "./y";
"#;
    let m = modules("a.ts", src);
    let texts: Vec<&str> = m.iter().map(|(_, s)| s.as_str()).collect();
    assert!(texts.contains(&"react"));
    assert!(texts.contains(&"node:fs"));
    assert!(texts.contains(&"./foo"));
    assert!(texts.contains(&"./side-effect"));
    assert!(texts.contains(&"./y"));
    assert!(m.iter().all(|(k, _)| matches!(k, ImportKind::Import)));
}

#[test]
fn javascript_require_and_import() {
    let src = r#"
const fs = require("fs");
import x from "y";
"#;
    let m = modules("a.js", src);
    let texts: Vec<&str> = m.iter().map(|(_, s)| s.as_str()).collect();
    assert!(texts.contains(&"fs"));
    assert!(texts.contains(&"y"));
}

#[test]
fn tsx_imports_parse_like_typescript() {
    let src = r#"
import React from "react";
import { Button } from "./Button";
const App = () => <Button/>;
"#;
    let m = modules("a.tsx", src);
    let texts: Vec<&str> = m.iter().map(|(_, s)| s.as_str()).collect();
    assert!(texts.contains(&"react"));
    assert!(texts.contains(&"./Button"));
}

#[test]
fn python_imports() {
    let src = r#"
import os
import collections.abc
from foo.bar import baz
from .sibling import thing
from ..parent import other
"#;
    let m = modules("a.py", src);
    let texts: Vec<&str> = m.iter().map(|(_, s)| s.as_str()).collect();
    assert!(texts.contains(&"os"));
    assert!(texts.contains(&"collections.abc"));
    assert!(texts.contains(&"foo.bar"));
    // Relative imports preserve the leading dots.
    assert!(texts.iter().any(|t| t.contains("sibling")));
    assert!(texts.iter().any(|t| t.contains("parent")));
}

#[test]
fn go_imports() {
    let src = r#"
package main

import "fmt"
import (
    "os"
    "github.com/foo/bar"
    alias "github.com/baz/qux"
)
"#;
    let m = modules("a.go", src);
    let texts: Vec<&str> = m.iter().map(|(_, s)| s.as_str()).collect();
    assert!(texts.contains(&"fmt"));
    assert!(texts.contains(&"os"));
    assert!(texts.contains(&"github.com/foo/bar"));
    assert!(texts.contains(&"github.com/baz/qux"));
    assert!(m.iter().all(|(k, _)| matches!(k, ImportKind::GoImport)));
}

#[test]
fn java_imports() {
    let src = r#"
package com.example;

import java.util.List;
import java.util.*;
import static java.lang.Math.PI;

class C {}
"#;
    let m = modules("A.java", src);
    let texts: Vec<&str> = m.iter().map(|(_, s)| s.as_str()).collect();
    assert!(texts.iter().any(|t| t.contains("java.util.List")));
    assert!(texts.iter().any(|t| t.contains("java.util")));
    assert!(texts.iter().any(|t| t.contains("Math")));
}

#[test]
fn c_includes() {
    let src = r#"
#include <stdio.h>
#include "local.h"

int main(void) { return 0; }
"#;
    let m = modules("a.c", src);
    let texts: Vec<&str> = m.iter().map(|(_, s)| s.as_str()).collect();
    assert!(texts.iter().any(|t| t.contains("stdio.h")));
    assert!(texts.iter().any(|t| t.contains("local.h")));
    assert!(m.iter().all(|(k, _)| matches!(k, ImportKind::Include)));
}

#[test]
fn cpp_includes_and_using() {
    let src = r#"
#include <vector>
#include "foo.hpp"
using std::cout;
using namespace foo;

int main() { return 0; }
"#;
    let m = modules("a.cpp", src);
    let kinds: Vec<ImportKind> = m.iter().map(|(k, _)| *k).collect();
    let texts: Vec<&str> = m.iter().map(|(_, s)| s.as_str()).collect();
    assert!(texts.iter().any(|t| t.contains("vector")));
    assert!(texts.iter().any(|t| t.contains("foo.hpp")));
    assert!(texts
        .iter()
        .any(|t| t.contains("cout") || t.contains("std")));
    assert!(texts.iter().any(|t| t.contains("foo")));
    assert!(kinds.iter().any(|k| matches!(k, ImportKind::Include)));
    assert!(kinds.iter().any(|k| matches!(k, ImportKind::Using)));
}

#[test]
fn clojure_ns_require() {
    let src = r#"
(ns my.ns
  (:require [foo.bar :as fb]
            [foo.baz :refer [a b]]
            qux.bare)
  (:import [java.util Date]))

(defn -main [] (fb/run))
"#;
    let m = modules("a.clj", src);
    let texts: Vec<&str> = m.iter().map(|(_, s)| s.as_str()).collect();
    assert!(texts.iter().any(|t| t == &"foo.bar"));
    assert!(texts.iter().any(|t| t == &"foo.baz"));
    assert!(texts.iter().any(|t| t == &"qux.bare"));
}

#[test]
fn unsupported_extension_yields_empty() {
    assert!(extract_imports(
        &oxplow_code_metrics::for_tests::shipped(),
        "a.md",
        "import foo from 'bar';"
    )
    .is_empty());
}

#[test]
fn diff_added_and_removed() {
    let before = r#"
use std::fs;
use std::io;
"#;
    let after = r#"
use std::fs;
use std::path::Path;
"#;
    let b = extract_imports(&oxplow_code_metrics::for_tests::shipped(), "a.rs", before);
    let a = extract_imports(&oxplow_code_metrics::for_tests::shipped(), "a.rs", after);
    let (added, removed) = diff_edges(&b, &a);
    let added_mods: Vec<&str> = added.iter().map(|e| e.module.as_str()).collect();
    let removed_mods: Vec<&str> = removed.iter().map(|e| e.module.as_str()).collect();
    assert_eq!(added_mods, vec!["std::path::Path"]);
    assert_eq!(removed_mods, vec!["std::io"]);
}
