//! Unit tests for every row of Part 2 table.
//! Testing colon vs. non-colon variants across unset, null, and non-empty values.

use xesubst::ModeConfig;
use xesubst::env::{EnvMap, VariableTable};
use xesubst::eval::expand_str;

#[test]
fn test_default_values_colon_vs_non_colon() {
    let config = ModeConfig::default();

    // 1. Unset variable
    let mut env_unset = EnvMap::new();
    assert_eq!(
        expand_str("${v:-default}", &mut env_unset, &config).unwrap(),
        "default"
    );
    assert_eq!(
        expand_str("${v-default}", &mut env_unset, &config).unwrap(),
        "default"
    );

    // 2. Null (set to empty string)
    let mut env_null = EnvMap::new();
    env_null.set("v", "");
    assert_eq!(
        expand_str("${v:-default}", &mut env_null, &config).unwrap(),
        "default"
    );
    assert_eq!(
        expand_str("${v-default}", &mut env_null, &config).unwrap(),
        ""
    );

    // 3. Non-empty
    let mut env_set = EnvMap::new();
    env_set.set("v", "present");
    assert_eq!(
        expand_str("${v:-default}", &mut env_set, &config).unwrap(),
        "present"
    );
    assert_eq!(
        expand_str("${v-default}", &mut env_set, &config).unwrap(),
        "present"
    );
}

#[test]
fn test_assign_default_colon_vs_non_colon() {
    let config = ModeConfig::default();

    // Unset: assigns in both
    let mut env1 = EnvMap::new();
    assert_eq!(
        expand_str("${v:=assigned}", &mut env1, &config).unwrap(),
        "assigned"
    );
    assert_eq!(env1.get("v").unwrap(), "assigned");

    let mut env2 = EnvMap::new();
    assert_eq!(
        expand_str("${v=assigned}", &mut env2, &config).unwrap(),
        "assigned"
    );
    assert_eq!(env2.get("v").unwrap(), "assigned");

    // Null: := assigns, = does not
    let mut env3 = EnvMap::new();
    env3.set("v", "");
    assert_eq!(
        expand_str("${v:=assigned}", &mut env3, &config).unwrap(),
        "assigned"
    );
    assert_eq!(env3.get("v").unwrap(), "assigned");

    let mut env4 = EnvMap::new();
    env4.set("v", "");
    assert_eq!(expand_str("${v=assigned}", &mut env4, &config).unwrap(), "");
    assert_eq!(env4.get("v").unwrap(), "");

    // Non-empty: neither assigns
    let mut env5 = EnvMap::new();
    env5.set("v", "keep");
    assert_eq!(
        expand_str("${v:=assigned}", &mut env5, &config).unwrap(),
        "keep"
    );
    assert_eq!(env5.get("v").unwrap(), "keep");

    let mut env6 = EnvMap::new();
    env6.set("v", "keep");
    assert_eq!(
        expand_str("${v=assigned}", &mut env6, &config).unwrap(),
        "keep"
    );
    assert_eq!(env6.get("v").unwrap(), "keep");
}

#[test]
fn test_error_if_unset_colon_vs_non_colon() {
    let config = ModeConfig::default();

    // Unset: errors in both
    let mut env1 = EnvMap::new();
    assert!(expand_str("${v:?custom error}", &mut env1, &config).is_err());
    assert!(expand_str("${v?custom error}", &mut env1, &config).is_err());

    // Null: :? errors, ? does not
    let mut env2 = EnvMap::new();
    env2.set("v", "");
    assert!(expand_str("${v:?custom error}", &mut env2, &config).is_err());
    assert_eq!(
        expand_str("${v?custom error}", &mut env2, &config).unwrap(),
        ""
    );

    // Non-empty: neither errors
    let mut env3 = EnvMap::new();
    env3.set("v", "ok");
    assert_eq!(
        expand_str("${v:?custom error}", &mut env3, &config).unwrap(),
        "ok"
    );
    assert_eq!(
        expand_str("${v?custom error}", &mut env3, &config).unwrap(),
        "ok"
    );
}

