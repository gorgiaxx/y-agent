//! Structured composer attachments kept separate from submitted text.

use std::collections::BTreeMap;
use std::ops::Range;

#[derive(Debug, Clone)]
struct DraftAttachment {
    token: String,
    attachment: y_core::types::Attachment,
}

/// Non-visible content associated with atomic tokens in the composer.
#[derive(Debug, Clone, Default)]
pub struct ComposerDraft {
    next_attachment_id: u64,
    attachments: BTreeMap<u64, DraftAttachment>,
}

impl ComposerDraft {
    /// Keep pasted text in the textarea so the visible draft is the submitted
    /// source of truth.
    pub fn ingest_paste(text: &str) -> String {
        text.to_string()
    }

    /// Insert a paste literally.
    pub fn ingest_raw_paste(text: &str) -> String {
        text.to_string()
    }

    /// Remove attachment display tokens before submitting the text.
    pub fn expand(&self, visible_text: &str) -> String {
        self.attachments
            .values()
            .fold(visible_text.to_string(), |text, attachment| {
                text.replace(&attachment.token, "")
            })
    }

    /// Return the character-index range of a token containing or immediately
    /// preceding the cursor on one visible line.
    pub fn token_touching_cursor(&self, line: &str, cursor: usize) -> Option<Range<usize>> {
        self.attachments
            .values()
            .map(|attachment| attachment.token.as_str())
            .find_map(|token| {
                line.match_indices(token).find_map(|(byte_start, _)| {
                    let start = line[..byte_start].chars().count();
                    let end = start + token.chars().count();
                    (cursor >= start && cursor <= end).then_some(start..end)
                })
            })
    }

    /// Forget an atom after the visible token is removed.
    pub fn remove_token(&mut self, token: &str) -> bool {
        let id = self
            .attachments
            .iter()
            .find_map(|(id, attachment)| (attachment.token == token).then_some(*id));
        id.and_then(|id| self.attachments.remove(&id)).is_some()
    }

    /// Register an attachment and return its atomic display token.
    pub fn add_attachment(
        &mut self,
        attachment: y_core::types::Attachment,
        dimensions: Option<(usize, usize)>,
    ) -> String {
        self.next_attachment_id = self.next_attachment_id.saturating_add(1);
        let id = self.next_attachment_id;
        let dimensions =
            dimensions.map_or_else(String::new, |(width, height)| format!(" {width}x{height}"));
        let kind = if attachment.mime_type.starts_with("image/") {
            "Image"
        } else {
            "File"
        };
        let token = format!("[{kind} #{id}: {}{dimensions}]", attachment.filename);
        self.attachments.insert(
            id,
            DraftAttachment {
                token: token.clone(),
                attachment,
            },
        );
        token
    }

    /// Typed attachments in insertion order.
    pub fn attachments(&self) -> Vec<y_core::types::Attachment> {
        self.attachments
            .values()
            .map(|attachment| attachment.attachment.clone())
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.attachments.is_empty()
    }

    pub fn clear(&mut self) {
        self.attachments.clear();
    }

    /// Drop hidden atoms whose visible token was removed by an external editor.
    pub fn retain_visible_tokens(&mut self, visible_text: &str) {
        self.attachments
            .retain(|_, attachment| visible_text.contains(&attachment.token));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_large_multiline_paste_keeps_source_text_through_submission() {
        let draft = ComposerDraft::default();
        let source = (0..47)
            .map(|index| format!("workflow line {index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let visible = format!(
            "Optimize .github/workflows\n\n{}",
            ComposerDraft::ingest_paste(&source)
        );

        assert_eq!(
            draft.expand(&visible),
            format!("Optimize .github/workflows\n\n{source}")
        );
    }

    #[test]
    fn test_large_single_line_paste_stays_visible() {
        let source = "x".repeat(1001);

        assert_eq!(ComposerDraft::ingest_paste(&source), source);
    }

    #[test]
    fn test_small_paste_stays_inline_and_is_not_registered() {
        assert_eq!(ComposerDraft::ingest_paste("small\npaste"), "small\npaste");
    }

    #[test]
    fn test_raw_paste_preserves_source_text() {
        let source = "x".repeat(2000);

        assert_eq!(ComposerDraft::ingest_raw_paste(&source), source);
    }

    #[test]
    fn test_attachment_token_is_atomic_and_excluded_from_submitted_text() {
        let mut draft = ComposerDraft::default();
        let attachment = y_core::types::Attachment {
            id: "image-1".into(),
            filename: "clipboard.png".into(),
            mime_type: "image/png".into(),
            size: 8,
            sha256: None,
            width: Some(640),
            height: Some(480),
            source: y_core::types::AttachmentSource::InlineBase64 {
                base64_data: "iVBORw==".into(),
            },
        };

        let token = draft.add_attachment(attachment.clone(), Some((640, 480)));

        assert!(token.contains("640x480"));
        assert_eq!(draft.expand(&format!("inspect {token}")), "inspect ");
        assert_eq!(draft.attachments(), vec![attachment]);
        assert!(draft.remove_token(&token));
        assert!(draft.attachments().is_empty());
    }

    #[test]
    fn external_editor_token_removal_discards_hidden_attachment() {
        let mut draft = ComposerDraft::default();
        let attachment = y_core::types::Attachment {
            id: "image-1".into(),
            filename: "clipboard.png".into(),
            mime_type: "image/png".into(),
            size: 8,
            sha256: None,
            width: None,
            height: None,
            source: y_core::types::AttachmentSource::InlineBase64 {
                base64_data: "iVBORw==".into(),
            },
        };
        draft.add_attachment(attachment, None);
        assert!(!draft.is_empty());

        draft.retain_visible_tokens("kept without token");

        assert!(draft.is_empty());
    }
}
