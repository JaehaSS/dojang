//! Bounded public tool outputs. Never serialize entire raw response/connector envelopes.
use base64::Engine;
use serde::Serialize;
use serde_json::Value;
use std::io::Read;
use std::path::Path;

const MAX_MEDIA: usize = 8 * 1024 * 1024;
const MAX_TEXT: usize = 16_000;
const MAX_CONTENTS: usize = 32;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Content {
    Text { text: String },
    Image { url: String },
    Audio { url: String },
    Resource { uri: String, title: String },
}

fn text(value: &str) -> Content {
    let mut s: String = value.chars().take(MAX_TEXT).collect();
    if value.chars().count() > MAX_TEXT {
        s.push_str("\n… (출력 일부 생략)");
    }
    Content::Text { text: s }
}

fn media(url: &str) -> Option<Content> {
    let (header, data) = url.split_once(',')?;
    if data.len() > MAX_MEDIA * 4 / 3 + 4 {
        return None;
    }
    let mime = header.strip_prefix("data:")?.strip_suffix(";base64")?;
    let image = matches!(
        mime,
        "image/png" | "image/jpeg" | "image/webp" | "image/gif"
    );
    let audio = matches!(
        mime,
        "audio/wav" | "audio/mpeg" | "audio/ogg" | "audio/webm"
    );
    if !image && !audio {
        return None;
    }
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(data)
        .ok()?;
    if decoded.is_empty() || decoded.len() > MAX_MEDIA {
        return None;
    }
    Some(if image {
        Content::Image { url: url.into() }
    } else {
        Content::Audio { url: url.into() }
    })
}

fn resource(uri: &str, title: &str) -> Option<Content> {
    if uri.len() > 8192 || uri.chars().any(char::is_control) {
        return None;
    }
    if uri.starts_with("https://")
        || uri.starts_with("http://")
        || uri.starts_with("file:///")
        || Path::new(uri).is_absolute()
    {
        Some(Content::Resource {
            uri: uri.into(),
            title: title.chars().take(200).collect(),
        })
    } else {
        None
    }
}