#[test]
fn test_alternate_value_colon_vs_non_colon() {
    let config = ModeConfig::default();

    // Unset: both return empty
    let mut env1 = EnvMap::new();
    assert_eq!(expand_str("${v:+alt}", &mut env1, &config).unwrap(), "");
    assert_eq!(expand_str("${v+alt}", &mut env1, &config).unwrap(), "");

    // Null: :+ returns empty, + returns alt
    let mut env2 = EnvMap::new();
    env2.set("v", "");
    assert_eq!(expand_str("${v:+alt}", &mut env2, &config).unwrap(), "");
    assert_eq!(expand_str("${v+alt}", &mut env2, &config).unwrap(), "alt");

    // Non-empty: both return alt
    let mut env3 = EnvMap::new();
    env3.set("v", "active");
    assert_eq!(expand_str("${v:+alt}", &mut env3, &config).unwrap(), "alt");
    assert_eq!(expand_str("${v+alt}", &mut env3, &config).unwrap(), "alt");
}

#[test]
fn test_length_unicode_chars() {
    let config = ModeConfig::default();
    let mut env = EnvMap::new();
    env.set("ascii", "hello");
    env.set("unicode", "café");
    env.set("emojis", "👋🌍🚀");
    env.set("empty", "");

    assert_eq!(expand_str("${#ascii}", &mut env, &config).unwrap(), "5");
    assert_eq!(expand_str("${#unicode}", &mut env, &config).unwrap(), "4");
    assert_eq!(expand_str("${#emojis}", &mut env, &config).unwrap(), "3");
    assert_eq!(expand_str("${#empty}", &mut env, &config).unwrap(), "0");
    assert_eq!(expand_str("${#unset_var}", &mut env, &config).unwrap(), "0");
}

#[test]
fn test_substring_and_arithmetic() {
    let config = ModeConfig::default();
    let mut env = EnvMap::new();
    env.set("s", "abcdefghij");
    env.set("OFFSET", "2");
    env.set("LEN", "3");

    assert_eq!(expand_str("${s:2}", &mut env, &config).unwrap(), "cdefghij");
    assert_eq!(expand_str("${s:2:3}", &mut env, &config).unwrap(), "cde");
    assert_eq!(
        expand_str("${s:OFFSET:LEN}", &mut env, &config).unwrap(),
        "cde"
    );
    assert_eq!(expand_str("${s: -3}", &mut env, &config).unwrap(), "hij");
    assert_eq!(expand_str("${s:(-3)}", &mut env, &config).unwrap(), "hij");
    assert_eq!(
        expand_str("${s:2:-2}", &mut env, &config).unwrap(),
        "cdefgh"
    );
    assert_eq!(
        expand_str("${s:1+2:2*2}", &mut env, &config).unwrap(),
        "defg"
    );
}

#[test]
fn test_prefix_suffix_removal() {
    let config = ModeConfig::default();
    let mut env = EnvMap::new();
    env.set("path", "/usr/local/bin/rustc");

    // Shortest prefix
    assert_eq!(
        expand_str("${path#/*/}", &mut env, &config).unwrap(),
        "local/bin/rustc"
    );
    // Longest prefix
    assert_eq!(
        expand_str("${path##/*/}", &mut env, &config).unwrap(),
        "rustc"
    );
    // Shortest suffix
    assert_eq!(
        expand_str("${path%/*}", &mut env, &config).unwrap(),
        "/usr/local/bin"
    );
    // Longest suffix
    assert_eq!(expand_str("${path%%/*}", &mut env, &config).unwrap(), "");
}

