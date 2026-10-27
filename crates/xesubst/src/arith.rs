//! Bash integer arithmetic evaluator.

use crate::env::VariableTable;
use crate::error::XesubstError;
use crate::span::Span;

/// Evaluates a Bash arithmetic string expression.
pub fn eval_arithmetic(
    expr: &str,
    env: &dyn VariableTable,
    span: Span,
) -> Result<i64, XesubstError> {
    let mut parser = ArithParser::new(expr, env, span);
    let val = parser.parse_expression(0)?;
    parser.skip_whitespace();
    if parser.pos < parser.input.len() {
        return Err(XesubstError::Arithmetic {
            span,
            message: format!(
                "unexpected character in arithmetic expression: '{}'",
                &parser.input[parser.pos..]
            ),
        });
    }
    Ok(val)
}

struct ArithParser<'a> {
    input: &'a str,
    chars: Vec<char>,
    pos: usize,
    env: &'a dyn VariableTable,
    span: Span,
}

impl<'a> ArithParser<'a> {
    fn new(input: &'a str, env: &'a dyn VariableTable, span: Span) -> Self {
        Self {
            input,
            chars: input.chars().collect(),
            pos: 0,
            env,
            span,
        }
    }

    fn skip_whitespace(&mut self) {
        while self.pos < self.chars.len() && self.chars[self.pos].is_whitespace() {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn next(&mut self) -> Option<char> {
        let c = self.chars.get(self.pos).copied();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn parse_expression(&mut self, min_prec: u8) -> Result<i64, XesubstError> {
        self.skip_whitespace();
        let mut lhs = self.parse_unary()?;

        loop {
            self.skip_whitespace();
            let op_info = match self.peek_binary_op() {
                Some(info) if info.prec >= min_prec => info,
                _ => break,
            };

            self.pos += op_info.op_len;

            // Handle ternary operator: condition ? true_expr : false_expr
            if op_info.is_ternary {
                let true_val = self.parse_expression(0)?;
                self.skip_whitespace();
                if self.next() != Some(':') {
                    return Err(XesubstError::Arithmetic {
                        span: self.span,
                        message: "expected ':' in ternary arithmetic expression".to_string(),
                    });
                }
                let false_val = self.parse_expression(op_info.prec)?;
                lhs = if lhs != 0 { true_val } else { false_val };
                continue;
            }

            // Normal binary operator
            let next_min_prec = op_info.prec + 1;
            let rhs = self.parse_expression(next_min_prec)?;

            lhs = match op_info.op_str {
                "+" => lhs.wrapping_add(rhs),
                "-" => lhs.wrapping_sub(rhs),
                "*" => lhs.wrapping_mul(rhs),
                "/" => {
                    if rhs == 0 {
                        return Err(XesubstError::Arithmetic {
                            span: self.span,
                            message: "division by 0 in arithmetic expression".to_string(),
                        });
                    }
                    lhs.wrapping_div(rhs)
                }
                "%" => {
                    if rhs == 0 {
                        return Err(XesubstError::Arithmetic {
                            span: self.span,
                            message: "division by 0 in arithmetic expression".to_string(),
                        });
                    }
                    lhs.wrapping_rem(rhs)
                }
                "<<" => lhs.wrapping_shl(rhs as u32),
                ">>" => lhs.wrapping_shr(rhs as u32),
                "<" => (lhs < rhs) as i64,
                "<=" => (lhs <= rhs) as i64,
                ">" => (lhs > rhs) as i64,
                ">=" => (lhs >= rhs) as i64,
                "==" => (lhs == rhs) as i64,
                "!=" => (lhs != rhs) as i64,
                "&" => lhs & rhs,
                "^" => lhs ^ rhs,
                "|" => lhs | rhs,
                "&&" => ((lhs != 0) && (rhs != 0)) as i64,
                "||" => ((lhs != 0) || (rhs != 0)) as i64,
                _ => {
                    return Err(XesubstError::Arithmetic {
                        span: self.span,
                        message: format!("unsupported operator '{}'", op_info.op_str),
                    });
                }
            };
        }

        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<i64, XesubstError> {
        self.skip_whitespace();
        match self.peek() {
            Some('+') => {
                self.pos += 1;
                self.parse_unary()
            }
            Some('-') => {
                self.pos += 1;
                let val = self.parse_unary()?;
                Ok(val.wrapping_neg())
            }
            Some('~') => {
                self.pos += 1;
                let val = self.parse_unary()?;
                Ok(!val)
            }
            Some('!') => {
                self.pos += 1;
                let val = self.parse_unary()?;
                Ok((val == 0) as i64)
            }
            Some('(') => {
                self.pos += 1;
                let val = self.parse_expression(0)?;
                self.skip_whitespace();
                if self.next() != Some(')') {
                    return Err(XesubstError::Arithmetic {
                        span: self.span,
                        message: "unclosed '(' in arithmetic expression".to_string(),
                    });
                }
                Ok(val)
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Result<i64, XesubstError> {
        self.skip_whitespace();
        if self.pos >= self.chars.len() {
            return Err(XesubstError::Arithmetic {
                span: self.span,
                message: "unexpected end of arithmetic expression".to_string(),
            });
        }

        let c = self.chars[self.pos];
        if c.is_ascii_digit() {
            self.parse_number()
        } else if c.is_ascii_alphabetic() || c == '_' {
            self.parse_variable()
        } else {
            Err(XesubstError::Arithmetic {
                span: self.span,
                message: format!("unexpected character in arithmetic expression: '{c}'"),
            })
        }
    }

    fn parse_number(&mut self) -> Result<i64, XesubstError> {
        let start = self.pos;
        while self.pos < self.chars.len()
            && (self.chars[self.pos].is_ascii_alphanumeric() || self.chars[self.pos] == '#')
        {
            self.pos += 1;
        }
        let token: String = self.chars[start..self.pos].iter().collect();

        // Check for base#number format (e.g. 2#1010)
        if let Some((base_str, val_str)) = token.split_once('#') {
            let base: u32 = base_str.parse().map_err(|_| XesubstError::Arithmetic {
                span: self.span,
                message: format!("invalid base in '{token}'"),
            })?;
            if !(2..=36).contains(&base) {
                return Err(XesubstError::Arithmetic {
                    span: self.span,
                    message: format!("base out of range 2..36 in '{token}'"),
                });
            }
            return i64::from_str_radix(val_str, base).map_err(|_| XesubstError::Arithmetic {
                span: self.span,
                message: format!("invalid digit for base {base} in '{token}'"),
            });
        }

        // Check for hex 0x...
        if (token.starts_with("0x") || token.starts_with("0X")) && token.len() > 2 {
            return i64::from_str_radix(&token[2..], 16).map_err(|_| XesubstError::Arithmetic {
                span: self.span,
                message: format!("invalid hex number '{token}'"),
            });
        }

        // Check for octal 0...
        if token.starts_with('0') && token.len() > 1 && token.chars().all(|c| c.is_ascii_digit()) {
            return i64::from_str_radix(&token, 8).map_err(|_| XesubstError::Arithmetic {
                span: self.span,
                message: format!("invalid octal number '{token}'"),
            });
        }

        // Decimal
        token.parse::<i64>().map_err(|_| XesubstError::Arithmetic {
            span: self.span,
            message: format!("invalid integer '{token}'"),
        })
    }

    fn parse_variable(&mut self) -> Result<i64, XesubstError> {
        let start = self.pos;
        while self.pos < self.chars.len()
            && (self.chars[self.pos].is_ascii_alphanumeric() || self.chars[self.pos] == '_')
        {
            self.pos += 1;
        }
        let name: String = self.chars[start..self.pos].iter().collect();
        let val_str = self.env.get(&name).unwrap_or("");
        if val_str.trim().is_empty() {
            Ok(0)
        } else if let Ok(val) = val_str.trim().parse::<i64>() {
            Ok(val)
        } else {
            // Recursively evaluate variable value as arithmetic expression
            eval_arithmetic(val_str.trim(), self.env, self.span)
        }
    }

    fn peek_binary_op(&self) -> Option<BinaryOpInfo> {
        let remaining: String = self.chars[self.pos..].iter().collect();

        // 2-character ops
        if remaining.starts_with("||") {
            return Some(BinaryOpInfo {
                op_str: "||",
                prec: 1,
                op_len: 2,
                is_ternary: false,
            });
        }
        if remaining.starts_with("&&") {
            return Some(BinaryOpInfo {
                op_str: "&&",
                prec: 2,
                op_len: 2,
                is_ternary: false,
            });
        }
        if remaining.starts_with("==") {
            return Some(BinaryOpInfo {
                op_str: "==",
                prec: 6,
                op_len: 2,
                is_ternary: false,
            });
        }
        if remaining.starts_with("!=") {
            return Some(BinaryOpInfo {
                op_str: "!=",
                prec: 6,
                op_len: 2,
                is_ternary: false,
            });
        }
        if remaining.starts_with("<=") {
            return Some(BinaryOpInfo {
                op_str: "<=",
                prec: 7,
                op_len: 2,
                is_ternary: false,
            });
        }
        if remaining.starts_with(">=") {
            return Some(BinaryOpInfo {
                op_str: ">=",
                prec: 7,
                op_len: 2,
                is_ternary: false,
            });
        }
        if remaining.starts_with("<<") {
            return Some(BinaryOpInfo {
                op_str: "<<",
                prec: 8,
                op_len: 2,
                is_ternary: false,
            });
        }
        if remaining.starts_with(">>") {
            return Some(BinaryOpInfo {
                op_str: ">>",
                prec: 8,
                op_len: 2,
                is_ternary: false,
            });
        }

        // 1-character ops
        if remaining.starts_with('?') {
            return Some(BinaryOpInfo {
                op_str: "?",
                prec: 0,
                op_len: 1,
                is_ternary: true,
            });
        }
        if remaining.starts_with('|') {
            return Some(BinaryOpInfo {
                op_str: "|",
                prec: 3,
                op_len: 1,
                is_ternary: false,
            });
        }
        if remaining.starts_with('^') {
            return Some(BinaryOpInfo {
                op_str: "^",
                prec: 4,
                op_len: 1,
                is_ternary: false,
            });
        }
        if remaining.starts_with('&') {
            return Some(BinaryOpInfo {
                op_str: "&",
                prec: 5,
                op_len: 1,
                is_ternary: false,
            });
        }
        if remaining.starts_with('<') {
            return Some(BinaryOpInfo {
                op_str: "<",
                prec: 7,
                op_len: 1,
                is_ternary: false,
            });
        }
        if remaining.starts_with('>') {
            return Some(BinaryOpInfo {
                op_str: ">",
                prec: 7,
                op_len: 1,
                is_ternary: false,
            });
        }
        if remaining.starts_with('+') {
            return Some(BinaryOpInfo {
                op_str: "+",
                prec: 9,
                op_len: 1,
                is_ternary: false,
            });
        }
        if remaining.starts_with('-') {
            return Some(BinaryOpInfo {
                op_str: "-",
                prec: 9,
                op_len: 1,
                is_ternary: false,
            });
        }
        if remaining.starts_with('*') {
            return Some(BinaryOpInfo {
                op_str: "*",
                prec: 10,
                op_len: 1,
                is_ternary: false,
            });
        }
        if remaining.starts_with('/') {
            return Some(BinaryOpInfo {
                op_str: "/",
                prec: 10,
                op_len: 1,
                is_ternary: false,
            });
        }
        if remaining.starts_with('%') {
            return Some(BinaryOpInfo {
                op_str: "%",
                prec: 10,
                op_len: 1,
                is_ternary: false,
            });
        }

        None
    }
}

struct BinaryOpInfo {
    op_str: &'static str,
    prec: u8,
    op_len: usize,
    is_ternary: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::EnvMap;

    #[test]
    fn test_basic_arithmetic() {
        let env = EnvMap::new();
        let span = Span::default();

        assert_eq!(eval_arithmetic("1 + 2 * 3", &env, span).unwrap(), 7);
        assert_eq!(eval_arithmetic("(1 + 2) * 3", &env, span).unwrap(), 9);
        assert_eq!(eval_arithmetic("10 - -5", &env, span).unwrap(), 15);
        assert_eq!(eval_arithmetic("10 % 3", &env, span).unwrap(), 1);
        assert_eq!(eval_arithmetic("1 << 3", &env, span).unwrap(), 8);
        assert_eq!(
            eval_arithmetic("4 > 2 ? 100 : 200", &env, span).unwrap(),
            100
        );
        assert_eq!(eval_arithmetic("0 ? 100 : 200", &env, span).unwrap(), 200);
    }

    #[test]
    fn test_variables_and_bases() {
        let mut env = EnvMap::new();
        env.set("X", "5");
        env.set("Y", "10");
        let span = Span::default();

        assert_eq!(eval_arithmetic("X * Y", &env, span).unwrap(), 50);
        assert_eq!(eval_arithmetic("0x10 + 010", &env, span).unwrap(), 16 + 8);
        assert_eq!(eval_arithmetic("2#101", &env, span).unwrap(), 5);
    }
}
