//! Text extraction from Slack file attachments and linked URLs.
//!
//! This module provides two public async functions used by the converter:
//! - [`extract_files`] — downloads and extracts text from Slack file attachments
//! - [`link_titles`] — fetches `<title>` tags from URLs embedded in Slack message text

use std::io::{Cursor, Read};
use std::sync::LazyLock;

use anyhow::{Context, Result};
use regex::Regex;
use tracing::warn;

use crate::slack::types::SlackFile;

// 10 MB upper bound for file downloads
const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;

/// Regex for Slack-formatted URLs: `<https://...>` or `<https://...|label>`.
///
/// SAFETY: pattern is a compile-time literal verified to be valid; Regex::new
/// cannot panic for this input at runtime.
static SLACK_URL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"<(https?://[^|>]+)(?:\|[^>]*)?>")
        .expect("SLACK_URL_RE pattern is a compile-time literal and is always valid")
});

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Download and extract text from every file in `files`.
///
/// Returns a newline-prefixed string of all extracted pieces joined with
/// newlines, or an empty string when `files` is empty or all files are skipped.
pub async fn extract_files(files: &[SlackFile], user_token: &str) -> String {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_default();

    let mut parts: Vec<String> = Vec::new();

    for file in files {
        let url = match file.url_private.as_deref() {
            Some(u) => u,
            None => continue,
        };
        let name = file.name.as_deref().unwrap_or("attachment");

        // External files (Google Docs, etc.) — link only, do not attempt download
        if file.external_type.is_some() {
            parts.push(format!("--- Attachment: {name} ---\n{url}"));
            continue;
        }

        // Files larger than the limit — link only
        if file.size.unwrap_or(0) > MAX_FILE_BYTES {
            warn!(name, "skipping oversized file attachment");
            parts.push(format!("--- Attachment: {name} ---\n{url}"));
            continue;
        }

        let filetype = file.filetype.as_deref().unwrap_or("");

        match extract_file_content(&client, url, user_token, filetype, name).await {
            Ok(text) => parts.push(format!("--- Attachment: {name} ---\n{text}")),
            Err(e) => {
                warn!(name, error = %e, "failed to extract file, falling back to URL");
                parts.push(format!("--- Attachment: {name} ---\n{url}"));
            }
        }
    }

    if parts.is_empty() {
        String::new()
    } else {
        format!("\n{}", parts.join("\n"))
    }
}

