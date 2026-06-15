//! Document-domain filters (§2.12): currency, number, date, datetime, percent,
//! ordinal, pad, truncate, wordwrap. Locale-aware where relevant. Jinja
//! built-ins (`upper`, `default`, `round`, …) come from MiniJinja unchanged.

use chrono::{DateTime, FixedOffset, NaiveDate, TimeZone, Utc};
use minijinja::value::Value;
use minijinja::{Error, ErrorKind};

// ---------------------------------------------------------------------------
// Numeric formatting
// ---------------------------------------------------------------------------

/// Locale-specific grouping/decimal/symbol placement.
struct Locale {
    thousands: char,
    decimal: char,
    symbol_before: bool,
}

fn is_euro_locale(code: &str) -> bool {
    let lang = code.split('-').next().unwrap_or("");
    matches!(lang, "pt" | "de" | "es" | "it" | "nl" | "pl" | "fr")
}

fn locale_of(code: &str) -> Locale {
    if is_euro_locale(code) {
        Locale {
            thousands: '.',
            decimal: ',',
            symbol_before: false,
        }
    } else {
        Locale {
            thousands: ',',
            decimal: '.',
            symbol_before: true,
        }
    }
}

fn group_int(int_digits: &str, sep: char) -> String {
    let n = int_digits.len();
    let mut out = String::with_capacity(n + n / 3);
    for (i, ch) in int_digits.chars().enumerate() {
        if i > 0 && (n - i).is_multiple_of(3) {
            out.push(sep);
        }
        out.push(ch);
    }
    out
}

fn split_decimal(s: &str) -> (&str, &str) {
    s.split_once('.').unwrap_or((s, ""))
}

fn fmt_amount(value: f64, decimals: usize, loc: &Locale) -> String {
    let neg = value.is_sign_negative() && value != 0.0;
    let rendered = format!("{:.*}", decimals, value.abs());
    let (int_part, frac) = split_decimal(&rendered);
    let mut out = String::new();
    if neg {
        out.push('-');
    }
    out.push_str(&group_int(int_part, loc.thousands));
    if !frac.is_empty() {
        out.push(loc.decimal);
        out.push_str(frac);
    }
    out
}

fn default_decimals(value: f64) -> usize {
    if value.fract() == 0.0 {
        0
    } else {
        2
    }
}

fn symbol_of(ccy: &str) -> String {
    match ccy.to_uppercase().as_str() {
        "EUR" => "€".into(),
        "USD" => "$".into(),
        "GBP" => "£".into(),
        "JPY" => "¥".into(),
        "PLN" => "zł".into(),
        other => format!("{other} "),
    }
}

fn assemble_currency(amount: &str, symbol: &str, loc: &Locale) -> String {
    if loc.symbol_before {
        format!("{symbol}{amount}")
    } else {
        format!("{amount}\u{00a0}{symbol}")
    }
}

/// `currency(value, ccy, locale="en")` — formats a money amount with grouping,
/// two decimals, and a currency symbol placed per locale. A negative sign sits
/// outside the symbol (`-£5.00`, not `£-5.00`).
pub fn currency(value: f64, ccy: String, locale: Option<String>) -> Result<String, Error> {
    let loc = locale_of(locale.as_deref().unwrap_or("en"));
    let body = assemble_currency(&fmt_amount(value.abs(), 2, &loc), &symbol_of(&ccy), &loc);
    let negative = value.is_sign_negative() && value != 0.0;
    Ok(if negative { format!("-{body}") } else { body })
}

/// `number(value, decimals=auto)` — grouped number; defaults to 0 decimals for
/// integers and 2 for fractional values.
pub fn number(value: f64, decimals: Option<i64>) -> Result<String, Error> {
    let dp = decimals.map_or_else(|| default_decimals(value), |d| d.max(0) as usize);
    Ok(fmt_amount(value, dp, &locale_of("en")))
}

/// `percent(value, decimals=0)` — multiplies by 100 and appends `%`.
pub fn percent(value: f64, decimals: Option<i64>) -> Result<String, Error> {
    let dp = decimals.map_or(0, |d| d.max(0) as usize);
    Ok(format!("{:.*}%", dp, value * 100.0))
}

fn ordinal_suffix(n: i64) -> &'static str {
    let abs = n.unsigned_abs() % 100;
    if (11..=13).contains(&abs) {
        return "th";
    }
    match abs % 10 {
        1 => "st",
        2 => "nd",
        3 => "rd",
        _ => "th",
    }
}

/// `ordinal(n)` — `1` → `1st`, `2` → `2nd`, `13` → `13th`.
pub fn ordinal(n: i64) -> String {
    format!("{n}{}", ordinal_suffix(n))
}

// ---------------------------------------------------------------------------
// String formatting
// ---------------------------------------------------------------------------

/// `pad(value, width, fill=" ")` — left-pads the stringified value to `width`.
pub fn pad(value: Value, width: i64, fill: Option<String>) -> String {
    let s = value.to_string();
    let target = width.max(0) as usize;
    let ch = fill.and_then(|f| f.chars().next()).unwrap_or(' ');
    let deficit = target.saturating_sub(s.chars().count());
    let mut out: String = std::iter::repeat_n(ch, deficit).collect();
    out.push_str(&s);
    out
}