#[test]
fn test_pattern_replacement() {
    let config = ModeConfig::default();
    let mut env = EnvMap::new();
    env.set("msg", "foo-bar-baz-bar");

    // Replace first
    assert_eq!(
        expand_str("${msg/bar/QUX}", &mut env, &config).unwrap(),
        "foo-QUX-baz-bar"
    );
    // Replace all
    assert_eq!(
        expand_str("${msg//bar/QUX}", &mut env, &config).unwrap(),
        "foo-QUX-baz-QUX"
    );
    // Anchored prefix
    assert_eq!(
        expand_str("${msg/#foo/START}", &mut env, &config).unwrap(),
        "START-bar-baz-bar"
    );
    assert_eq!(
        expand_str("${msg/#bar/START}", &mut env, &config).unwrap(),
        "foo-bar-baz-bar"
    );
    // Anchored suffix
    assert_eq!(
        expand_str("${msg/%bar/END}", &mut env, &config).unwrap(),
        "foo-bar-baz-END"
    );
    // Omitted replacement (deletes match)
    assert_eq!(
        expand_str("${msg//-}", &mut env, &config).unwrap(),
        "foobarbazbar"
    );
}

#[test]
fn test_case_modification() {
    let config = ModeConfig::default();
    let mut env = EnvMap::new();
    env.set("name", "alICE");

    assert_eq!(expand_str("${name^}", &mut env, &config).unwrap(), "AlICE");
    assert_eq!(expand_str("${name^^}", &mut env, &config).unwrap(), "ALICE");
    assert_eq!(expand_str("${name,}", &mut env, &config).unwrap(), "alICE");
    assert_eq!(expand_str("${name,,}", &mut env, &config).unwrap(), "alice");

    // With pattern
    assert_eq!(
        expand_str("${name^^[lc]}", &mut env, &config).unwrap(),
        "aLICE"
    );
    assert_eq!(
        expand_str("${name,,[IE]}", &mut env, &config).unwrap(),
        "aliCe"
    );
}

#[test]
fn test_indirect_and_prefix_listing() {
    let config = ModeConfig::default();
    let mut env = EnvMap::new();
    env.set("TARGET", "FINAL_VALUE");
    env.set("PTR", "TARGET");

    env.set("APP_HOST", "localhost");
    env.set("APP_PORT", "8080");
    env.set("APP_NAME", "demo");
    env.set("OTHER_VAR", "xyz");

    // Indirect
    assert_eq!(
        expand_str("${!PTR}", &mut env, &config).unwrap(),
        "FINAL_VALUE"
    );

    // Prefix listing
    let star = expand_str("${!APP_*}", &mut env, &config).unwrap();
    assert_eq!(star, "APP_HOST APP_NAME APP_PORT");

    let at = expand_str("${!APP_@}", &mut env, &config).unwrap();
    assert_eq!(at, "APP_HOST APP_NAME APP_PORT");
}

#[test]
fn test_transformations() {
    let config = ModeConfig::default();
    let mut env = EnvMap::new();
    env.set("v", "hello world");
    env.set("v_quote", "it's \"fine\"");
    env.set("v_esc", "line1\\nline2");

    assert_eq!(
        expand_str("${v@U}", &mut env, &config).unwrap(),
        "HELLO WORLD"
    );
    assert_eq!(
        expand_str("${v@u}", &mut env, &config).unwrap(),
        "Hello world"
    );
    assert_eq!(
        expand_str("${v@L}", &mut env, &config).unwrap(),
        "hello world"
    );
    assert_eq!(
        expand_str("${v_quote@Q}", &mut env, &config).unwrap(),
        "'it'\\''s \"fine\"'"
    );
    assert_eq!(
        expand_str("${v@A}", &mut env, &config).unwrap(),
        "v='hello world'"
    );
    assert_eq!(
        expand_str("${v_esc@E}", &mut env, &config).unwrap(),
        "line1\nline2"
    );

    // Reject @P and @K
    assert!(expand_str("${v@P}", &mut env, &config).is_err());
    assert!(expand_str("${v@K}", &mut env, &config).is_err());
}

#[test]
fn test_recursive_nested_expansion_and_quotes() {
    let config = ModeConfig::default();
    let mut env = EnvMap::new();
    env.set("C", "final");

    assert_eq!(
        expand_str("${A:-${B:-$C}}", &mut env, &config).unwrap(),
        "final"
    );

    env.set("AZURE_TENANT_ID", "tenant-1234");
    assert_eq!(
        expand_str("${BC_TENANT_ID:-\"$AZURE_TENANT_ID\"}", &mut env, &config).unwrap(),
        "tenant-1234"
    );
}
