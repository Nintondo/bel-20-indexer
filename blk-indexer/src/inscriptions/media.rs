use std::str::FromStr;

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum Media {
    Audio,
    Iframe,
    Image,
    Pdf,
    Text,
    Unknown,
    Video,
}

impl Media {
    const TABLE: &'static [(&'static str, Media, &'static [&'static str])] = &[
        ("application/json", Media::Text, &["json"]),
        ("application/json; charset=utf-8", Media::Text, &["json"]),
        ("application/json;charset=utf-8", Media::Text, &["json"]),
        ("application/pdf", Media::Pdf, &["pdf"]),
        ("application/pgp-signature", Media::Text, &["asc"]),
        ("application/yaml", Media::Text, &["yaml", "yml"]),
        ("audio/flac", Media::Audio, &["flac"]),
        ("audio/mpeg", Media::Audio, &["mp3"]),
        ("audio/wav", Media::Audio, &["wav"]),
        ("image/apng", Media::Image, &["apng"]),
        ("image/avif", Media::Image, &[]),
        ("image/gif", Media::Image, &["gif"]),
        ("image/jpeg", Media::Image, &["jpg", "jpeg"]),
        ("image/png", Media::Image, &["png"]),
        ("image/svg+xml", Media::Iframe, &["svg"]),
        ("image/webp", Media::Image, &["webp"]),
        ("model/gltf-binary", Media::Unknown, &["glb"]),
        ("model/stl", Media::Unknown, &["stl"]),
        ("text/html;charset=utf-8", Media::Iframe, &["html"]),
        ("text/html; charset=utf-8", Media::Iframe, &["html"]),
        ("text/plain;charset=utf-8", Media::Text, &["txt"]),
        ("text/plain; charset=utf-8", Media::Text, &["txt"]),
        ("text/plain", Media::Text, &["txt"]),
        ("video/mp4", Media::Video, &["mp4"]),
        ("video/webm", Media::Video, &["webm"]),
    ];
}

impl FromStr for Media {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        for entry in Self::TABLE {
            if entry.0 == s {
                return Ok(entry.1);
            }
        }

        anyhow::bail!("unknown content type: {s}");
    }
}