/// `truncate(value, length, suffix="…")` — shortens to `length` chars including
/// the suffix when the value is longer.
pub fn truncate(value: Value, length: i64, suffix: Option<String>) -> String {
    let s = value.to_string();
    let max = length.max(0) as usize;
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s;
    }
    let suf = suffix.unwrap_or_else(|| "…".into());
    let keep = max.saturating_sub(suf.chars().count());
    let head: String = chars.iter().take(keep).collect();
    format!("{head}{suf}")
}

fn push_word(lines: &mut Vec<String>, line: &mut String, word: &str, width: usize) {
    if line.is_empty() {
        *line = word.to_string();
    } else if line.chars().count() + 1 + word.chars().count() <= width {
        line.push(' ');
        line.push_str(word);
    } else {
        lines.push(std::mem::take(line));
        *line = word.to_string();
    }
}

fn wrap_words(s: &str, width: usize) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in s.split_whitespace() {
        push_word(&mut lines, &mut line, word, width);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines.join("\n")
}

/// `wordwrap(value, width)` — greedy word wrap at `width` characters.
pub fn wordwrap(value: Value, width: i64) -> String {
    wrap_words(&value.to_string(), width.max(1) as usize)
}

// ---------------------------------------------------------------------------
// Date / time
// ---------------------------------------------------------------------------

fn date_err(msg: &str) -> Error {
    Error::new(ErrorKind::InvalidOperation, msg.to_string())
}

fn ts_to_utc(ts: i64) -> Result<DateTime<Utc>, Error> {
    Utc.timestamp_opt(ts, 0)
        .single()
        .ok_or_else(|| date_err("date(): timestamp out of range"))
}

fn parse_str_dt(s: &str) -> Result<DateTime<Utc>, Error> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc));
    }
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        let naive = d.and_hms_opt(0, 0, 0).expect("midnight is always valid");
        return Ok(Utc.from_utc_datetime(&naive));
    }
    Err(date_err("date(): unrecognized date string"))
}

fn parse_numeric_dt(value: &Value) -> Result<DateTime<Utc>, Error> {
    let ts = value
        .to_string()
        .parse::<i64>()
        .map_err(|_| date_err("date(): not a timestamp"))?;
    ts_to_utc(ts)
}

fn parse_value_dt(value: &Value) -> Result<DateTime<Utc>, Error> {
    match value.as_str() {
        Some(s) => parse_str_dt(s),
        None => parse_numeric_dt(value),
    }
}

fn parse_hhmm_offset(s: &str) -> Result<FixedOffset, Error> {
    let sign = offset_sign(s)?;
    let (h, m) = s[1..]
        .split_once(':')
        .ok_or_else(|| date_err("date(): bad tz offset"))?;
    let hours = parse_i32(h)?;
    let mins = parse_i32(m)?;
    FixedOffset::east_opt(sign * (hours * 3600 + mins * 60))
        .ok_or_else(|| date_err("date(): tz out of range"))
}

fn offset_sign(s: &str) -> Result<i32, Error> {
    match s.as_bytes().first() {
        Some(b'+') => Ok(1),
        Some(b'-') => Ok(-1),
        _ => Err(date_err("date(): bad tz offset")),
    }
}

fn parse_i32(s: &str) -> Result<i32, Error> {
    s.parse().map_err(|_| date_err("date(): bad tz offset"))
}

fn parse_offset(tz: Option<&str>) -> Result<FixedOffset, Error> {
    let Some(s) = tz else {
        return Ok(utc_offset());
    };
    if s.eq_ignore_ascii_case("utc") || s == "Z" {
        return Ok(utc_offset());
    }
    parse_hhmm_offset(s)
}

fn utc_offset() -> FixedOffset {
    FixedOffset::east_opt(0).expect("zero offset is valid")
}

fn translate_fmt(fmt: &str) -> String {
    const PAIRS: [(&str, &str); 7] = [
        ("YYYY", "%Y"),
        ("YY", "%y"),
        ("MM", "%m"),
        ("DD", "%d"),
        ("HH", "%H"),
        ("mm", "%M"),
        ("ss", "%S"),
    ];
    let mut out = fmt.to_string();
    for (token, repl) in PAIRS {
        out = out.replace(token, repl);
    }
    out
}

fn format_dt(value: &Value, fmt: &str, tz: Option<&str>) -> Result<String, Error> {
    let utc = parse_value_dt(value)?;
    let local = utc.with_timezone(&parse_offset(tz)?);
    Ok(local.format(&translate_fmt(fmt)).to_string())
}

/// `date(value, fmt="YYYY-MM-DD", tz="UTC")` — formats a timestamp or RFC3339 /
/// `YYYY-MM-DD` string with `YYYY MM DD HH mm ss` tokens.
pub fn date(value: Value, fmt: Option<String>, tz: Option<String>) -> Result<String, Error> {
    let f = fmt.unwrap_or_else(|| "YYYY-MM-DD".into());
    format_dt(&value, &f, tz.as_deref())
}

/// `datetime(value, fmt="YYYY-MM-DD HH:mm:ss", tz="UTC")` — like `date` with a
/// time-bearing default format.
pub fn datetime(value: Value, fmt: Option<String>, tz: Option<String>) -> Result<String, Error> {
    let f = fmt.unwrap_or_else(|| "YYYY-MM-DD HH:mm:ss".into());
    format_dt(&value, &f, tz.as_deref())
}
