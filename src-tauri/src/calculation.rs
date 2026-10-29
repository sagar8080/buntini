//! Bounded arithmetic over user/model-supplied expressions. This parser has no
//! names, function calls, evaluation runtime, I/O, external services or effects.
use serde_json::{json, Value};

pub const MAX_EXPRESSIONS: usize = 8;
pub const MAX_EXPRESSION_BYTES: usize = 256;
const MAX_DEPTH: usize = 32;
const MAX_OPERATIONS: usize = 128;
const MAX_LITERAL_BYTES: usize = 64;
const MAX_MAGNITUDE: f64 = 1e100;
const MIN_MAGNITUDE: f64 = 1e-100;

pub fn validate(arguments: &Value) -> Result<Vec<String>, String> {
    let fields = arguments
        .as_object()
        .ok_or("Calculate arguments must be an object.")?;
    if fields.len() != 1 || !fields.contains_key("expressions") {
        return Err("Calculate accepts only an expressions array.".into());
    }
    let expressions = fields["expressions"]
        .as_array()
        .ok_or("Expressions must be a string array.")?;
    if expressions.is_empty() || expressions.len() > MAX_EXPRESSIONS {
        return Err("Calculate accepts 1–8 expressions per call.".into());
    }
    expressions
        .iter()
        .map(|expression| {
            let expression = expression.as_str().ok_or("Each expression must be text.")?;
            validate_text(expression)?;
            Ok(expression.to_string())
        })
        .collect()
}

fn validate_text(expression: &str) -> Result<(), String> {
    if expression.trim().is_empty()
        || expression.len() > MAX_EXPRESSION_BYTES
        || !expression.is_ascii()
    {
        return Err("Each calculation must contain 1–256 ASCII bytes.".into());
    }
    Ok(())
}

fn bounded(value: f64) -> Result<f64, String> {
    if !value.is_finite()
        || value.abs() > MAX_MAGNITUDE
        || (value != 0.0 && value.abs() < MIN_MAGNITUDE)
    {
        Err("Calculation exceeds the supported finite magnitude range (1e-100 to 1e100).".into())
    } else {
        Ok(if value == 0.0 { 0.0 } else { value })
    }
}

struct Parser<'a> {
    input: &'a [u8],
    at: usize,
    operations: usize,
}
impl Parser<'_> {
    fn whitespace(&mut self) {
        while self.input.get(self.at).is_some_and(u8::is_ascii_whitespace) {
            self.at += 1;
        }
    }
    fn take(&mut self, byte: u8) -> bool {
        self.whitespace();
        if self.input.get(self.at) == Some(&byte) {
            self.at += 1;
            true
        } else {
            false
        }
    }
    fn operation(&mut self) -> Result<(), String> {
        self.operations += 1;
        if self.operations > MAX_OPERATIONS {
            Err("Calculation exceeds its operation limit.".into())
        } else {
            Ok(())
        }
    }
    fn sum(&mut self, depth: usize) -> Result<f64, String> {
        let mut value = self.product(depth)?;
        loop {
            let subtract = if self.take(b'+') {
                false
            } else if self.take(b'-') {
                true
            } else {
                break;
            };
            self.operation()?;
            let right = self.product(depth)?;
            value = bounded(if subtract {
                value - right
            } else {
                value + right
            })?;
        }
        Ok(value)
    }
    fn product(&mut self, depth: usize) -> Result<f64, String> {
        let mut value = self.unary(depth)?;
        loop {
            let operator = if self.take(b'*') {
                b'*'
            } else if self.take(b'/') {
                b'/'
            } else if self.take(b'%') {
                b'%'
            } else {
                break;
            };
            self.operation()?;
            let right = self.unary(depth)?;
            if right == 0.0 && operator != b'*' {
                return Err("Division or remainder by zero is not allowed.".into());
            }
            value = bounded(match operator {
                b'*' => value * right,
                b'/' => value / right,
                _ => value % right,
            })?;
        }
        Ok(value)
    }
    fn unary(&mut self, depth: usize) -> Result<f64, String> {
        let mut negative = false;
        loop {
            if self.take(b'+') {
                self.operation()?;
            } else if self.take(b'-') {
                self.operation()?;
                negative = !negative;
            } else {
                break;
            }
        }
        let value = self.primary(depth)?;
        bounded(if negative { -value } else { value })
    }
    fn primary(&mut self, depth: usize) -> Result<f64, String> {
        if self.take(b'(') {
            if depth >= MAX_DEPTH {
                return Err("Calculation exceeds its parenthesis depth limit.".into());
            }
            let value = self.sum(depth + 1)?;
            if !self.take(b')') {
                return Err("Calculation has an unmatched parenthesis.".into());
            }
            return Ok(value);
        }
        self.whitespace();
        // Dollar-prefixed numeric literals are common in copied bill amounts.
        // This consumes one prefix only; it never strips arbitrary source text.
        if self.input.get(self.at) == Some(&b'$') {
            self.at += 1;
        }
        let start = self.at;
        while self.input.get(self.at).is_some_and(u8::is_ascii_digit) {
            self.at += 1;
        }
        let mut digits = self.at - start;
        if self.input.get(self.at) == Some(&b'.') {
            self.at += 1;
            let fraction = self.at;
            while self.input.get(self.at).is_some_and(u8::is_ascii_digit) {
                self.at += 1;
            }
            digits += self.at - fraction;
        }
        if digits == 0 {
            return Err(
                "Calculation requires a numeric literal or parenthesized expression.".into(),
            );
        }
        let mantissa_end = self.at;
        if matches!(self.input.get(self.at), Some(b'e' | b'E')) {
            self.at += 1;
            if matches!(self.input.get(self.at), Some(b'+' | b'-')) {
                self.at += 1;
            }
            let exponent = self.at;
            while self.input.get(self.at).is_some_and(u8::is_ascii_digit) {
                self.at += 1;
            }
            if exponent == self.at {
                return Err("Calculation has an invalid numeric exponent.".into());
            }
        }
        if self.at - start > MAX_LITERAL_BYTES {
            return Err("Calculation contains an oversized numeric literal.".into());
        }
        let literal = std::str::from_utf8(&self.input[start..self.at]).unwrap();
        let value = literal
            .parse::<f64>()
            .map_err(|_| "Calculation contains an invalid number.".to_string())?;
        if value == 0.0
            && self.input[start..mantissa_end]
                .iter()
                .any(|digit| matches!(digit, b'1'..=b'9'))
        {
            return Err("Calculation numeric literal underflowed; use a larger magnitude.".into());
        }
        bounded(value)
    }
}

