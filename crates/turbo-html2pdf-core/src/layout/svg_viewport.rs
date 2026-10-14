//! Native replaced SVG viewport sizing; this path never invents image resources.
use super::imgsize::{size_viewport_live, SizeCtx};
use super::value::{parse_length_pct, BoxStyle, Display, LengthPct};
use crate::StyledElement;

#[derive(Debug, Clone)]
pub(crate) struct SvgViewport {
    width: Option<String>,
    layout_scale: f32,
    height: Option<String>,
    ratio: Option<f32>,
    width_from_attribute: bool,
    height_from_attribute: bool,
}
impl SvgViewport {
    pub(crate) fn from_element(element: &StyledElement, layout_scale: f32) -> Self {
        Self {
            layout_scale,
            width: attribute(element, "width").map(str::to_owned),
            height: attribute(element, "height").map(str::to_owned),
            ratio: viewbox_ratio(element),
            width_from_attribute: !css_dimension_overrides(element.style.get("width")),
            height_from_attribute: !css_dimension_overrides(element.style.get("height")),
        }
    }
    pub(crate) fn apply_attributes(&self, mut style: BoxStyle) -> BoxStyle {
        if self.width_from_attribute {
            style.width =
                attribute_dimension(self.width.as_deref(), style.font_size).unwrap_or(style.width);
        }
        if self.height_from_attribute {
            style.height = attribute_dimension(self.height.as_deref(), style.font_size)
                .unwrap_or(style.height);
        }
        style
    }
    pub(crate) fn size(&self, context: &SizeCtx) -> (f32, f32) {
        let width = intrinsic(self.width.as_deref(), context.style.font_size);
        let height = intrinsic(self.height.as_deref(), context.style.font_size);
        let ratio = dimension_ratio(width, height).or(self.ratio);
        size_viewport_live(width, height, ratio, self.layout_scale, context)
    }
    pub(crate) fn natural_width(&self, font_size: f32) -> f32 {
        let width = intrinsic(self.width.as_deref(), font_size);
        let height = intrinsic(self.height.as_deref(), font_size);
        width
            .or_else(|| height.zip(self.ratio).map(|(h, ratio)| h * ratio))
            .unwrap_or(300.0)
    }
}

pub(crate) fn display(element: &StyledElement) -> Display {
    match element.style.get("display") {
        None => Display::Inline,
        Some(_) => super::value::display_of(&element.style),
    }
}

fn attribute<'a>(element: &'a StyledElement, name: &str) -> Option<&'a str> {
    element
        .attrs
        .iter()
        .find(|attr| attr.name == name)
        .map(|attr| attr.value.as_str())
}

fn css_dimension_overrides(value: Option<&str>) -> bool {
    let Some(value) = value else {
        return false;
    };
    // The cascade retains an empty winning value after failed var substitution;
    // it is initial/auto, not a discarded declaration that revives a hint.
    if value.is_empty() {
        return true;
    }
    let Some(parsed) = dimension(Some(value), super::value::DEFAULT_FONT_SIZE) else {
        return false;
    };
    valid_css_dimension(value, parsed)
}

fn valid_css_dimension(raw: &str, value: LengthPct) -> bool {
    match value {
        LengthPct::Px(value) | LengthPct::Pct(value) => value >= 0.0 || raw.starts_with("calc("),
        _ => true,
    }
}

fn attribute_dimension(value: Option<&str>, font_size: f32) -> Option<LengthPct> {
    let raw = value?;
    // A present invalid SVG animated length retains its initial 100% value;
    // an absent presentation attribute leaves CSS auto sizing intact.
    Some(dimension(Some(raw), font_size).unwrap_or(LengthPct::Pct(100.0)))
}

fn dimension(value: Option<&str>, font_size: f32) -> Option<LengthPct> {
    let dimension = parse_length_pct(value?, font_size)?;
    valid_dimension(dimension).then_some(dimension)
}

fn valid_dimension(dimension: LengthPct) -> bool {
    match dimension {
        LengthPct::Px(value) | LengthPct::Pct(value) => value.is_finite(),
        LengthPct::Auto => true,
        LengthPct::Calc { pct, px } => pct.is_finite() && px.is_finite(),
    }
}

fn intrinsic(value: Option<&str>, font_size: f32) -> Option<f32> {
    match dimension(value, font_size)? {
        LengthPct::Px(value) => Some(value.max(0.0)),
        _ => None,
    }
}

fn dimension_ratio(width: Option<f32>, height: Option<f32>) -> Option<f32> {
    let (width, height) = width.zip(height)?;
    let ratio = width / height;
    (width > 0.0 && height > 0.0 && ratio.is_finite()).then_some(ratio)
}

fn viewbox_ratio(element: &StyledElement) -> Option<f32> {
    let values = super::svg_numbers::viewbox(attribute(element, "viewBox")?)?;
    dimension_ratio(Some(values[2]), Some(values[3]))
}
