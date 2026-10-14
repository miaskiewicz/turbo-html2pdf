const LONGHANDS: &[(&str, [&str; 4])] = &[
    ("inset", ["top", "right", "bottom", "left"]),
    (
        "border-width",
        [
            "border-top-width",
            "border-right-width",
            "border-bottom-width",
            "border-left-width",
        ],
    ),
    (
        "border-style",
        [
            "border-top-style",
            "border-right-style",
            "border-bottom-style",
            "border-left-style",
        ],
    ),
    (
        "border-color",
        [
            "border-top-color",
            "border-right-color",
            "border-bottom-color",
            "border-left-color",
        ],
    ),
];

pub(super) fn expand<'a>(property: &str, value: &'a str) -> Option<[(&'static str, &'a str); 4]> {
    let names = LONGHANDS.iter().find(|(name, _)| *name == property)?.1;
    let parts = components(value)?;
    let values = match parts.as_slice() {
        [a] => [*a; 4],
        [a, b] => [*a, *b, *a, *b],
        [a, b, c] => [*a, *b, *c, *b],
        [a, b, c, d] => [*a, *b, *c, *d],
        _ => return None,
    };
    Some(std::array::from_fn(|i| (names[i], values[i])))
}

fn components(value: &str) -> Option<Vec<&str>> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut depth = 0;
    for (index, ch) in value.char_indices() {
        depth = next_depth(ch, depth);
        if depth == 0 && ch.is_whitespace() {
            parts.push(value[start..index].trim());
            start = index + ch.len_utf8();
        }
    }
    if depth != 0 {
        return None;
    }
    parts.push(value[start..].trim());
    Some(parts.into_iter().filter(|part| !part.is_empty()).collect())
}

fn next_depth(ch: char, depth: i32) -> i32 {
    match ch {
        '(' => depth + 1,
        ')' => depth - 1,
        _ => depth,
    }
}

#[cfg(test)]
mod tests {
    use super::expand;
    #[test]
    fn quad_values_keep_nested_functions_and_reject_invalid_counts() {
        for (input, expected) in [
            ("1px", ["1px"; 4]),
            ("1px 2px", ["1px", "2px", "1px", "2px"]),
            ("1px\t2px 3px", ["1px", "2px", "3px", "2px"]),
            ("1px 2px 3px 4px", ["1px", "2px", "3px", "4px"]),
            (
                "calc(10% + 2px) auto",
                ["calc(10% + 2px)", "auto", "calc(10% + 2px)", "auto"],
            ),
        ] {
            assert_eq!(
                expand("inset", input).unwrap().map(|(_, value)| value),
                expected
            );
        }
        for property in ["border-width", "border-style", "border-color"] {
            assert!(expand(property, "inherit").is_some());
        }
        for value in ["", "1 2 3 4 5", "calc(1px", "1px)"] {
            assert!(expand("inset", value).is_none());
        }
        assert!(expand("width", "1px").is_none());
    }
}