/// Fetch `<title>` tags from all non-Slack URLs found in `text`.
///
/// Returns a formatted `"\n--- Links ---\n..."` block, or an empty string
/// when no URLs are found or no titles are retrievable.
pub async fn link_titles(text: &str) -> String {
    let urls = extract_urls(text);
    if urls.is_empty() {
        return String::new();
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_default();

    let mut lines: Vec<String> = Vec::new();

    for url in &urls {
        if let Some(title) = fetch_title(&client, url).await {
            lines.push(format!("{url}: {title}"));
        }
    }

    if lines.is_empty() {
        String::new()
    } else {
        format!("\n--- Links ---\n{}", lines.join("\n"))
    }
}

// ---------------------------------------------------------------------------
// URL extraction (pub(crate) so tests in this module can reach it)
// ---------------------------------------------------------------------------

/// Extract unique, non-Slack HTTPS URLs from Slack-formatted message text.
///
/// Slack encodes links as `<https://example.com>` or `<https://example.com|label>`.
/// URLs containing `.slack.com/` are excluded.
pub(crate) fn extract_urls(text: &str) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();

    for capture in SLACK_URL_RE.captures_iter(text) {
        let url = capture[1].to_string();
        if url.contains(".slack.com/") {
            continue;
        }
        if !seen.contains(&url) {
            seen.push(url);
        }
    }

    seen
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Download and extract text from a single file, dispatching on `filetype`.
async fn extract_file_content(
    client: &reqwest::Client,
    url: &str,
    token: &str,
    filetype: &str,
    name: &str,
) -> Result<String> {
    match filetype {
        "text" | "csv" | "markdown" | "txt" | "md" => {
            let data = download(client, url, token)
                .await
                .with_context(|| format!("downloading {name}"))?;
            String::from_utf8(data).with_context(|| format!("decoding {name} as UTF-8"))
        }
        "docx" => {
            let data = download(client, url, token)
                .await
                .with_context(|| format!("downloading {name}"))?;
            extract_docx(&data).with_context(|| format!("extracting docx {name}"))
        }
        "xlsx" => {
            let data = download(client, url, token)
                .await
                .with_context(|| format!("downloading {name}"))?;
            extract_xlsx(&data).with_context(|| format!("extracting xlsx {name}"))
        }
        "pptx" => {
            let data = download(client, url, token)
                .await
                .with_context(|| format!("downloading {name}"))?;
            extract_pptx(&data).with_context(|| format!("extracting pptx {name}"))
        }
        "pdf" => {
            let data = download(client, url, token)
                .await
                .with_context(|| format!("downloading {name}"))?;
            extract_pdf(&data).with_context(|| format!("extracting pdf {name}"))
        }
        _ => {
            // Unsupported type — caller will format as URL
            anyhow::bail!("unsupported filetype: {filetype}")
        }
    }
}

/// Strip XML/HTML tags and collapse whitespace.
fn xml_to_text(xml: &str) -> String {
    let mut output = String::with_capacity(xml.len());
    let mut inside_tag = false;

    for ch in xml.chars() {
        match ch {
            '<' => inside_tag = true,
            '>' => inside_tag = false,
            _ if !inside_tag => output.push(ch),
            _ => {}
        }
    }

    output.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Extract text from a `.docx` file (ZIP containing `word/document.xml`).
fn extract_docx(data: &[u8]) -> Result<String> {
    let cursor = Cursor::new(data);
    let mut archive = zip::ZipArchive::new(cursor).context("opening docx as zip")?;

    let mut xml_file = archive
        .by_name("word/document.xml")
        .context("word/document.xml not found in docx")?;

    let mut xml = String::new();
    xml_file
        .read_to_string(&mut xml)
        .context("reading word/document.xml")?;

    Ok(xml_to_text(&xml))
}

/// Extract text from a `.pptx` file (ZIP containing `ppt/slides/slide*.xml`).
///
/// Uses a two-pass approach (collect slide names first, then read each) to
/// avoid simultaneous borrows on the `ZipArchive`.
fn extract_pptx(data: &[u8]) -> Result<String> {
    let cursor = Cursor::new(data);
    let mut archive = zip::ZipArchive::new(cursor).context("opening pptx as zip")?;

    // First pass: collect indices of slide entries
    let slide_indices: Vec<usize> = (0..archive.len())
        .filter(|&i| {
            archive
                .by_index(i)
                .map(|f| {
                    let n = f.name().to_string();
                    n.starts_with("ppt/slides/slide") && n.ends_with(".xml")
                })
                .unwrap_or(false)
        })
        .collect();

    // Second pass: read each slide
    let mut slide_texts: Vec<String> = Vec::new();
    for idx in slide_indices {
        let mut slide_file = archive
            .by_index(idx)
            .with_context(|| format!("reading pptx slide at index {idx}"))?;
        let mut xml = String::new();
        slide_file
            .read_to_string(&mut xml)
            .with_context(|| format!("reading pptx slide xml at index {idx}"))?;
        slide_texts.push(xml_to_text(&xml));
    }

    Ok(slide_texts.join("\n"))
}

/// Extract text from an `.xlsx` file using `calamine`.
fn extract_xlsx(data: &[u8]) -> Result<String> {
    use calamine::{Data, Reader, Xlsx};

    let cursor = Cursor::new(data);
    let mut workbook: Xlsx<_> = Xlsx::new(cursor).context("opening xlsx with calamine")?;

    let sheet_names = workbook.sheet_names().to_vec();
    let mut rows_text: Vec<String> = Vec::new();

    for sheet_name in &sheet_names {
        let range = workbook
            .worksheet_range(sheet_name)
            .with_context(|| format!("reading sheet {sheet_name}"))?;

        for row in range.rows() {
            let cells: Vec<String> = row
                .iter()
                .filter(|cell| !matches!(cell, Data::Empty))
                .map(|cell| cell.to_string())
                .collect();
            if !cells.is_empty() {
                rows_text.push(cells.join("\t"));
            }
        }
    }

    Ok(rows_text.join("\n"))
}

/// Extract text from a PDF using `pdf-extract`.
fn extract_pdf(data: &[u8]) -> Result<String> {
    pdf_extract::extract_text_from_mem(data).map_err(|e| anyhow::anyhow!("{e}"))
}

/// Download bytes from `url` using a Bearer token, with a 30-second timeout.
async fn download(client: &reqwest::Client, url: &str, token: &str) -> Result<Vec<u8>> {
    let response = client
        .get(url)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;

    let bytes = response
        .bytes()
        .await
        .with_context(|| format!("reading response body from {url}"))?;

    Ok(bytes.to_vec())
}

/// Fetch the `<title>` of a web page, returning `None` on any error or missing title.
async fn fetch_title(client: &reqwest::Client, url: &str) -> Option<String> {
    let response = client.get(url).send().await.ok()?;

    if !response.status().is_success() {
        return None;
    }

    let body = response.text().await.ok()?;
    let document = scraper::Html::parse_document(&body);

    // SAFETY: "title" is a valid CSS selector literal; Selector::parse cannot
    // fail for this input.
    let selector =
        scraper::Selector::parse("title").expect("\"title\" is a valid CSS selector literal");

    let title = document
        .select(&selector)
        .next()?
        .text()
        .collect::<String>();

    let trimmed = title.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_zip_with_entry(entry_name: &str, content: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut buf = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut buf);
            let mut zip = zip::ZipWriter::new(cursor);
            zip.start_file(entry_name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(content).unwrap();
            zip.finish().unwrap();
        }
        buf
    }

    // --- xml_to_text ---------------------------------------------------------

    #[test]
    fn test_xml_to_text_strips_tags() {
        let input = "<w:document><w:t>Hello</w:t></w:document>";
        assert_eq!(xml_to_text(input), "Hello");
    }

    #[test]
    fn test_xml_to_text_collapses_whitespace() {
        let input = "<tag>   foo    bar   </tag>";
        assert_eq!(xml_to_text(input), "foo bar");
    }

    #[test]
    fn test_xml_to_text_empty_input() {
        assert_eq!(xml_to_text(""), "");
        assert_eq!(xml_to_text("<tag></tag>"), "");
    }

    // --- extract_docx --------------------------------------------------------

    #[test]
    fn test_extract_docx_returns_text() {
        let xml = b"<w:document><w:t>Test</w:t></w:document>";
        let zip_bytes = make_zip_with_entry("word/document.xml", xml);
        let result = extract_docx(&zip_bytes).unwrap();
        assert_eq!(result, "Test");
    }

    #[test]
    fn test_extract_docx_missing_entry_returns_err() {
        let zip_bytes = make_zip_with_entry("other.xml", b"content");
        assert!(extract_docx(&zip_bytes).is_err());
    }

    // --- extract_pptx --------------------------------------------------------

    #[test]
    fn test_extract_pptx_returns_text() {
        let xml = b"<p:sld><a:t>Slide text</a:t></p:sld>";
        let zip_bytes = make_zip_with_entry("ppt/slides/slide1.xml", xml);
        let result = extract_pptx(&zip_bytes).unwrap();
        assert_eq!(result, "Slide text");
    }

    #[test]
    fn test_extract_pptx_ignores_non_slide_entries() {
        let xml = b"<root><t>Should be ignored</t></root>";
        let zip_bytes = make_zip_with_entry("ppt/notaslide.xml", xml);
        let result = extract_pptx(&zip_bytes).unwrap();
        assert_eq!(result, "");
    }

    // --- extract_urls --------------------------------------------------------

    #[test]
    fn test_extract_urls_basic() {
        let urls = extract_urls("Check <https://example.com>");
        assert_eq!(urls, vec!["https://example.com"]);
    }

    #[test]
    fn test_extract_urls_with_label() {
        let urls = extract_urls("<https://example.com|Example>");
        assert_eq!(urls, vec!["https://example.com"]);
    }

    #[test]
    fn test_extract_urls_skips_slack_links() {
        let urls = extract_urls("<https://myworkspace.slack.com/archives/C001>");
        assert!(urls.is_empty());
    }

    #[test]
    fn test_extract_urls_deduplicates() {
        let urls = extract_urls("<https://example.com> and again <https://example.com>");
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0], "https://example.com");
    }

    #[test]
    fn test_extract_urls_empty_text() {
        let urls = extract_urls("no links here");
        assert!(urls.is_empty());
    }
}
