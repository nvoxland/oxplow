#!/bin/sh
# `bun run test:collect`: every suite with its reports (JUnit + lcov), the
# run failing if either suite did. Both always run: a Rust failure must not
# skip the frontend's tests and their report (tsk1012). llvm-cov doesn't
# create its output directory, which `cargo clean` removes (tsk1075).
mkdir -p target/coverage
cargo cov
rust=$?
bun run --cwd apps/desktop test:junit
ts=$?
[ "$rust" -eq 0 ] && [ "$ts" -eq 0 ]
