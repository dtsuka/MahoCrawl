use base64::Engine;
use image::{codecs::jpeg::JpegEncoder, ImageReader};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use url::Url;

use crate::config::{
    CaptureImage, CaptureItem, CaptureViewport, CrawlError, SeoPageItem, MAX_REPORT_LENGTH,
};
use crate::paths::{canonical_path, is_path_allowed, safe_size_slug};

pub(crate) fn image_mime_type(path: &Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => Some("image/png"),
        Some("jpg") | Some("jpeg") => Some("image/jpeg"),
        Some("webp") => Some("image/webp"),
        _ => None,
    }
}

pub(crate) fn clean_ansi(value: &str) -> String {
    let mut output = String::new();
    let mut escape = false;
    for ch in value.chars() {
        if escape {
            if ch.is_ascii_alphabetic() {
                escape = false;
            }
            continue;
        }
        if ch == '\u{1b}' {
            escape = true;
        } else {
            output.push(ch);
        }
    }
    output
}

pub(crate) fn encode_thumbnail(path: &Path) -> Result<CaptureImage, String> {
    let image = ImageReader::open(path)
        .map_err(|error| error.to_string())?
        .decode()
        .map_err(|error| error.to_string())?;
    let thumbnail = image.thumbnail(480, 320);
    let mut bytes = Vec::new();
    let mut encoder = JpegEncoder::new_with_quality(&mut bytes, 78);
    encoder
        .encode_image(&thumbnail)
        .map_err(|error| error.to_string())?;
    Ok(CaptureImage {
        mime_type: "image/jpeg".to_string(),
        data_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
    })
}

pub(crate) fn list_capture_items(
    root: &Path,
    configured: &[CaptureViewport],
) -> Result<Vec<CaptureItem>, CrawlError> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let allowed = ["png", "jpg", "jpeg", "webp"];
    let mut captures = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let entries =
            fs::read_dir(&directory).map_err(|error| CrawlError::Io(error.to_string()))?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let is_hidden = path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(|name| name.starts_with('.'));
                if !is_hidden {
                    stack.push(path);
                }
                continue;
            }
            let extension = path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if !allowed.contains(&extension.as_str()) {
                continue;
            }
            let metadata =
                fs::metadata(&path).map_err(|error| CrawlError::Io(error.to_string()))?;
            let size_slug = path
                .parent()
                .and_then(Path::parent)
                .and_then(Path::file_name)
                .and_then(|value| value.to_str())
                .unwrap_or_default();
            let matching = configured
                .iter()
                .find(|viewport| safe_size_slug(viewport) == size_slug);
            let (size_id, size_label, width, height) = matching
                .map(|viewport| {
                    (
                        Some(viewport.id.clone()),
                        Some(viewport.label.clone()),
                        Some(viewport.width),
                        Some(viewport.height),
                    )
                })
                .unwrap_or((None, None, None, None));
            let modified_at = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs())
                .unwrap_or_default();
            captures.push(CaptureItem {
                path: path.to_string_lossy().to_string(),
                filename: path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default()
                    .to_string(),
                bytes: metadata.len(),
                modified_at,
                size_id,
                size_label,
                width,
                height,
            });
        }
    }
    captures.sort_by(|left, right| {
        left.filename
            .to_lowercase()
            .cmp(&right.filename.to_lowercase())
    });
    Ok(captures)
}

pub(crate) fn is_page_screenshot_url(raw_url: &str) -> bool {
    let Ok(url) = Url::parse(raw_url) else {
        return true;
    };
    let Some(last_segment) = url.path().rsplit('/').next() else {
        return true;
    };
    let Some((_, extension)) = last_segment.rsplit_once('.') else {
        return true;
    };
    if extension.is_empty() {
        return true;
    }
    !matches!(
        extension.to_ascii_lowercase().as_str(),
        "7z" | "7zip"
            | "aac"
            | "avif"
            | "avi"
            | "bmp"
            | "css"
            | "csv"
            | "doc"
            | "docx"
            | "eot"
            | "gif"
            | "gz"
            | "ico"
            | "jpeg"
            | "jpg"
            | "js"
            | "json"
            | "m4a"
            | "m4v"
            | "map"
            | "mjs"
            | "mov"
            | "mp3"
            | "mp4"
            | "ogg"
            | "otf"
            | "pdf"
            | "png"
            | "ppt"
            | "pptx"
            | "rar"
            | "svg"
            | "tar"
            | "tif"
            | "tiff"
            | "ttf"
            | "txt"
            | "wav"
            | "wasm"
            | "webmanifest"
            | "webm"
            | "webp"
            | "woff"
            | "woff2"
            | "xls"
            | "xlsx"
            | "xml"
            | "zip"
    )
}

