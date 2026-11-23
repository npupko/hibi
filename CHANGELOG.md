# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [5.0.0] - 2026-11-23

hibi 5 is a new program, written from scratch in Rust. It gives an AI coding
agent memory that survives between sessions: the agent records a check of a
statement against evidence as a claim, `hibi status` reports which claims
need a new check because the statement or its evidence changed or their
recheck period ran out, and
`hibi cover` reports which parts of a declared scope have no claim. Its store
is plain text under `.hibi/`, committed with the code.

It does not read the store of the earlier TypeScript line (`.claims/`),
and refuses to run, `hibi init` included, while `.claims/` exists in the
repository. Move `.claims/` away first, then start a store with
`hibi init`.

The accepted limits of how a claim finds its unit again after edits are
listed at <https://hibi.mintlify.app/concepts/anchors#limits>.

## Before the Rust rewrite

hibi was rewritten from scratch in Rust, and this changelog restarts with
the first Rust release. The earlier TypeScript line ended at 0.7.0. Its
history, including its changelog, is at tag
[v0.7.0](https://github.com/npupko/hibi/blob/v0.7.0/CHANGELOG.md). Stores
written by that line (`.claims/`) are not read by the Rust version.
