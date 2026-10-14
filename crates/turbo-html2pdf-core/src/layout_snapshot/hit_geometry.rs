//! Native paint scopes snap physical origins when CSS transforms discard residuals.
use crate::{Fragment, FragmentContent};

pub(super) fn own_matrix(
    fragment: &Fragment,
    mut matrix: [f64; 6],
    scope: [f32; 2],
    scale: Option<f32>,
) -> ([f64; 6], [f32; 2]) {
    let Some(scale) = scale else {
        return (matrix, scope);
    };
    if !matches!(
        fragment.content,
        FragmentContent::Box {
            transform: Some(_),
            ..
        }
    ) {
        return (matrix, scope);
    }
    let propagate = propagation(matrix);
    let origin = [fragment.x, fragment.y];
    let mut next_scope = origin;
    for axis in 0..2 {
        let offset = origin[axis] - scope[axis];
        let rounded = (offset * scale + 0.5).floor() / scale;
        if propagate[axis] {
            next_scope[axis] = scope[axis] + rounded;
        } else {
            matrix[axis + 4] += f64::from(rounded - offset);
        }
    }
    (matrix, next_scope)
}

fn propagation(matrix: [f64; 6]) -> [bool; 2] {
    if matrix[1] == 0.0 && matrix[2] == 0.0 {
        [matrix[0] == 1.0, matrix[3] == 1.0]
    } else {
        [false, false]
    }
}