/// Resolve a screenshot path from a report row without ever leaving that
/// report's own `screenshots` directory. SiteOne currently emits absolute
/// paths, but a relative path is interpreted relative to the directory that
/// contains `report.json` (for example, `screenshots/page.png`).
pub(crate) fn resolve_screenshot_path(
    report_path: &Path,
    screenshot_dir: &Path,
    raw_path: &str,
) -> Option<PathBuf> {
    let raw_path = raw_path.trim();
    if raw_path.is_empty() {
        return None;
    }
    let report_root = report_path.parent()?;
    let canonical_report_root = canonical_path(report_root)?;
    let expected_screenshot_dir = report_root.join("screenshots");
    let canonical_expected_screenshot_dir = canonical_path(&expected_screenshot_dir)?;
    let canonical_screenshot_dir = canonical_path(screenshot_dir)?;
    // Keep the caller's directory tied to this report's size folder. This
    // rejects a report row that points to another size's screenshots and also
    // rejects a `screenshots` symlink that escapes the size folder.
    if canonical_screenshot_dir != canonical_expected_screenshot_dir
        || !canonical_screenshot_dir.starts_with(&canonical_report_root)
    {
        return None;
    }

    let raw = Path::new(raw_path);
    let candidate = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        report_root.join(raw)
    };
    let canonical_target = canonical_path(&candidate)?;
    if !canonical_target.starts_with(&canonical_screenshot_dir) {
        return None;
    }
    Some(canonical_target)
}

pub(crate) fn capture_item_for_path(
    path: &Path,
    viewport: &CaptureViewport,
) -> Option<CaptureItem> {
    image_mime_type(path)?;
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    let filename = path.file_name()?.to_str()?.to_string();
    let modified_at = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    Some(CaptureItem {
        path: path.to_string_lossy().to_string(),
        filename,
        bytes: metadata.len(),
        modified_at,
        size_id: Some(viewport.id.clone()),
        size_label: Some(viewport.label.clone()),
        width: Some(viewport.width),
        height: Some(viewport.height),
    })
}