/// Only an already-emitted tool path is read, with a bounded allocation. Arbitrary
/// remote media URLs are links, never fetched by Dojang or its transcript renderer.
pub(super) fn local_media(path: &str) -> Option<Content> {
    let p = Path::new(path);
    if !p.is_absolute() {
        return None;
    }
    let mime = match p.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "wav" => "audio/wav",
        "mp3" => "audio/mpeg",
        "ogg" => "audio/ogg",
        "webm" => "audio/webm",
        _ => return resource(path, "결과 파일"),
    };
    if !std::fs::metadata(p).ok()?.is_file() {
        return None;
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    // A provider may return a special file (or replace a path between stat/open).
    // Do not let a FIFO block the event reader and prevent cancellation.
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(nix::libc::O_NONBLOCK);
    }
    let file = options.open(p).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    file.take((MAX_MEDIA + 1) as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > MAX_MEDIA {
        return resource(path, "큰 결과 파일 열기");
    }
    media(&format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

pub(super) fn contents(value: &Value) -> Vec<Content> {
    let mut result = Vec::new();
    if let Some(s) = value.as_str() {
        result.push(text(s));
    } else if let Some(items) = value.as_array() {
        let mut media_bytes = 0;
        for item in items.iter().take(MAX_CONTENTS) {
            let kind = item["type"].as_str().unwrap_or_default();
            let content = match kind {
                "text" | "inputText" | "input_text" => item["text"].as_str().map(text),
                "image" | "audio" => {
                    let mime = item["mimeType"].as_str().unwrap_or_default();
                    item["data"]
                        .as_str()
                        .and_then(|s| media(&format!("data:{mime};base64,{s}")))
                }
                "inputImage" | "input_image" | "inputAudio" | "input_audio" => {
                    let url = item["image_url"]
                        .as_str()
                        .or_else(|| item["imageUrl"].as_str())
                        .or_else(|| item["audio_url"].as_str())
                        .or_else(|| item["audioUrl"].as_str());
                    url.and_then(|u| media(u).or_else(|| resource(u, "도구 결과")))
                }
                "resource_link" => item["uri"]
                    .as_str()
                    .and_then(|u| resource(u, item["name"].as_str().unwrap_or("결과 파일"))),
                "resource" => item["resource"]["text"].as_str().map(text).or_else(|| {
                    item["resource"]["uri"]
                        .as_str()
                        .and_then(|u| resource(u, "결과 리소스"))
                }),
                // Encrypted content, tool definitions and provider metadata stay private.
                _ => None,
            };
            if let Some(content) = content {
                if let Content::Image { url } | Content::Audio { url } = &content {
                    media_bytes += url.len();
                    if media_bytes > MAX_MEDIA * 4 / 3 + 4 {
                        result.push(text("추가 미디어 결과가 표시 한도를 초과했습니다."));
                        break;
                    }
                }
                result.push(content);
            }
        }
    }
    result
}

pub(super) fn for_item(item: &Value) -> Vec<Content> {
    match item["type"].as_str() {
        Some("mcpToolCall") => contents(&item["result"]["content"]),
        Some("functionCallOutput" | "function_call_output" | "custom_tool_call_output") => {
            contents(&item["output"])
        }
        Some("imageView") => item["path"]
            .as_str()
            .and_then(local_media)
            .into_iter()
            .collect(),
        Some("imageGeneration") => {
            if let Some(path) = item["savedPath"].as_str() {
                if let Some(content) = local_media(path) {
                    return vec![content];
                }
            }
            item["result"]
                .as_str()
                .and_then(|data| {
                    if data.starts_with("data:") {
                        media(data)
                    } else {
                        media(&format!("data:image/png;base64,{data}"))
                    }
                })
                .into_iter()
                .collect()
        }
        _ => Vec::new(),
    }
}

pub(super) fn summary(item: &Value) -> String {
    let contents = for_item(item);
    let texts: Vec<_> = contents
        .iter()
        .filter_map(|c| match c {
            Content::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    if texts.is_empty() {
        item["error"]["message"]
            .as_str()
            .or_else(|| item["failure"]["message"].as_str())
            .unwrap_or("작업 종료")
            .chars()
            .take(2000)
            .collect()
    } else {
        texts.join("\n").chars().take(2000).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn maps_public_text_image_audio_and_safe_links_only() {
        let got = contents(&json!([
            {"type":"input_text","text":"42"},
            {"type":"input_image","image_url":"data:image/png;base64,AQ=="},
            {"type":"input_audio","audio_url":"data:audio/wav;base64,AQ=="},
            {"type":"encrypted_content","encrypted_content":"NEVER-SHOW"},
            {"type":"input_image","image_url":"javascript:alert(1)"},
            {"type":"input_image","image_url":"https://example.com/result.png"}
        ]));
        assert_eq!(got.len(), 4);
        assert!(matches!(got[0], Content::Text { .. }));
        assert!(matches!(got[1], Content::Image { .. }));
        assert!(matches!(got[2], Content::Audio { .. }));
        assert!(matches!(got[3], Content::Resource { .. }));
        assert!(!serde_json::to_string(&got).unwrap().contains("NEVER-SHOW"));
    }
    #[test]
    fn rejects_active_mime_invalid_base64_and_bounds_text() {
        assert!(media("data:image/svg+xml;base64,AQ==").is_none());
        assert!(media("data:text/html;base64,AQ==").is_none());
        assert!(media("data:image/png;base64,%%%").is_none());
        let got = contents(&json!("한".repeat(MAX_TEXT + 10)));
        let Content::Text { text } = &got[0] else {
            panic!()
        };
        assert!(text.ends_with("(출력 일부 생략)"));
    }
    #[test]
    fn image_generation_and_mcp_errors_are_visible() {
        assert!(matches!(
            &for_item(&json!({"type":"imageGeneration","result":"AQ=="}))[0],
            Content::Image { .. }
        ));
        assert_eq!(
            summary(&json!({"type":"mcpToolCall","error":{"message":"connection denied"}})),
            "connection denied"
        );
    }
    #[cfg(unix)]
    #[test]
    fn local_media_rejects_a_fifo_without_opening_a_blocking_reader() {
        let path = std::env::temp_dir().join(format!(
            "dojang-media-{}.png",
            super::super::interaction::id().unwrap()
        ));
        let cpath = std::ffi::CString::new(path.to_str().unwrap()).unwrap();
        assert_eq!(unsafe { nix::libc::mkfifo(cpath.as_ptr(), 0o600) }, 0);
        let result = local_media(path.to_str().unwrap());
        std::fs::remove_file(path).unwrap();
        assert!(result.is_none());
    }
}
