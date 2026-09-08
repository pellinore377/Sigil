use crate::Error;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Number(String);
impl Number {
    pub fn new(value: f64) -> Result<Self, Error> {
        if !value.is_finite() {
            return Err(Error::Invalid);
        }
        Ok(Self(if value == 0.0 {
            "0".into()
        } else {
            value.to_string()
        }))
    }
    pub fn value(&self) -> f64 {
        self.0.parse().expect("validated finite number")
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn fixed(&self, precision: usize) -> Result<String, Error> {
        if precision > 6 {
            return Err(Error::Limit);
        }
        let value = format!("{:.*}", precision, self.value());
        let value = if value.contains('.') {
            value.trim_end_matches('0').trim_end_matches('.')
        } else {
            &value
        };
        Ok(if value == "-0" {
            "0".into()
        } else {
            value.into()
        })
    }
}
impl TryFrom<String> for Number {
    type Error = Error;
    fn try_from(value: String) -> Result<Self, Error> {
        if value.len() > 384 || value.trim() != value || value.is_empty() {
            return Err(Error::Invalid);
        }
        Self::new(value.parse().map_err(|_| Error::Invalid)?)
    }
}
impl From<Number> for String {
    fn from(value: Number) -> String {
        value.0
    }
}

/// Finite arithmetic only; bounded token count, recursion and integer powers.
pub fn calculate(expression: &str) -> Result<Number, Error> {
    if expression.len() > 1024 {
        return Err(Error::Limit);
    }
    struct Parser<'a> {
        text: &'a [u8],
        at: usize,
        steps: usize,
    }
    impl Parser<'_> {
        fn space(&mut self) {
            while self.text.get(self.at).is_some_and(u8::is_ascii_whitespace) {
                self.at += 1;
            }
        }
        fn expr(&mut self, min: u8, depth: u8) -> Result<f64, Error> {
            if depth > 32 || self.steps >= 256 {
                return Err(Error::Limit);
            }
            self.steps += 1;
            self.space();
            let first = *self.text.get(self.at).ok_or(Error::Invalid)?;
            let mut left = match first {
                b'+' | b'-' => {
                    self.at += 1;
                    let value = self.expr(5, depth + 1)?;
                    if first == b'-' {
                        -value
                    } else {
                        value
                    }
                }
                b'(' => {
                    self.at += 1;
                    let v = self.expr(0, depth + 1)?;
                    self.space();
                    if self.text.get(self.at) != Some(&b')') {
                        return Err(Error::Invalid);
                    }
                    self.at += 1;
                    v
                }
                _ => {
                    let start = self.at;
                    while self
                        .text
                        .get(self.at)
                        .is_some_and(|b| b.is_ascii_digit() || *b == b'.')
                    {
                        self.at += 1;
                    }
                    if self
                        .text
                        .get(self.at)
                        .is_some_and(|b| *b == b'e' || *b == b'E')
                    {
                        self.at += 1;
                        if self
                            .text
                            .get(self.at)
                            .is_some_and(|b| *b == b'+' || *b == b'-')
                        {
                            self.at += 1;
                        }
                        while self.text.get(self.at).is_some_and(u8::is_ascii_digit) {
                            self.at += 1;
                        }
                    }
                    std::str::from_utf8(&self.text[start..self.at])
                        .map_err(|_| Error::Invalid)?
                        .parse::<f64>()
                        .map_err(|_| Error::Invalid)?
                }
            };
            loop {
                self.space();
                let Some(&op) = self.text.get(self.at) else {
                    break;
                };
                let (l, r) = match op {
                    b'+' | b'-' => (1, 2),
                    b'*' | b'/' | b'%' => (3, 4),
                    b'^' => (6, 5),
                    _ => break,
                };
                if l < min {
                    break;
                }
                self.at += 1;
                let right = self.expr(r, depth + 1)?;
                left = match op {
                    b'+' => left + right,
                    b'-' => left - right,
                    b'*' => left * right,
                    b'/' => left / right,
                    b'%' => left % right,
                    b'^' => {
                        if right.fract() != 0.0 || right.abs() > 64.0 {
                            return Err(Error::Invalid);
                        }
                        let mut n = right.abs() as u32;
                        let mut base = left;
                        let mut result = 1.0;
                        while n > 0 {
                            if n & 1 != 0 {
                                result *= base;
                            }
                            n >>= 1;
                            if n > 0 {
                                base *= base;
                            }
                        }
                        if right < 0.0 {
                            1.0 / result
                        } else {
                            result
                        }
                    }
                    _ => return Err(Error::Invalid),
                };
                if !left.is_finite() {
                    return Err(Error::Invalid);
                }
            }
            if !left.is_finite() {
                return Err(Error::Invalid);
            }
            Ok(left)
        }
    }
    let mut parser = Parser {
        text: expression.as_bytes(),
        at: 0,
        steps: 0,
    };
    let value = parser.expr(0, 0)?;
    parser.space();
    if parser.at != expression.len() {
        return Err(Error::Invalid);
    }
    Number::new(value)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Conversion {
    pub input: Number,
    pub from: String,
    pub output: Number,
    pub to: String,
}
impl Conversion {
    pub fn parse(source: &str) -> Result<Self, Error> {
        if source.len() > 512 {
            return Err(Error::Limit);
        }
        let source = source.trim();
        let n = source
            .bytes()
            .take_while(|b| b.is_ascii_digit() || matches!(b, b'+' | b'-' | b'.' | b'e' | b'E'))
            .count();
        let input = Number::try_from(source[..n].to_owned())?;
        let unit = source[n..].trim().to_ascii_lowercase();
        let (from, to, factor, offset) = match unit.as_str() {
            "c" | "°c" | "celsius" => ("°C", "°F", 1.8, 32.0),
            "f" | "°f" | "fahrenheit" => ("°F", "°C", 1.0 / 1.8, -32.0 / 1.8),
            "m" | "meter" | "meters" => ("m", "ft", 1.0 / 0.3048, 0.0),
            "ft" | "foot" | "feet" => ("ft", "m", 0.3048, 0.0),
            "cm" => ("cm", "in", 1.0 / 2.54, 0.0),
            "in" | "inch" | "inches" => ("in", "cm", 2.54, 0.0),
            "km" | "kilometers" => ("km", "mi", 1.0 / 1.609344, 0.0),
            "mi" | "mile" | "miles" => ("mi", "km", 1.609344, 0.0),
            "kg" | "kilograms" => ("kg", "lb", 1.0 / 0.45359237, 0.0),
            "lb" | "lbs" | "pound" | "pounds" => ("lb", "kg", 0.45359237, 0.0),
            "g" | "grams" => ("g", "oz", 1.0 / 28.349523125, 0.0),
            "oz" | "ounces" => ("oz", "g", 28.349523125, 0.0),
            "l" | "liter" | "liters" => ("L", "US gal", 1.0 / 3.785411784, 0.0),
            "gal" | "gallon" | "gallons" | "us gal" => ("US gal", "L", 3.785411784, 0.0),
            "ml" => ("mL", "US fl oz", 1.0 / 29.5735295625, 0.0),
            "fl oz" | "us fl oz" => ("US fl oz", "mL", 29.5735295625, 0.0),
            "km/h" | "kph" => ("km/h", "mph", 1.0 / 1.609344, 0.0),
            "mph" => ("mph", "km/h", 1.609344, 0.0),
            "m/s" => ("m/s", "ft/s", 1.0 / 0.3048, 0.0),
            "ft/s" => ("ft/s", "m/s", 0.3048, 0.0),
            _ => return Err(Error::Invalid),
        };
        let output = Number::new(input.value() * factor + offset)?;
        Ok(Self {
            input,
            from: from.into(),
            output,
            to: to.into(),
        })
    }
    pub fn validate(&self) -> Result<(), Error> {
        if Self::parse(&format!("{} {}", self.input.as_str(), self.from))? != *self {
            return Err(Error::Invalid);
        }
        Ok(())
    }
    pub fn body(&self) -> Result<String, Error> {
        self.validate()?;
        Ok(format!(
            "{} {} = {} {}",
            self.input.as_str(),
            self.from,
            self.output.fixed(2)?,
            self.to
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arithmetic_is_bounded_finite_and_has_no_execution_facilities() {
        for (expression, result) in [
            ("17 * 34", "578"),
            ("2 + 3 * (4 - 1)", "11"),
            ("2^3^2", "512"),
            ("-2^2", "-4"),
            ("1e3 / 2", "500"),
        ] {
            assert_eq!(calculate(expression).unwrap().as_str(), result);
        }
        for bad in [
            "1/0",
            "NaN",
            "1e309",
            "exec(1)",
            "2;3",
            "2^0.5",
            "999999^64",
            "(1+2",
            "1+",
            "",
        ] {
            assert!(calculate(bad).is_err(), "{bad}");
        }
        assert!(calculate(&format!("{}1{}", "(".repeat(40), ")".repeat(40))).is_err());
        assert_eq!(
            Conversion::parse("20C").unwrap().body().unwrap(),
            "20 °C = 68 °F"
        );
        assert_eq!(
            Conversion::parse("5 miles").unwrap().body().unwrap(),
            "5 mi = 8.05 km"
        );
        assert_eq!(
            Conversion::parse("1 lb").unwrap().output.as_str(),
            "0.45359237"
        );
        assert!(Conversion::parse("3 unknown").is_err());
    }
}