pub(crate) fn filter_non_page_screenshots(
    report_path: &Path,
    screenshot_dir: &Path,
) -> Result<usize, CrawlError> {
    let metadata = fs::metadata(report_path).map_err(|error| CrawlError::Io(error.to_string()))?;
    if metadata.len() > MAX_REPORT_LENGTH {
        return Ok(0);
    }
    let data = fs::read(report_path).map_err(|error| CrawlError::Io(error.to_string()))?;
    let report: serde_json::Value = serde_json::from_slice(&data)
        .map_err(|error| CrawlError::Io(format!("JSONレポートを読み込めません: {}", error)))?;
    let mut removed = 0;
    for row in report_rows(&report, "browser-screenshots") {
        let Some(raw_url) = row.get("url").and_then(serde_json::Value::as_str) else {
            continue;
        };
        if is_page_screenshot_url(raw_url) {
            continue;
        }
        let Some(raw_path) = row.get("path").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let Some(path) = resolve_screenshot_path(report_path, screenshot_dir, raw_path) else {
            continue;
        };
        if fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

pub(crate) fn report_rows<'a>(
    report: &'a serde_json::Value,
    table_name: &str,
) -> Vec<&'a serde_json::Map<String, serde_json::Value>> {
    report
        .get("tables")
        .and_then(|tables| tables.get(table_name))
        .and_then(|table| table.get("rows"))
        .and_then(serde_json::Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(serde_json::Value::as_object)
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn row_text(
    row: &serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<String> {
    keys.iter().find_map(|key| {
        row.get(*key)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    })
}

pub(crate) fn report_url(base_url: &str, raw_url: &str) -> String {
    if let Ok(url) = Url::parse(raw_url) {
        return url.to_string();
    }
    Url::parse(base_url)
        .ok()
        .and_then(|base| base.join(raw_url).ok())
        .map(|url| url.to_string())
        .unwrap_or_else(|| raw_url.to_string())
}

pub(crate) fn remove_html_block(value: &str, tag: &str) -> String {
    let lowered = value.to_ascii_lowercase();
    let open = format!("<{}", tag);
    let close = format!("</{}>", tag);
    let mut output = String::new();
    let mut cursor = 0;
    while let Some(relative_start) = lowered[cursor..].find(&open) {
        let start = cursor + relative_start;
        output.push_str(&value[cursor..start]);
        let after_start = start + open.len();
        let Some(relative_end) = lowered[after_start..].find(&close) else {
            return output;
        };
        cursor = after_start + relative_end + close.len();
    }
    output.push_str(&value[cursor..]);
    output
}

pub(crate) fn normalize_heading_text(value: &str) -> String {
    let value = remove_html_block(&remove_html_block(value, "script"), "style");
    let mut output = String::new();
    let mut in_tag = false;
    for ch in value.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => output.push(ch),
            _ => {}
        }
    }
    let mut decoded = String::new();
    let mut remaining = output.as_str();
    while let Some(index) = remaining.find('&') {
        decoded.push_str(&remaining[..index]);
        remaining = &remaining[index..];
        if let Some(end) = remaining.find(';') {
            let entity = &remaining[1..end];
            let character = match entity {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                _ => entity
                    .strip_prefix("#x")
                    .or_else(|| entity.strip_prefix("#X"))
                    .and_then(|value| u32::from_str_radix(value, 16).ok())
                    .or_else(|| {
                        entity
                            .strip_prefix('#')
                            .and_then(|value| value.parse().ok())
                    })
                    .and_then(char::from_u32),
            };
            if let Some(character) = character {
                decoded.push(character);
                remaining = &remaining[end + 1..];
                continue;
            }
        }
        decoded.push('&');
        remaining = &remaining[1..];
    }
    decoded.push_str(remaining);
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) fn extract_heading(value: &str, level: u8) -> Option<String> {
    let lowered = value.to_ascii_lowercase();
    let marker = format!("<h{}", level);
    let mut search_from = 0;
    while let Some(relative_start) = lowered[search_from..].find(&marker) {
        let start = search_from + relative_start;
        let Some(relative_end_tag) = lowered[start..].find('>') else {
            break;
        };
        let content_start = start + relative_end_tag + 1;
        let content_end = (1..=6)
            .filter_map(|next_level| lowered[content_start..].find(&format!("<h{}", next_level)))
            .map(|offset| content_start + offset)
            .min()
            .unwrap_or(value.len());
        let text = normalize_heading_text(&value[content_start..content_end]);
        if !text.is_empty() {
            return Some(text);
        }
        search_from = content_start;
    }
    None
}

pub(crate) fn size_for_report_path<'a>(
    path: &Path,
    configured: &'a [CaptureViewport],
) -> Option<&'a CaptureViewport> {
    let size_slug = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    configured
        .iter()
        .find(|viewport| safe_size_slug(viewport) == size_slug)
}

pub(crate) fn ensure_seo_page(
    pages: &mut HashMap<String, SeoPageItem>,
    base_url: &str,
    row: &serde_json::Map<String, serde_json::Value>,
    size: Option<&CaptureViewport>,
) -> Option<String> {
    let raw_url = row_text(row, &["urlPathAndQuery", "url"])?;
    let url = report_url(base_url, &raw_url);
    let page = pages.entry(url.clone()).or_insert_with(|| SeoPageItem {
        url: url.clone(),
        ..SeoPageItem::default()
    });
    if let Some(viewport) = size {
        if !page.size_ids.iter().any(|id| id == &viewport.id) {
            page.size_ids.push(viewport.id.clone());
        }
        if !page
            .size_labels
            .iter()
            .any(|label| label == &viewport.label)
        {
            page.size_labels.push(viewport.label.clone());
        }
    }
    Some(url)
}

pub(crate) fn fill_if_missing(target: &mut Option<String>, value: Option<String>) {
    if target.is_none() {
        *target = value;
    }
}

