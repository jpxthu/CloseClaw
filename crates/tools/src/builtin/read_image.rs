//! Image file support for the Read tool.
//!
//! Detects image files by extension, resizes them to a maximum dimension,
//! and encodes the result as a base64 data URI for LLM consumption.

use image::GenericImageView;
use std::path::Path;

/// Maximum dimension (width or height) for resized images.
pub(crate) const MAX_IMAGE_DIM: u32 = 2000;

/// Supported image file extensions (lowercase, without dot).
const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "gif", "webp"];

/// Check if a file path points to a supported image file by extension.
pub(crate) fn is_image_file(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|ext| IMAGE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Determine the MIME type from a file extension.
fn mime_from_extension(path: &str) -> Option<&'static str> {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())?
        .to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

/// Resize raw image bytes so that neither dimension exceeds `max_dim`.
///
/// Preserves aspect ratio. Returns the resized image encoded as PNG bytes.
pub(crate) fn resize_image(raw_bytes: &[u8], max_dim: u32) -> Result<Vec<u8>, String> {
    let img =
        image::load_from_memory(raw_bytes).map_err(|e| format!("failed to decode image: {e}"))?;
    let (w, h) = img.dimensions();
    if w <= max_dim && h <= max_dim {
        // No resize needed — re-encode as PNG for consistency.
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png)
            .map_err(|e| format!("failed to encode image: {e}"))?;
        return Ok(buf.into_inner());
    }
    let resized = img.resize(max_dim, max_dim, image::imageops::FilterType::Lanczos3);
    let mut buf = std::io::Cursor::new(Vec::new());
    resized
        .write_to(&mut buf, image::ImageFormat::Png)
        .map_err(|e| format!("failed to encode resized image: {e}"))?;
    Ok(buf.into_inner())
}

/// Encode bytes into a `data:image/{mime};base64,{data}` URI string.
pub(crate) fn encode_base64_data_uri(bytes: &[u8], mime: &str) -> String {
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    format!("data:{mime};base64,{encoded}")
}

