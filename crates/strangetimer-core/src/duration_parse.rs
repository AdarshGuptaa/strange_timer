use chrono::Duration;

/// Parse a time-offset string into a `chrono::Duration`.
///
/// Supports both single-unit forms (`30s`, `5m`, `5min`, `2h`, `1D`, `1W`)
/// and compound forms that concatenate them left-to-right (`1h30m`,
/// `2h15m30s`). Parsing is greedy and case-sensitive on the documented
/// capital forms (`D`, `W`); the rest are lowercase.
///
/// Returns a descriptive error string for unrecognised input.
pub fn parse_offset(s: &str) -> Result<Duration, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty offset string".to_string());
    }

    let bytes = s.as_bytes();
    let mut total = Duration::zero();
    let mut i = 0;

    while i < bytes.len() {
        // Parse the numeric prefix.
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if start == i {
            return Err(format!(
                "expected a digit at position {} in {:?}",
                start, s
            ));
        }
        let n: i64 = s[start..i]
            .parse()
            .map_err(|e| format!("invalid number {:?}: {}", &s[start..i], e))?;

        // Parse the unit suffix.
        let unit_start = i;
        while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
            i += 1;
        }
        if unit_start == i {
            return Err(format!(
                "missing unit suffix after {} in {:?}",
                n, s
            ));
        }
        let unit = &s[unit_start..i];
        let part = unit_to_duration(n, unit).map_err(|e| {
            format!("unrecognised unit {:?} in {:?}: {}", unit, s, e)
        })?;
        total += part;
    }

    Ok(total)
}

fn unit_to_duration(n: i64, unit: &str) -> Result<Duration, String> {
    if n < 0 {
        return Err(format!("negative quantity {} not allowed", n));
    }
    match unit {
        "s" => Ok(Duration::seconds(n)),
        "m" | "min" => Ok(Duration::minutes(n)),
        "h" => Ok(Duration::hours(n)),
        "D" => Ok(Duration::days(n)),
        "W" => Ok(Duration::weeks(n)),
        other => Err(format!("unknown unit {:?}", other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_30s() {
        assert_eq!(parse_offset("30s").unwrap(), Duration::seconds(30));
    }

    #[test]
    fn parses_5m() {
        assert_eq!(parse_offset("5m").unwrap(), Duration::minutes(5));
    }

    #[test]
    fn parses_5min() {
        assert_eq!(parse_offset("5min").unwrap(), Duration::minutes(5));
    }

    #[test]
    fn parses_2h() {
        assert_eq!(parse_offset("2h").unwrap(), Duration::hours(2));
    }

    #[test]
    fn parses_1d() {
        // Capital D per the spec.
        assert_eq!(parse_offset("1D").unwrap(), Duration::days(1));
    }

    #[test]
    fn parses_1w() {
        // Capital W per the spec.
        assert_eq!(parse_offset("1W").unwrap(), Duration::weeks(1));
    }

    #[test]
    fn parses_compound_1h30m() {
        assert_eq!(
            parse_offset("1h30m").unwrap(),
            Duration::hours(1) + Duration::minutes(30)
        );
    }

    #[test]
    fn rejects_banana() {
        assert!(parse_offset("banana").is_err());
    }
}
