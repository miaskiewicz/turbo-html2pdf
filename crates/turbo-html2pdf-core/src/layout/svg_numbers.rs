//! A bounded four-number SVG viewBox parser, with optional single separators.
pub(super) fn viewbox(value: &str) -> Option<[f32; 4]> {
    let mut remaining = value.trim_ascii_start();
    let mut values = [0.0; 4];
    for (index, value) in values.iter_mut().enumerate() {
        if index != 0 {
            remaining = separator(remaining);
        }
        let (number, rest) = number(remaining)?;
        *value = number;
        remaining = rest;
    }
    if remaining.trim_ascii().is_empty() {
        Some(values)
    } else {
        None
    }
}

fn separator(value: &str) -> &str {
    let value = value.trim_ascii_start();
    value.strip_prefix(',').unwrap_or(value).trim_ascii_start()
}

fn number(value: &str) -> Option<(f32, &str)> {
    let bytes = value.as_bytes();
    let start = sign(bytes, 0);
    let end = mantissa(bytes, start)?;
    let end = exponent(bytes, end)?;
    let parsed = value[..end].parse::<f32>().ok()?;
    parsed.is_finite().then_some((parsed, &value[end..]))
}

fn sign(bytes: &[u8], index: usize) -> usize {
    match bytes.get(index) {
        Some(b'+' | b'-') => index + 1,
        _ => index,
    }
}

fn digits(bytes: &[u8], mut index: usize) -> usize {
    while bytes.get(index).is_some_and(u8::is_ascii_digit) {
        index += 1;
    }
    index
}

fn mantissa(bytes: &[u8], start: usize) -> Option<usize> {
    let whole = digits(bytes, start);
    if bytes.get(whole) != Some(&b'.') {
        return (whole > start).then_some(whole);
    }
    let fraction = digits(bytes, whole + 1);
    (whole > start || fraction > whole + 1).then_some(fraction)
}

fn exponent(bytes: &[u8], end: usize) -> Option<usize> {
    if !matches!(bytes.get(end), Some(b'e' | b'E')) {
        return Some(end);
    }
    let start = sign(bytes, end + 1);
    let end = digits(bytes, start);
    (end > start).then_some(end)
}