/// Full pipeline: detect image, resize, encode as data URI.
///
/// Returns `(data_uri, mime_type)` on success.
pub(crate) fn process_image(path: &str, raw_bytes: &[u8]) -> Result<(String, String), String> {
    let mime =
        mime_from_extension(path).ok_or_else(|| format!("unsupported image extension: {path}"))?;
    let resized = resize_image(raw_bytes, MAX_IMAGE_DIM)?;
    let data_uri = encode_base64_data_uri(&resized, mime);
    Ok((data_uri, mime.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_image_file_jpg() {
        assert!(is_image_file("photo.jpg"));
        assert!(is_image_file("photo.JPG"));
        assert!(is_image_file("photo.Jpeg"));
    }

    #[test]
    fn test_is_image_file_png() {
        assert!(is_image_file("icon.png"));
        assert!(is_image_file("icon.PNG"));
    }

    #[test]
    fn test_is_image_file_gif() {
        assert!(is_image_file("anim.gif"));
    }

    #[test]
    fn test_is_image_file_webp() {
        assert!(is_image_file("photo.webp"));
    }

    #[test]
    fn test_is_image_file_non_image() {
        assert!(!is_image_file("script.rs"));
        assert!(!is_image_file("data.txt"));
        assert!(!is_image_file("image.bmp"));
        assert!(!is_image_file("file"));
        assert!(!is_image_file(""));
    }

    #[test]
    fn test_is_image_file_path_with_dirs() {
        assert!(is_image_file("/tmp/photos/pic.jpg"));
        assert!(!is_image_file("/tmp/data/archive.tar"));
    }

    #[test]
    fn test_mime_from_extension() {
        assert_eq!(mime_from_extension("a.jpg"), Some("image/jpeg"));
        assert_eq!(mime_from_extension("a.jpeg"), Some("image/jpeg"));
        assert_eq!(mime_from_extension("a.png"), Some("image/png"));
        assert_eq!(mime_from_extension("a.gif"), Some("image/gif"));
        assert_eq!(mime_from_extension("a.webp"), Some("image/webp"));
        assert_eq!(mime_from_extension("a.txt"), None);
    }

    #[test]
    fn test_encode_base64_data_uri() {
        let data = b"hello";
        let uri = encode_base64_data_uri(data, "image/png");
        assert!(uri.starts_with("data:image/png;base64,"));
        // "hello" in base64 = "aGVsbG8="
        assert!(uri.contains("aGVsbG8="));
    }

    #[test]
    fn test_resize_image_small_no_resize() {
        // Create a tiny 10x10 red PNG
        let img = image::RgbaImage::from_fn(10, 10, |_, _| image::Rgba([255, 0, 0, 255]));
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
        let raw = buf.into_inner();

        let result = resize_image(&raw, MAX_IMAGE_DIM).unwrap();
        let decoded = image::load_from_memory(&result).unwrap();
        assert_eq!(decoded.dimensions(), (10, 10));
    }

    #[test]
    fn test_resize_image_large_resizes() {
        // Create a 4000x3000 image
        let img = image::RgbaImage::from_fn(4000, 3000, |_, _| image::Rgba([0, 0, 255, 255]));
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
        let raw = buf.into_inner();

        let result = resize_image(&raw, MAX_IMAGE_DIM).unwrap();
        let decoded = image::load_from_memory(&result).unwrap();
        let (w, h) = decoded.dimensions();
        assert!(w <= MAX_IMAGE_DIM);
        assert!(h <= MAX_IMAGE_DIM);
        // Aspect ratio: 4000/3000 = 4/3, so width should be 2000, height 1500
        assert_eq!(w, 2000);
        assert_eq!(h, 1500);
    }

    #[test]
    fn test_resize_image_preserves_aspect_ratio() {
        // Create a 3000x1000 image
        let img = image::RgbaImage::from_fn(3000, 1000, |_, _| image::Rgba([0, 128, 0, 255]));
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
        let raw = buf.into_inner();

        let result = resize_image(&raw, MAX_IMAGE_DIM).unwrap();
        let decoded = image::load_from_memory(&result).unwrap();
        let (w, h) = decoded.dimensions();
        // 3000/1000 = 3:1, so width=2000, height=666 (rounded)
        assert!(w <= MAX_IMAGE_DIM);
        assert!(h <= MAX_IMAGE_DIM);
        // Check aspect ratio is approximately preserved (within 1px rounding)
        let original_ratio = 3000.0 / 1000.0;
        let result_ratio = w as f64 / h as f64;
        assert!(
            (original_ratio - result_ratio).abs() < 0.01,
            "aspect ratio not preserved: {result_ratio} vs {original_ratio}"
        );
    }

    #[test]
    fn test_resize_image_exact_max_dim() {
        let img = image::RgbaImage::from_fn(2000, 2000, |_, _| image::Rgba([128, 128, 128, 255]));
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
        let raw = buf.into_inner();

        let result = resize_image(&raw, MAX_IMAGE_DIM).unwrap();
        let decoded = image::load_from_memory(&result).unwrap();
        assert_eq!(decoded.dimensions(), (2000, 2000));
    }

    #[test]
    fn test_resize_image_invalid_bytes() {
        let result = resize_image(b"not an image", MAX_IMAGE_DIM);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("failed to decode"));
    }

    #[test]
    fn test_process_image_integration() {
        let img = image::RgbaImage::from_fn(100, 50, |_, _| image::Rgba([255, 0, 0, 255]));
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
        let raw = buf.into_inner();

        let (data_uri, mime) = process_image("photo.png", &raw).unwrap();
        assert_eq!(mime, "image/png");
        assert!(data_uri.starts_with("data:image/png;base64,"));
    }

    #[test]
    fn test_process_image_non_image_extension() {
        let result = process_image("file.txt", b"not image data");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unsupported image extension"));
    }
}
