use super::SnapshotError;
use crate::layout::value::{display_of, parse_length_pct, Display, DEFAULT_FONT_SIZE};
use crate::{ComputedStyle, StyledNode};

pub(super) fn validate(nodes: &[StyledNode], transformed: bool) -> Result<(), SnapshotError> {
    for node in nodes {
        validate_node(node, transformed)?;
    }
    Ok(())
}

fn validate_node(node: &StyledNode, transformed: bool) -> Result<(), SnapshotError> {
    let StyledNode::Element(element) = node else {
        return Ok(());
    };
    if display_of(&element.style) == Display::None {
        return Ok(());
    }
    if is_transformed_fixed(&element.style, transformed) {
        return Err(SnapshotError::UnsupportedFixedContainingBlock);
    }
    validate(
        &element.children,
        transformed || establishes_transform(&element.style),
    )
}

fn is_transformed_fixed(style: &ComputedStyle, transformed: bool) -> bool {
    transformed
        && style
            .get("position")
            .is_some_and(|p| p.eq_ignore_ascii_case("fixed"))
}

fn establishes_transform(style: &ComputedStyle) -> bool {
    style.get("transform").is_some_and(valid_transform)
}

// Validate the supported transform grammar rather than treating malformed CSS as
// an identity transform. A valid identity function still establishes a CB.
fn valid_transform(value: &str) -> bool {
    let mut remaining = value.trim();
    let mut found = false;
    while !remaining.is_empty() {
        let Some((name, args, tail)) = function(remaining) else {
            return false;
        };
        if !valid_function(name, args) {
            return false;
        }
        found = true;
        remaining = tail.trim();
    }
    found
}

fn function(value: &str) -> Option<(&str, &str, &str)> {
    let open = value.find('(')?;
    let close = closing_parenthesis(&value[open..])? + open;
    Some((&value[..open], &value[open + 1..close], &value[close + 1..]))
}

fn closing_parenthesis(value: &str) -> Option<usize> {
    let mut depth = 0;
    for (index, ch) in value.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

type Argument = fn(&str) -> bool;
const FUNCTIONS: &[(&str, usize, usize, &[Argument])] = &[
    ("translate", 1, 2, &[length]),
    ("translatex", 1, 1, &[length]),
    ("translatey", 1, 1, &[length]),
    ("translatez", 1, 1, &[absolute_length]),
    ("translate3d", 3, 3, &[length, length, absolute_length]),
    ("scale", 1, 2, &[number]),
    ("scalex", 1, 1, &[number]),
    ("scaley", 1, 1, &[number]),
    ("scalez", 1, 1, &[number]),
    ("scale3d", 3, 3, &[number]),
    ("rotate", 1, 1, &[angle]),
    ("rotatex", 1, 1, &[angle]),
    ("rotatey", 1, 1, &[angle]),
    ("rotatez", 1, 1, &[angle]),
    ("rotate3d", 4, 4, &[number, number, number, angle]),
    ("skew", 1, 2, &[angle]),
    ("skewx", 1, 1, &[angle]),
    ("skewy", 1, 1, &[angle]),
    ("perspective", 1, 1, &[perspective]),
    ("matrix", 6, 6, &[number]),
    ("matrix3d", 16, 16, &[number]),
];

fn valid_function(name: &str, arguments: &str) -> bool {
    let Some((_, min, max, kinds)) = FUNCTIONS.iter().find(|f| f.0.eq_ignore_ascii_case(name))
    else {
        return false;
    };
    let args: Vec<_> = arguments.split(',').map(str::trim).collect();
    if !(*min..=*max).contains(&args.len()) {
        return false;
    }
    args.iter()
        .enumerate()
        .all(|(index, arg)| kinds[index.min(kinds.len() - 1)](&arg.to_ascii_lowercase()))
}

fn number(value: &str) -> bool {
    value.parse::<f32>().is_ok_and(f32::is_finite)
}

fn length(value: &str) -> bool {
    if let Ok(number) = value.parse::<f32>() {
        return number == 0.0;
    }
    parse_length_pct(value, DEFAULT_FONT_SIZE)
        .and_then(|v| v.resolve(1.0))
        .is_some_and(f32::is_finite)
}

fn absolute_length(value: &str) -> bool {
    !value.contains('%') && length(value)
}

fn perspective(value: &str) -> bool {
    absolute_length(value)
        && parse_length_pct(value, DEFAULT_FONT_SIZE)
            .and_then(|v| v.resolve(1.0))
            .is_some_and(|v| v >= 0.0)
}

fn angle(value: &str) -> bool {
    if value.parse::<f32>().is_ok_and(|v| v == 0.0) {
        return true;
    }
    ["deg", "grad", "rad", "turn"]
        .iter()
        .find_map(|unit| value.strip_suffix(unit))
        .is_some_and(number)
}

#[cfg(test)]
mod tests {
    use super::valid_transform;
    #[test]
    fn transform_admission_recognizes_identity_and_ignores_invalid_css() {
        for value in [
            "translateX(0px)",
            "translateX(2PX)",
            "perspective(1px)",
            "perspective(0)",
            "rotate(0.0)",
            "translateY(0)",
            "translateZ(2px)",
            "translate3d(0,0,2px)",
            "translate(2%,3px)",
            "scale(1)",
            "scaleX(1)",
            "scaleY(1)",
            "scaleZ(1)",
            "scale3d(1,1,1)",
            "rotate(0deg)",
            "rotateX(0)",
            "rotateY(1rad)",
            "rotateZ(1turn)",
            "rotate3d(0,0,1,2grad)",
            "skew(1deg,2deg)",
            "skewX(0)",
            "skewY(0)",
            "matrix(1,0,0,1,0,0)",
            "matrix3d(1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1)",
            "translateX(calc(10% + 2px)) scale(1)",
        ] {
            assert!(valid_transform(value), "{value}");
        }
        for value in [
            "",
            "none",
            "bogus(0)",
            "translateX (0px)",
            "translateZ(2%)",
            "perspective(-1px)",
            "perspective(2%)",
            "perspective(foo)",
            "translateX(foo)",
            "translateX(auto)",
            "translateX(2)",
            "translateX(0px) junk",
            "translateX(0px",
            "rotate(foo)",
            "rotate(1)",
            "scale(NaN)",
            "scale(1,2,3)",
            "matrix(1,0,0)",
        ] {
            assert!(!valid_transform(value), "{value}");
        }
    }
}
