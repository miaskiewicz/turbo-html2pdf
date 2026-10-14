//! Preserve native 2D CSS transform precision separately from PDF float matrices.
use super::*;

#[derive(Debug, Clone, Copy)]
pub(crate) struct NativeTransform {
    pub(crate) matrix: [f64; 6],
    pub(crate) origin: [f64; 2],
}
const IDENTITY: [f64; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

pub(crate) fn native_transform(
    style: &ComputedStyle,
    bs: &BoxStyle,
    width: f32,
    height: f32,
) -> Option<NativeTransform> {
    let value = style.get("transform")?;
    if is_none_keyword(value) {
        return None;
    }
    let mut matrix = IDENTITY;
    let mut any = false;
    for (name, args) in transform_functions(value) {
        let next = function(&name, &args, bs.font_size, [width, height])?;
        matrix = multiply(matrix, next);
        any = true;
    }
    any.then_some(NativeTransform {
        matrix,
        origin: [
            f64::from(bs.transform_origin.0.resolve(width).unwrap_or(width / 2.0)),
            f64::from(
                bs.transform_origin
                    .1
                    .resolve(height)
                    .unwrap_or(height / 2.0),
            ),
        ],
    })
}
fn function(name: &str, args: &[&str], fs: f32, size: [f32; 2]) -> Option<[f64; 6]> {
    match name {
        "translate" | "translatex" | "translatey" | "translate3d" => {
            translate(name, args, fs, size)
        }
        "scale" | "scale3d" | "scalex" | "scaley" => Some(scale(name, args)),
        "rotate" | "rotatez" => Some(rotation(args)),
        "skewx" | "skewy" => Some(skew(name, args)),
        "matrix" => matrix(args),
        _ => None,
    }
}
fn number(args: &[&str], index: usize, default: f64) -> f64 {
    args.get(index)
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}
fn length(value: Option<&&str>, fs: f32, basis: f32) -> f64 {
    value
        .and_then(|value| parse_length_pct(value, fs))
        .and_then(|value| value.resolve(basis))
        .map(f64::from)
        .unwrap_or(0.0)
}
fn translate(name: &str, args: &[&str], fs: f32, size: [f32; 2]) -> Option<[f64; 6]> {
    if name == "translate3d" && length(args.get(2), fs, 0.0) != 0.0 {
        return None;
    }
    let mut out = IDENTITY;
    if name == "translatey" {
        out[5] = length(args.first(), fs, size[1]);
    } else {
        out[4] = length(args.first(), fs, size[0]);
        out[5] = length(args.get(1), fs, size[1]);
    }
    Some(out)
}
fn scale(name: &str, args: &[&str]) -> [f64; 6] {
    let x = number(args, 0, 1.0);
    let (a, d) = match name {
        "scalex" => (x, 1.0),
        "scaley" => (1.0, x),
        _ => (x, number(args, 1, x)),
    };
    [a, 0.0, 0.0, d, 0.0, 0.0]
}
fn angle(args: &[&str]) -> f64 {
    let Some(value) = args.first() else {
        return 0.0;
    };
    let units = [
        ("deg", std::f64::consts::PI / 180.0),
        ("rad", 1.0),
        ("grad", std::f64::consts::PI / 200.0),
        ("turn", std::f64::consts::TAU),
    ];
    for (unit, factor) in units {
        if let Some(value) = value.strip_suffix(unit) {
            return value.parse::<f64>().unwrap_or(0.0) * factor;
        }
    }
    value.parse::<f64>().unwrap_or(0.0).to_radians()
}
fn rotation(args: &[&str]) -> [f64; 6] {
    let (s, c) = angle(args).sin_cos();
    [c, s, -s, c, 0.0, 0.0]
}
fn skew(name: &str, args: &[&str]) -> [f64; 6] {
    let mut out = IDENTITY;
    out[if name == "skewx" { 2 } else { 1 }] = angle(args).tan();
    out
}
fn matrix(args: &[&str]) -> Option<[f64; 6]> {
    let values = args
        .iter()
        .map(|value| value.parse::<f64>())
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    values.try_into().ok()
}
fn multiply(a: [f64; 6], b: [f64; 6]) -> [f64; 6] {
    [
        a[0] * b[0] + a[2] * b[1],
        a[1] * b[0] + a[3] * b[1],
        a[0] * b[2] + a[2] * b[3],
        a[1] * b[2] + a[3] * b[3],
        a[0] * b[4] + a[2] * b[5] + a[4],
        a[1] * b[4] + a[3] * b[5] + a[5],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    fn capture(value: &str) -> Option<NativeTransform> {
        let style = ComputedStyle::from_pairs([("transform", value), ("transform-origin", "0 0")]);
        let bs = resolve_box_style(
            &style,
            ResolveCtx {
                parent_font_size: 16.0,
                cb_width: 100.0,
            },
        );
        native_transform(&style, &bs, 100.0, 60.0)
    }
    #[test]
    fn native_css_matrix_composition_keeps_full_precision_and_function_order() {
        for (value, expected) in [
            ("translate(3px,4px)", [1.0, 0.0, 0.0, 1.0, 3.0, 4.0]),
            ("translateX(25%)", [1.0, 0.0, 0.0, 1.0, 25.0, 0.0]),
            ("translateY(50%)", [1.0, 0.0, 0.0, 1.0, 0.0, 30.0]),
            ("scale(2,3)", [2.0, 0.0, 0.0, 3.0, 0.0, 0.0]),
            ("scale3d(2,3,1)", [2.0, 0.0, 0.0, 3.0, 0.0, 0.0]),
            ("scaleX(2)", [2.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
            ("scaleY(3)", [1.0, 0.0, 0.0, 3.0, 0.0, 0.0]),
            ("matrix(2,0,0,3,4,5)", [2.0, 0.0, 0.0, 3.0, 4.0, 5.0]),
            (
                "scale(2) translate(3px,4px)",
                [2.0, 0.0, 0.0, 2.0, 6.0, 8.0],
            ),
        ] {
            let actual = capture(value).unwrap();
            assert_eq!(actual.matrix, expected);
            assert_eq!(actual.origin, [0.0, 0.0]);
        }
        let rotated = capture("rotate(10deg)").unwrap().matrix;
        assert_eq!(rotated[0], 10_f64.to_radians().cos());
        assert_ne!(rotated[0], f64::from(10_f32.to_radians().cos()));
        let skew = capture("skewY(15deg)").unwrap().matrix;
        assert_eq!(skew[1], 15_f64.to_radians().tan());
    }
    #[test]
    fn native_angle_units_and_unsupported_lists_preserve_explicit_absence() {
        for value in [
            "rotate(0.1rad)",
            "rotate(10grad)",
            "rotateZ(.1turn)",
            "rotate(10)",
            "rotate()",
            "translate3d(2px,3px,0)",
            "scale()",
            "translate(foo,0)",
        ] {
            assert!(capture(value).is_some(), "{value}");
        }
        for value in [
            "none",
            "",
            "unknown(1)",
            "translate3d(2px,3px,1px)",
            "matrix(2,3)",
            "matrix(foo,0,0,1,0,0)",
        ] {
            assert!(capture(value).is_none(), "{value}");
        }
        assert_eq!(angle(&[]), 0.0);
        let style = ComputedStyle::from_pairs([("font-size", "16px")]);
        let bs = resolve_box_style(
            &style,
            ResolveCtx {
                parent_font_size: 16.0,
                cb_width: 100.0,
            },
        );
        assert!(native_transform(&style, &bs, 100.0, 60.0).is_none());
    }
}
