//! Property-based differential tests with `proptest` comparing against GNU Bash.

use proptest::prelude::*;
use std::process::Command;
use xesubst::ModeConfig;
use xesubst::env::{EnvMap, VariableTable};
use xesubst::eval::expand_str;

fn eval_bash(val: &str, expr_fmt: &str) -> Option<String> {
    // Avoid characters that cause bash syntax errors in the bash test driver itself,
    // and use printf to avoid values like -n/-e/-E being parsed as echo flags.
    let bash_cmd = format!("V={val:?}; printf \"%s\" {expr_fmt}");
    let output = Command::new("bash")
        .arg("-c")
        .arg(&bash_cmd)
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        None
    }
}

fn eval_xes(val: &str, expr: &str) -> String {
    let mut env = EnvMap::new();
    env.set("V", val);
    let config = ModeConfig::default();
    expand_str(expr, &mut env, &config).unwrap()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]

    #[test]
    fn prop_prefix_strip_shortest(
        val in "[a-zA-Z0-9_/.-]{1,20}",
        sub in "[a-zA-Z0-9]{1,4}"
    ) {
        let pattern = format!("*{sub}");
        let expr = format!("${{V#{pattern}}}");
        let bash_expr = format!("\"${{V#{pattern}}}\"");
        if let Some(bash_out) = eval_bash(&val, &bash_expr) {
            let xes_out = eval_xes(&val, &expr);
            prop_assert_eq!(xes_out, bash_out);
        }
    }

    #[test]
    fn prop_prefix_strip_longest(
        val in "[a-zA-Z0-9_/.-]{1,20}",
        sub in "[a-zA-Z0-9]{1,4}"
    ) {
        let pattern = format!("*{sub}");
        let expr = format!("${{V##{pattern}}}");
        let bash_expr = format!("\"${{V##{pattern}}}\"");
        if let Some(bash_out) = eval_bash(&val, &bash_expr) {
            let xes_out = eval_xes(&val, &expr);
            prop_assert_eq!(xes_out, bash_out);
        }
    }

    #[test]
    fn prop_suffix_strip_shortest(
        val in "[a-zA-Z0-9_/.-]{1,20}",
        sub in "[a-zA-Z0-9]{1,4}"
    ) {
        let pattern = format!("{sub}*");
        let expr = format!("${{V%{pattern}}}");
        let bash_expr = format!("\"${{V%{pattern}}}\"");
        if let Some(bash_out) = eval_bash(&val, &bash_expr) {
            let xes_out = eval_xes(&val, &expr);
            prop_assert_eq!(xes_out, bash_out);
        }
    }

    #[test]
    fn prop_suffix_strip_longest(
        val in "[a-zA-Z0-9_/.-]{1,20}",
        sub in "[a-zA-Z0-9]{1,4}"
    ) {
        let pattern = format!("{sub}*");
        let expr = format!("${{V%%{pattern}}}");
        let bash_expr = format!("\"${{V%%{pattern}}}\"");
        if let Some(bash_out) = eval_bash(&val, &bash_expr) {
            let xes_out = eval_xes(&val, &expr);
            prop_assert_eq!(xes_out, bash_out);
        }
    }

    #[test]
    fn prop_replace_first(
        val in "[a-zA-Z0-9_-]{1,20}",
        pat in "[a-zA-Z0-9]{1,3}",
        rep in "[a-zA-Z0-9]{0,3}"
    ) {
        let expr = format!("${{V/{pat}/{rep}}}");
        let bash_expr = format!("\"${{V/{pat}/{rep}}}\"");
        if let Some(bash_out) = eval_bash(&val, &bash_expr) {
            let xes_out = eval_xes(&val, &expr);
            prop_assert_eq!(xes_out, bash_out);
        }
    }

    #[test]
    fn prop_replace_all(
        val in "[a-zA-Z0-9_-]{1,20}",
        pat in "[a-zA-Z0-9]{1,3}",
        rep in "[a-zA-Z0-9]{0,3}"
    ) {
        let expr = format!("${{V//{pat}/{rep}}}");
        let bash_expr = format!("\"${{V//{pat}/{rep}}}\"");
        if let Some(bash_out) = eval_bash(&val, &bash_expr) {
            let xes_out = eval_xes(&val, &expr);
            prop_assert_eq!(xes_out, bash_out);
        }
    }
}