pub(crate) fn parse_seo_report(
    path: &Path,
    configured: &[CaptureViewport],
) -> Result<Vec<SeoPageItem>, CrawlError> {
    let metadata = fs::metadata(path).map_err(|error| CrawlError::Io(error.to_string()))?;
    if metadata.len() > MAX_REPORT_LENGTH {
        return Err(CrawlError::Io("SEOレポートが大きすぎます。".to_string()));
    }
    let data = fs::read(path).map_err(|error| CrawlError::Io(error.to_string()))?;
    let report: serde_json::Value = serde_json::from_slice(&data)
        .map_err(|error| CrawlError::Io(format!("SEOレポートを読み込めません: {}", error)))?;
    let base_url = report
        .pointer("/options/url")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let size = size_for_report_path(path, configured);
    let mut pages = HashMap::<String, SeoPageItem>::new();

    for row in report_rows(&report, "seo") {
        if let Some(url) = ensure_seo_page(&mut pages, base_url, row, size) {
            let page = pages.get_mut(&url).expect("SEO page was inserted");
            fill_if_missing(&mut page.title, row_text(row, &["title"]));
            fill_if_missing(&mut page.description, row_text(row, &["description"]));
            fill_if_missing(
                &mut page.canonical,
                row_text(row, &["canonical", "canonicalUrl", "canonicalURL"]),
            );
            fill_if_missing(&mut page.h1, row_text(row, &["h1"]));
            fill_if_missing(&mut page.status, row_text(row, &["indexing"]));
        }
    }
    for row in report_rows(&report, "open-graph") {
        if let Some(url) = ensure_seo_page(&mut pages, base_url, row, size) {
            let page = pages.get_mut(&url).expect("SEO page was inserted");
            fill_if_missing(&mut page.og_title, row_text(row, &["ogTitle"]));
            fill_if_missing(&mut page.og_description, row_text(row, &["ogDescription"]));
            fill_if_missing(&mut page.og_image, row_text(row, &["ogImage"]));
            fill_if_missing(&mut page.twitter_title, row_text(row, &["twitterTitle"]));
            fill_if_missing(
                &mut page.twitter_description,
                row_text(row, &["twitterDescription"]),
            );
            fill_if_missing(&mut page.twitter_image, row_text(row, &["twitterImage"]));
        }
    }
    for row in report_rows(&report, "seo-headings") {
        if let Some(url) = ensure_seo_page(&mut pages, base_url, row, size) {
            let page = pages.get_mut(&url).expect("SEO page was inserted");
            fill_if_missing(&mut page.h1, row_text(row, &["h1"]));
            fill_if_missing(&mut page.h2, row_text(row, &["h2"]));
            if page.h1.is_none() {
                page.h1 = row_text(row, &["headings"]).and_then(|value| extract_heading(&value, 1));
            }
            if page.h2.is_none() {
                page.h2 = row_text(row, &["headings"]).and_then(|value| extract_heading(&value, 2));
            }
        }
    }
    if let Some(results) = report.get("results").and_then(serde_json::Value::as_array) {
        for result in results.iter().filter_map(serde_json::Value::as_object) {
            if result.get("type").and_then(serde_json::Value::as_i64) != Some(1) {
                continue;
            }
            let Some(raw_url) = row_text(result, &["url", "urlPathAndQuery"]) else {
                continue;
            };
            if !is_page_screenshot_url(&raw_url) {
                continue;
            }
            let Some(url) = ensure_seo_page(&mut pages, base_url, result, size) else {
                continue;
            };
            let page = pages.get_mut(&url).expect("SEO result page was inserted");
            fill_if_missing(&mut page.status, row_text(result, &["status"]));
            let canonical = result
                .get("extras")
                .and_then(serde_json::Value::as_object)
                .and_then(|extras| row_text(extras, &["Canonical", "canonical", "canonicalUrl"]));
            fill_if_missing(
                &mut page.canonical,
                canonical.map(|value| report_url(base_url, &value)),
            );
        }
    }
    if let Some(viewport) = size {
        let Some(report_root) = path.parent() else {
            return Ok(pages.into_values().collect());
        };
        let screenshot_dir = report_root.join("screenshots");
        let mut ambiguous_captures = HashSet::<(String, String)>::new();
        for row in report_rows(&report, "browser-screenshots") {
            let Some(raw_url) = row_text(row, &["url"]) else {
                continue;
            };
            if !is_page_screenshot_url(&raw_url) {
                continue;
            }
            let Some(raw_path) = row_text(row, &["path"]) else {
                continue;
            };
            let Some(capture_path) = resolve_screenshot_path(path, &screenshot_dir, &raw_path)
            else {
                continue;
            };
            let Some(capture) = capture_item_for_path(&capture_path, viewport) else {
                continue;
            };
            let Some(url) = ensure_seo_page(&mut pages, base_url, row, size) else {
                continue;
            };
            let key = (url.clone(), viewport.id.clone());
            if ambiguous_captures.contains(&key) {
                continue;
            }
            let page = pages.get_mut(&url).expect("screenshot page was inserted");
            match page.capture_by_size.get(&viewport.id) {
                None => {
                    page.capture_by_size.insert(viewport.id.clone(), capture);
                }
                Some(existing) if existing.path == capture.path => {}
                Some(_) => {
                    page.capture_by_size.remove(&viewport.id);
                    ambiguous_captures.insert(key);
                }
            }
        }
    }
    Ok(pages.into_values().collect())
}

