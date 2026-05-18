//! `nod-od-suite` — curated OpenDylan-flavoured fixture regression suite.
//!
//! The actual fixtures live in `fixtures/` and the runner lives in
//! `tests/run.rs`. This library crate exists only so Cargo treats
//! `tests/run.rs` as an integration test target.
//!
//! Each fixture is a small hand-written `.dylan` program in the spirit
//! of `opendylan-tests/sources/`, restricted to language features the
//! current compiler implements (no macros, no collections, no
//! conditions — those land in Sprints 17, 20, 19 respectively). The
//! curated set is intentionally narrow and is the substitute for
//! self-hosting that PLAN.md §2.7 commits to.
