//! Native HTML fallback follows the host font cascade; scalar/PDF lookup stays stable.
#[cfg(target_os = "macos")]
pub(super) fn families(primary: &super::FontFace) -> Vec<String> {
    use core_foundation::{array::CFArray, string::CFString};
    native_font(primary)
        .map(|font| {
            let languages = CFArray::from_CFTypes(&[CFString::new("en")]);
            core_text::font::cascade_list_for_languages(&font, &languages)
                .iter()
                .map(|descriptor| descriptor.family_name())
                .collect()
        })
        .unwrap_or_default()
}
#[cfg(target_os = "macos")]
fn native_font(primary: &super::FontFace) -> Result<core_text::font::CTFont, ()> {
    if primary.index() == 0 {
        core_text::font::new_from_buffer(primary.data())
    } else {
        primary
            .native_postscript_name()
            .ok_or(())
            .and_then(|name| core_text::font::new_from_name(&name, 16.0))
    }
}
#[cfg(not(target_os = "macos"))]
pub(super) fn families(primary: &super::FontFace) -> Vec<String> {
    let _ = primary;
    Vec::new()
}