pub(crate) fn collect_report_files(
    directory: &Path,
    allowed_roots: &[PathBuf],
    reports: &mut Vec<PathBuf>,
) -> Result<(), CrawlError> {
    let entries = fs::read_dir(directory).map_err(|error| CrawlError::Io(error.to_string()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !is_path_allowed(&path, allowed_roots) {
            continue;
        }
        if path.is_dir() {
            let hidden = path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name.starts_with('.'));
            if !hidden {
                collect_report_files(&path, allowed_roots, reports)?;
            }
        } else if path.file_name().and_then(|value| value.to_str()) == Some("report.json")
            && is_path_allowed(&path, allowed_roots)
        {
            reports.push(path);
        }
    }
    Ok(())
}

pub(crate) fn list_seo_page_items(
    root: &Path,
    configured: &[CaptureViewport],
    allowed_roots: &[PathBuf],
) -> Result<Vec<SeoPageItem>, CrawlError> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut reports = Vec::new();
    collect_report_files(root, allowed_roots, &mut reports)?;
    reports.sort();
    let mut pages = HashMap::<String, SeoPageItem>::new();
    let mut ambiguous_captures = HashSet::<(String, String)>::new();
    for report_path in reports {
        let Ok(report_pages) = parse_seo_report(&report_path, configured) else {
            continue;
        };
        for incoming in report_pages {
            let page = pages.entry(incoming.url.clone()).or_default();
            if page.url.is_empty() {
                page.url = incoming.url.clone();
            }
            for (target, value) in [
                (&mut page.title, incoming.title),
                (&mut page.description, incoming.description),
                (&mut page.canonical, incoming.canonical),
                (&mut page.og_title, incoming.og_title),
                (&mut page.og_description, incoming.og_description),
                (&mut page.og_image, incoming.og_image),
                (&mut page.twitter_title, incoming.twitter_title),
                (&mut page.twitter_description, incoming.twitter_description),
                (&mut page.twitter_image, incoming.twitter_image),
                (&mut page.h1, incoming.h1),
                (&mut page.h2, incoming.h2),
                (&mut page.status, incoming.status),
            ] {
                fill_if_missing(target, value);
            }
            for id in incoming.size_ids {
                if !page.size_ids.iter().any(|known| known == &id) {
                    page.size_ids.push(id);
                }
            }
            for label in incoming.size_labels {
                if !page.size_labels.iter().any(|known| known == &label) {
                    page.size_labels.push(label);
                }
            }
            for (size_id, capture) in incoming.capture_by_size {
                let key = (incoming.url.clone(), size_id.clone());
                if ambiguous_captures.contains(&key) {
                    continue;
                }
                match page.capture_by_size.get(&size_id) {
                    None => {
                        page.capture_by_size.insert(size_id, capture);
                    }
                    Some(existing) if existing.path == capture.path => {}
                    Some(_) => {
                        page.capture_by_size.remove(&size_id);
                        ambiguous_captures.insert(key);
                    }
                }
            }
        }
    }
    let mut result = pages.into_values().collect::<Vec<_>>();
    result.sort_by(|left, right| left.url.cmp(&right.url));
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CrawlConfiguration;

    #[test]
    fn relative_non_page_screenshots_are_removed_and_outside_files_preserved() {
        let root = std::env::temp_dir().join(format!("maho-relative-{}", std::process::id()));
        let shots = root.join("screenshots");
        fs::create_dir_all(&shots).unwrap();
        fs::write(shots.join("font.png"), b"image").unwrap();
        fs::write(root.join("outside.png"), b"keep").unwrap();
        let report = root.join("report.json");
        fs::write(
            &report,
            serde_json::to_vec(
                &serde_json::json!({"tables": {"browser-screenshots": {"rows": [
                    {"url":"https://example.com/font.woff2", "path":"screenshots/font.png"},
                    {"url":"https://example.com/font.woff2", "path":"outside.png"}
                ]}}}),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(filter_non_page_screenshots(&report, &shots).unwrap(), 1);
        assert!(root.join("outside.png").exists());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn heading_entities_are_decoded_once() {
        assert_eq!(
            normalize_heading_text("&amp;lt; &lt; &#65; &#x1F600; &unknown;"),
            "&lt; < A 😀 &unknown;"
        );
    }
    #[test]
    fn capture_listing_is_recursive_but_ignores_non_images() {
        let root = std::env::temp_dir().join(format!("maho-crawl-captures-{}", std::process::id()));
        let nested = root.join("desktop-1440x900/screenshots");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("one.png"), [0_u8, 1]).unwrap();
        std::fs::write(nested.join("two.webp"), [0_u8, 1, 2]).unwrap();
        let hidden = root.join(".siteone-http-cache");
        std::fs::create_dir_all(&hidden).unwrap();
        std::fs::write(hidden.join("cached.png"), [0_u8, 1, 2]).unwrap();
        std::fs::write(nested.join("ignore.txt"), [0_u8]).unwrap();
        let mut config = CrawlConfiguration::default();
        config.captures.truncate(1);
        let items = list_capture_items(&root, &config.captures).unwrap();
        assert_eq!(
            items
                .iter()
                .map(|item| item.filename.as_str())
                .collect::<Vec<_>>(),
            vec!["one.png", "two.webp"]
        );
        assert_eq!(items[0].size_id.as_deref(), Some("desktop"));
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn page_screenshot_url_filter_excludes_known_static_resources() {
        assert!(is_page_screenshot_url("https://example.test/about"));
        assert!(is_page_screenshot_url("https://example.test/about.html"));
        assert!(is_page_screenshot_url(
            "https://example.test/about?view=mobile"
        ));
        assert!(!is_page_screenshot_url(
            "https://example.test/assets/fonts/site.woff2?v=1"
        ));
        assert!(!is_page_screenshot_url(
            "https://example.test/assets/images/logo.svg"
        ));
        assert!(!is_page_screenshot_url(
            "https://example.test/assets/app.js?v=1"
        ));
    }
    #[test]
    fn non_page_screenshot_filter_removes_only_reported_static_resources() {
        let root = std::env::temp_dir().join(format!(
            "maho-crawl-screenshot-filter-{}",
            std::process::id()
        ));
        let screenshot_dir = root.join("screenshots");
        let report_path = root.join("report.json");
        std::fs::create_dir_all(&screenshot_dir).unwrap();
        let page = screenshot_dir.join("page.png");
        let font = screenshot_dir.join("font.png");
        let unknown = screenshot_dir.join("unknown.png");
        std::fs::write(&page, [0_u8, 1]).unwrap();
        std::fs::write(&font, [0_u8, 1]).unwrap();
        std::fs::write(&unknown, [0_u8, 1]).unwrap();
        let report = serde_json::json!({
            "tables": {
                "browser-screenshots": {
                    "rows": [
                        { "path": page.to_string_lossy(), "url": "https://example.test/about" },
                        { "path": font.to_string_lossy(), "url": "https://example.test/assets/site.woff2" },
                        { "path": unknown.to_string_lossy(), "url": "https://example.test/assets/custom.page" }
                    ]
                }
            }
        });
        std::fs::write(&report_path, serde_json::to_vec(&report).unwrap()).unwrap();

        assert_eq!(
            filter_non_page_screenshots(&report_path, &screenshot_dir).unwrap(),
            1
        );
        assert!(page.exists());
        assert!(!font.exists());
        assert!(unknown.exists());
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn seo_report_parser_merges_sizes_and_extracts_safe_standard_fields() {
        let root = std::env::temp_dir().join(format!("maho-crawl-seo-{}", std::process::id()));
        let desktop = root.join("desktop-1440x900");
        let tablet = root.join("tablet-768x1024");
        std::fs::create_dir_all(&desktop).unwrap();
        std::fs::create_dir_all(&tablet).unwrap();
        let report = |title: &str| {
            serde_json::json!({
                "options": { "url": "https://example.test/" },
            "tables": {
                    "seo": { "rows": [{
                        "urlPathAndQuery": "/about?from=report",
                        "title": title,
                        "description": "Description",
                        "h1": "Heading one",
                        "indexing": "Allowed"
                    }] },
                    "open-graph": { "rows": [{
                        "urlPathAndQuery": "/about?from=report",
                        "ogTitle": "OG title",
                        "ogDescription": "OG description",
                        "ogImage": "https://example.test/og.png",
                        "twitterTitle": "Twitter title"
                    }] },
                    "seo-headings": { "rows": [{
                        "urlPathAndQuery": "/about?from=report",
                        "headings": "<h1> Heading one <h2> Heading two <script>ignored</script>"
                    }] }
                },
                "results": [{
                    "url": "https://example.test/about?from=report",
                    "type": 1,
                    "status": "200",
                    "extras": { "Canonical": "https://example.test/about" }
                }, {
                    "url": "https://example.test/not-in-standard-tables",
                    "type": 1,
                    "status": "204"
                }, {
                    "url": "https://example.test/assets/site.woff2",
                    "type": 1,
                    "status": "404"
                }]
            })
        };
        std::fs::write(
            desktop.join("report.json"),
            serde_json::to_vec(&report("Desktop title")).unwrap(),
        )
        .unwrap();
        std::fs::write(
            tablet.join("report.json"),
            serde_json::to_vec(&report("Tablet title")).unwrap(),
        )
        .unwrap();

        let pages = list_seo_page_items(
            &root,
            &CrawlConfiguration::default().captures,
            std::slice::from_ref(&root),
        )
        .unwrap();
        assert_eq!(pages.len(), 2);
        let page = pages
            .iter()
            .find(|page| page.url.ends_with("/about?from=report"))
            .unwrap();
        assert_eq!(page.url, "https://example.test/about?from=report");
        assert_eq!(page.title.as_deref(), Some("Desktop title"));
        assert_eq!(page.description.as_deref(), Some("Description"));
        assert_eq!(
            page.canonical.as_deref(),
            Some("https://example.test/about")
        );
        assert_eq!(page.og_title.as_deref(), Some("OG title"));
        assert_eq!(page.twitter_title.as_deref(), Some("Twitter title"));
        assert_eq!(page.h1.as_deref(), Some("Heading one"));
        assert_eq!(page.h2.as_deref(), Some("Heading two"));
        assert_eq!(page.size_ids, vec!["desktop", "tablet"]);
        assert_eq!(page.status.as_deref(), Some("Allowed"));
        let tableless_page = pages
            .iter()
            .find(|page| page.url.ends_with("/not-in-standard-tables"))
            .unwrap();
        assert_eq!(tableless_page.title, None);
        assert_eq!(tableless_page.status.as_deref(), Some("204"));
        assert_eq!(tableless_page.size_ids, vec!["desktop", "tablet"]);
        assert!(!pages
            .iter()
            .any(|page| page.url.ends_with("/assets/site.woff2")));
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn seo_report_parser_maps_page_captures_by_viewport_id_and_serializes_contract() {
        let root =
            std::env::temp_dir().join(format!("maho-crawl-seo-captures-{}", std::process::id()));
        let wide = CaptureViewport::new("wide", "Same label", 1440, 900);
        let narrow = CaptureViewport::new("narrow", "Same label", 768, 1024);
        let configured = vec![wide.clone(), narrow.clone()];
        let wide_root = root.join(safe_size_slug(&wide));
        let narrow_root = root.join(safe_size_slug(&narrow));
        let wide_screenshots = wide_root.join("screenshots");
        let narrow_screenshots = narrow_root.join("screenshots");
        std::fs::create_dir_all(&wide_screenshots).unwrap();
        std::fs::create_dir_all(&narrow_screenshots).unwrap();

        let wide_about = wide_screenshots.join("about-wide.png");
        let wide_child = wide_screenshots.join("child-wide.jpg");
        let narrow_about = narrow_screenshots.join("about-narrow.webp");
        let narrow_child = narrow_screenshots.join("child-narrow.png");
        let outside = root.join("outside.png");
        let static_resource = wide_screenshots.join("font.png");
        std::fs::write(&wide_about, [1_u8, 2, 3]).unwrap();
        std::fs::write(&wide_child, [4_u8, 5]).unwrap();
        std::fs::write(&narrow_about, [6_u8, 7, 8, 9]).unwrap();
        std::fs::write(&narrow_child, [10_u8]).unwrap();
        std::fs::write(&outside, [11_u8]).unwrap();
        std::fs::write(&static_resource, [12_u8]).unwrap();

        let report = |title: &str, screenshot_rows: Vec<serde_json::Value>| {
            serde_json::json!({
                "options": { "url": "https://example.test:8443/base/" },
                "tables": {
                    "seo": { "rows": [
                        {
                            "urlPathAndQuery": "/parent/about?from=report",
                            "title": title,
                            "indexing": "Allowed"
                        },
                        {
                            "urlPathAndQuery": "/parent/child?from=report",
                            "title": "Child title"
                        }
                    ] },
                    "browser-screenshots": { "rows": screenshot_rows }
                },
                "results": [{
                    "url": "https://example.test:8443/parent/child?from=report",
                    "type": 1,
                    "status": "200"
                }]
            })
        };
        let wide_report = report(
            "Wide title",
            vec![
                serde_json::json!({
                    "url": "https://example.test:8443/parent/about?from=report",
                    "path": "screenshots/about-wide.png"
                }),
                serde_json::json!({
                    "url": "https://example.test:8443/parent/child?from=report",
                    "path": wide_child.to_string_lossy()
                }),
                serde_json::json!({
                    "url": "https://example.test:8443/assets/site.woff2",
                    "path": static_resource.to_string_lossy()
                }),
                serde_json::json!({
                    "url": "https://example.test:8443/parent/about?from=report",
                    "path": "../outside.png"
                }),
                serde_json::json!({
                    "url": "https://example.test:8443/parent/child?from=report",
                    "path": "screenshots/missing.png"
                }),
            ],
        );
        let narrow_report = report(
            "Narrow title",
            vec![
                serde_json::json!({
                    "url": "https://example.test:8443/parent/about?from=report",
                    "path": narrow_about.to_string_lossy()
                }),
                serde_json::json!({
                    "url": "https://example.test:8443/parent/child?from=report",
                    "path": narrow_child.to_string_lossy()
                }),
            ],
        );
        std::fs::write(
            wide_root.join("report.json"),
            serde_json::to_vec(&wide_report).unwrap(),
        )
        .unwrap();
        std::fs::write(
            narrow_root.join("report.json"),
            serde_json::to_vec(&narrow_report).unwrap(),
        )
        .unwrap();

        let pages = list_seo_page_items(&root, &configured, std::slice::from_ref(&root)).unwrap();
        assert_eq!(pages.len(), 2);
        let about = pages
            .iter()
            .find(|page| page.url.ends_with("/parent/about?from=report"))
            .unwrap();
        let child = pages
            .iter()
            .find(|page| page.url.ends_with("/parent/child?from=report"))
            .unwrap();
        assert_eq!(
            about.url,
            "https://example.test:8443/parent/about?from=report"
        );
        assert_eq!(about.size_ids, vec!["wide", "narrow"]);
        assert_eq!(about.size_labels, vec!["Same label"]);
        assert_eq!(about.capture_by_size.len(), 2);
        assert_eq!(about.capture_by_size["wide"].filename, "about-wide.png");
        assert_eq!(
            about.capture_by_size["narrow"].filename,
            "about-narrow.webp"
        );
        assert_eq!(child.capture_by_size.len(), 2);
        assert_eq!(child.capture_by_size["wide"].filename, "child-wide.jpg");
        assert_eq!(child.capture_by_size["narrow"].filename, "child-narrow.png");
        assert_eq!(
            child.capture_by_size["wide"].size_label.as_deref(),
            Some("Same label")
        );
        assert!(!pages
            .iter()
            .any(|page| page.url.ends_with("/assets/site.woff2")));

        let serialized = serde_json::to_value(about).unwrap();
        let captures = serialized
            .get("captureBySize")
            .and_then(serde_json::Value::as_object)
            .unwrap();
        assert_eq!(captures["wide"]["sizeId"], "wide");
        assert_eq!(captures["wide"]["sizeLabel"], "Same label");
        assert_eq!(captures["wide"]["width"], 1440);
        assert_eq!(captures["narrow"]["sizeId"], "narrow");
        assert_eq!(captures["narrow"]["height"], 1024);

        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn thumbnail_encoder_reduces_dimensions_and_returns_jpeg() {
        let root =
            std::env::temp_dir().join(format!("maho-crawl-thumbnail-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let image_path = root.join("large.png");
        let image = image::RgbImage::from_pixel(1200, 900, image::Rgb([40, 80, 120]));
        image.save(&image_path).unwrap();
        let thumbnail = encode_thumbnail(&image_path).unwrap();
        assert_eq!(thumbnail.mime_type, "image/jpeg");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(thumbnail.data_base64)
            .unwrap();
        let decoded = image::load_from_memory(&bytes).unwrap();
        assert!(decoded.width() <= 480);
        assert!(decoded.height() <= 320);
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn ansi_cleaner_removes_siteone_version_color_codes() {
        assert_eq!(
            clean_ansi("\u{1b}[0;34mVersion: 2.5.1\u{1b}[0m"),
            "Version: 2.5.1"
        );
    }
}
