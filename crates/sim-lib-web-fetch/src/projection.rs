use crate::{FetchError, RepresentationOutcome};
use sim_codec_doc::{HtmlDecodeOptions, Inline, MarkupBlock, decode_html_bytes};
use sim_lib_web_core::{RepresentationMetadata, WebCapture, WebRepresentation};

pub(crate) fn project(
    raw: &WebCapture,
    version: &str,
    media: Option<&str>,
) -> Result<(Option<WebRepresentation>, RepresentationOutcome), FetchError> {
    let (text, codec, warnings) = match media {
        Some("text/html") | Some("application/xhtml+xml") => {
            let (doc, fidelity) = decode_html_bytes(&raw.body, &HtmlDecodeOptions::default())
                .map_err(|e| FetchError::Decode(e.to_string()))?;
            (
                markup_text(&doc.blocks),
                "codec/doc",
                fidelity
                    .dropped
                    .into_iter()
                    .map(|v| format!("{v:?}"))
                    .chain(fidelity.warnings)
                    .collect(),
            )
        }
        Some(value) if sim_codec_feed::MEDIA_TYPES.contains(&value) => {
            let doc = sim_codec_feed::decode_feed(&raw.body, &Default::default())
                .map_err(|e| FetchError::Decode(e.to_string()))?;
            let mut text = String::new();
            if let Some(title) = doc.title {
                text.push_str(&title);
                text.push('\n');
            }
            for entry in doc.entries {
                if let Some(value) = entry.title {
                    text.push_str(&value);
                    text.push('\n');
                }
                if let Some(value) = entry.summary.or(entry.content) {
                    text.push_str(&value);
                    text.push('\n');
                }
            }
            (text, "codec/feed", doc.warnings)
        }
        Some("text/plain") => (
            String::from_utf8_lossy(&raw.body).into_owned(),
            "codec/plain",
            vec![],
        ),
        _ => {
            return Ok((
                None,
                RepresentationOutcome::UnsupportedRepresentation {
                    media_type: media.map(str::to_owned),
                },
            ));
        }
    };
    let representation = WebRepresentation::checked(
        raw.content_id.clone(),
        text,
        RepresentationMetadata {
            codec: codec.into(),
            codec_version: version.into(),
            media_type: media.unwrap_or("text/plain").into(),
            charset: None,
            language: None,
            fidelity_warnings: warnings,
        },
        Default::default(),
    )
    .map_err(|e| FetchError::Decode(e.to_string()))?;
    Ok((Some(representation), RepresentationOutcome::Decoded))
}

fn markup_text(blocks: &[MarkupBlock]) -> String {
    let mut out = String::new();
    for block in blocks {
        match block {
            MarkupBlock::Heading { text, .. } => {
                inline_text(text, &mut out);
                out.push('\n');
            }
            MarkupBlock::Paragraph { content, .. } => {
                inline_text(content, &mut out);
                out.push('\n');
            }
            MarkupBlock::CodeBlock { code, .. } => {
                out.push_str(code);
                out.push('\n');
            }
            MarkupBlock::Raw { text, .. } => {
                out.push_str(text);
                out.push('\n');
            }
            MarkupBlock::Quote { blocks, .. } => out.push_str(&markup_text(blocks)),
            MarkupBlock::List { items, .. } => {
                for item in items {
                    out.push_str(&markup_text(item));
                }
            }
            MarkupBlock::Table { header, rows, .. } => {
                for cell in header {
                    inline_text(cell, &mut out);
                    out.push('\t');
                }
                out.push('\n');
                for row in rows {
                    for cell in row {
                        inline_text(cell, &mut out);
                        out.push('\t');
                    }
                    out.push('\n');
                }
            }
            MarkupBlock::Figure { caption, .. } => {
                inline_text(caption, &mut out);
                out.push('\n');
            }
            MarkupBlock::MathBlock { source, .. } => {
                out.push_str(&source.text);
                out.push('\n');
            }
        }
    }
    out
}
fn inline_text(values: &[Inline], out: &mut String) {
    for value in values {
        match value {
            Inline::Text(text) | Inline::Code(text) => out.push_str(text),
            Inline::Emph(children)
            | Inline::Strong(children)
            | Inline::Link {
                label: children, ..
            } => inline_text(children, out),
            Inline::Math(source) => out.push_str(&source.text),
            Inline::Raw { text, .. } => out.push_str(text),
        }
    }
}