pub fn evaluate(expression: &str) -> Result<f64, String> {
    validate_text(expression)?;
    let mut parser = Parser {
        input: expression.as_bytes(),
        at: 0,
        operations: 0,
    };
    let result = parser.sum(0)?;
    parser.whitespace();
    if parser.at != parser.input.len() {
        return Err("Calculation contains unsupported syntax. Use only numbers, parentheses and + - * / % operators.".into());
    }
    bounded(result)
}

fn formatted(value: f64) -> String {
    if value == 0.0 {
        return "0".into();
    }
    if value.abs() >= 1e15 || value.abs() < 1e-6 {
        let scientific = format!("{value:.14e}");
        let (mantissa, exponent) = scientific.split_once('e').unwrap();
        format!(
            "{}e{}",
            mantissa.trim_end_matches('0').trim_end_matches('.'),
            exponent
        )
    } else {
        format!("{value:.12}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string()
    }
}

pub fn run(arguments: &Value) -> Result<Value, String> {
    let expressions = validate(arguments)?;
    let results = expressions
        .into_iter()
        .map(|expression| {
            let result = evaluate(&expression)?;
            Ok(json!({"expression":expression,"result":result,"formattedValue":formatted(result)}))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(json!({"kind":"calculation_result","results":results,
        "precision":"IEEE 754 finite arithmetic; formatted values round display-only noise. % is binary remainder, not percentage."}))
}

/// Recompute from the approved read arguments, rather than accepting a model-
/// supplied total or a shape-only success claim as calculation evidence.
pub fn verify(arguments: &Value, evidence: &Value) -> bool {
    run(arguments).is_ok_and(|expected| expected == *evidence)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arithmetic_precedence_parentheses_decimal_and_unary_signs() {
        for (expression, expected) in [
            ("18+15", 33.0),
            ("240+84/4", 261.0),
            ("(240+84)/4+16/2", 89.0),
            (".5 * (4 - -2)", 3.0),
            ("-(-3)+2*4", 11.0),
            ("7%4", 3.0),
            ("1e2/4", 25.0),
        ] {
            assert_eq!(evaluate(expression).unwrap(), expected, "{expression}");
        }
        let value = run(&json!({"expressions":["0.1+0.2","1/8"]})).unwrap();
        assert_eq!(value["results"][0]["formattedValue"], "0.3");
        assert_eq!(value["results"][1]["formattedValue"], "0.125");
    }
    #[test]
    fn hostile_syntax_missing_operands_and_zero_divisors_are_rejected() {
        for expression in [
            "1/0",
            "1%-0",
            "system('rm -rf /')",
            "__import__('os')",
            "$(id)",
            "3$4",
            "$$4",
            "$(1+2)",
            "1;2",
            "2**3",
            "2^3",
            "sqrt(9)",
            "NaN",
            "inf",
            "1e999",
            "1e-999",
            "2(3)",
            "(1+2",
            "1+",
            "1..2",
            ".",
            "1e+",
            "10000000000000000000000000000000000000000000000000000000000000000",
            "2×3",
        ] {
            assert!(evaluate(expression).is_err(), "{expression}");
        }
    }
    #[test]
    fn copied_dollar_amounts_are_numeric_literals_only() {
        assert_eq!(evaluate("$32/4 + $7").unwrap(),15.0);
        assert_eq!(evaluate("-$8 + 20").unwrap(),12.0);
    }
    #[test]
    fn call_shape_and_all_resource_limits_are_enforced() {
        for arguments in [
            json!({}),
            json!({"expressions":[]}),
            json!({"expressions":["1"],"command":"bad"}),
            json!({"expressions":[false]}),
            json!({"expressions":vec!["1";9]}),
        ] {
            assert!(run(&arguments).is_err(), "{arguments}");
        }
        assert!(evaluate(&" ".repeat(257)).is_err());
        assert!(evaluate(&format!("{}1{}", "(".repeat(33), ")".repeat(33))).is_err());
        assert!(evaluate(&format!("{}1", "-".repeat(129))).is_err());
        assert!(evaluate("1e100*10").is_err());
        assert!(evaluate("1e-100/10").is_err());
        assert!(evaluate(&format!("{}1{}", "(".repeat(32), ")".repeat(32))).is_ok());
    }
    #[test]
    fn receipts_are_bound_to_actual_expressions_and_computed_values() {
        let args = json!({"expressions":["18+15","(240+84)/4"]});
        let mut evidence = run(&args).unwrap();
        assert!(verify(&args, &evidence));
        evidence["results"][0]["result"] = 36.into();
        assert!(!verify(&args, &evidence));
        let mut evidence = run(&args).unwrap();
        evidence["results"][0]["expression"] = "18+18".into();
        assert!(!verify(&args, &evidence));
        assert!(
            run(&json!({"expressions":["18+15","1/0"]})).is_err(),
            "an invalid batch cannot publish a partial success receipt"
        );
    }
}
