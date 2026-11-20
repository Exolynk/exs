//! The images in which values cross the boundary of a module in the browser.

use exs_abi::{BoundaryLimits, ExsValue};

/// Decodes one image a module wrote within the default limits.
pub(super) fn read_image(bytes: &[u8]) -> Result<ExsValue, exs_abi::BoundaryError> {
    ExsValue::from_image_with_limits(bytes, BoundaryLimits::default())
}

/// Encodes one value the runner passes to a module as an image.
pub(super) fn image(value: &ExsValue) -> Result<Vec<u8>, String> {
    value
        .to_image_with_limits(BoundaryLimits::unrestricted())
        .map_err(|error| format!("could not encode image: {error}"))
}

/// Encodes the List of `items` the runner passes to a module as an image, without copying them.
pub(super) fn list_image(items: &[ExsValue]) -> Result<Vec<u8>, String> {
    ExsValue::list_to_image_with_limits(items, BoundaryLimits::unrestricted())
        .map_err(|error| format!("could not encode image: {error}"))
}
