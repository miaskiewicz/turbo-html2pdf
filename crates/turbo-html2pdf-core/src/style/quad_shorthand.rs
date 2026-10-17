use crate::layout::value::{parse_color, parse_length_pct, parse_px, DEFAULT_FONT_SIZE};

type Validator = fn(&str) -> bool;

const LONGHANDS: &[(&str, [&str; 4])] = &[
    (
        "margin",
        ["margin-top", "margin-right", "margin-bottom", "margin-left"],
    ),
    (
        "padding",
        [
            "padding-top",
            "padding-right",
            "padding-bottom",
            "padding-left",
        ],
    ),
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

pub(super) fn is_quad(property: &str) -> bool {
    LONGHANDS.iter().any(|(name, _)| *name == property)
}

pub(super) fn expand<'a>(property: &str, value: &'a str) -> Option<[(&'static str, &'a str); 4]> {
    let names = LONGHANDS.iter().find(|(name, _)| *name == property)?.1;
    let parts = components(value)?;
    if !admitted(property, &parts) {
        return None;
    }
    let values = match parts.as_slice() {
        [a] => [*a; 4],
        [a, b] => [*a, *b, *a, *b],
        [a, b, c] => [*a, *b, *c, *b],
        [a, b, c, d] => [*a, *b, *c, *d],
        _ => return None,
    };
    Some(std::array::from_fn(|i| (names[i], values[i])))
}

fn admitted(property: &str, parts: &[&str]) -> bool {
    if parts.len() == 1 && css_wide(parts[0]) {
        return true;
    }
    if parts.iter().any(|part| css_wide(part)) {
        return false;
    }
    let validators: &[(&str, Validator)] = &[
        ("margin", inset),
        ("padding", padding),
        ("inset", inset),
        ("border-width", border_width),
        ("border-style", border_style),
        ("border-color", border_color),
    ];
    let validate = validators.iter().find(|v| v.0 == property).unwrap().1;
    parts
        .iter()
        .all(|part| validate(&part.to_ascii_lowercase()))
}

fn css_wide(value: &str) -> bool {
    ["initial", "inherit", "unset", "revert", "revert-layer"]
        .iter()
        .any(|keyword| value.eq_ignore_ascii_case(keyword))
}

fn inset(value: &str) -> bool {
    value == "auto" || valid_length(value, true, false)
}

fn padding(value: &str) -> bool {
    valid_length(value, true, true)
}

fn border_width(value: &str) -> bool {
    ["thin", "medium", "thick"].contains(&value) || valid_length(value, false, true)
}

fn border_style(value: &str) -> bool {
    [
        "none", "hidden", "dotted", "dashed", "solid", "double", "groove", "ridge", "inset",
        "outset",
    ]
    .contains(&value)
}

fn border_color(value: &str) -> bool {
    value == "currentcolor" || parse_color(value).is_some()
}

fn valid_length(value: &str, percentage: bool, nonnegative: bool) -> bool {
    if let Ok(number) = value.parse::<f32>() {
        return number == 0.0;
    }
    length_value(value, percentage)
        .is_some_and(|value| value.is_finite() && (!nonnegative || value >= 0.0))
}

fn length_value(value: &str, percentage: bool) -> Option<f32> {
    if percentage {
        parse_length_pct(value, DEFAULT_FONT_SIZE)?.resolve(1.0)
    } else {
        parse_px(value, DEFAULT_FONT_SIZE)
    }
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

    #[test]
    fn complete_quad_grammar_is_validated_before_expansion() {
        for (property, value) in [
            ("padding", "-1px"),
            ("padding", "auto"),
            ("margin", "5px bogus"),
            ("inset", "10px bogus"),
            ("inset", "initial auto"),
            ("border-width", "3px red"),
            ("border-width", "-1px"),
            ("border-width", "3%"),
            ("border-style", "solid bogus"),
            ("border-color", "red bogus"),
        ] {
            assert!(expand(property, value).is_none(), "{property}:{value}");
        }
        for (property, value) in [
            ("padding", "0 3% 2em 4px"),
            ("margin", "-2px auto 3% 0"),
            ("inset", "auto 0 -2px 4%"),
            ("border-width", "thin medium thick 0"),
            ("border-style", "solid dotted double groove"),
            ("border-color", "red currentColor #fff rgb(0, 0, 0)"),
        ] {
            assert!(expand(property, value).is_some(), "{property}:{value}");
        }
    }
}
