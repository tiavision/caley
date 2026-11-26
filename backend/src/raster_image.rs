//! The raster images served from data a sender or contact chose (mail images, attachments, contact
//! photos): sniffed from the bytes, never taken from a declared type, so nothing active (HTML, SVG,
//! script) is served as an image.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AllowedImage {
    Png,
    Jpeg,
    Gif,
    WebP,
    Avif,
}

impl AllowedImage {
    pub(crate) fn content_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::WebP => "image/webp",
            Self::Avif => "image/avif",
        }
    }
}

/// PNG, JPEG, GIF, WebP or AVIF, by the bytes' signature.
pub(crate) fn detect_allowed_image(data: &[u8]) -> Option<AllowedImage> {
    match infer::get(data).map(|t| t.mime_type()) {
        Some("image/png") => Some(AllowedImage::Png),
        Some("image/jpeg") => Some(AllowedImage::Jpeg),
        Some("image/gif") => Some(AllowedImage::Gif),
        Some("image/webp") => Some(AllowedImage::WebP),
        Some("image/avif") => Some(AllowedImage::Avif),
        _ => None,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A 1×1 PNG.
    pub(crate) const PNG: &[u8] =
        b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89";

    #[test]
    fn image_detection_sniffs_bytes_not_declared_type() {
        assert_eq!(detect_allowed_image(PNG), Some(AllowedImage::Png));

        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>"#;
        assert_eq!(detect_allowed_image(svg), None);

        let html = b"<!doctype html><script>alert(1)</script>";
        assert_eq!(detect_allowed_image(html), None);
    }
}
