//! Structured composer fragments for bounded large-paste rendering.

use std::collections::BTreeMap;
use std::ops::Range;

const LARGE_PASTE_LINE_THRESHOLD: usize = 10;
const LARGE_PASTE_CHAR_THRESHOLD: usize = 1000;

/// How pasted text should be inserted into the visible textarea.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PasteDisposition {
    Inline(String),
    Collapsed { token: String },
}

#[derive(Debug, Clone)]
struct PasteFragment {
    token: String,
    content: String,
}

#[derive(Debug, Clone)]
struct DraftAttachment {
    token: String,
    attachment: y_core::types::Attachment,
}

/// Non-visible content associated with atomic tokens in the composer.
#[derive(Debug, Clone, Default)]
pub struct ComposerDraft {
    next_fragment_id: u64,
    fragments: BTreeMap<u64, PasteFragment>,
    attachments: BTreeMap<u64, DraftAttachment>,
}

impl ComposerDraft {
    /// Collapse a paste when either the line or character threshold is exceeded.
    pub fn ingest_paste(&mut self, text: &str) -> PasteDisposition {
        let line_count = text.split('\n').count();
        let char_count = text.chars().count();
        if line_count <= LARGE_PASTE_LINE_THRESHOLD && char_count <= LARGE_PASTE_CHAR_THRESHOLD {
            return PasteDisposition::Inline(text.to_string());
        }

        self.next_fragment_id = self.next_fragment_id.saturating_add(1);
        let id = self.next_fragment_id;
        let token = format!("[Paste #{id}: {line_count} lines, {char_count} chars]");
        self.fragments.insert(
            id,
            PasteFragment {
                token: token.clone(),
                content: text.to_string(),
            },
        );
        PasteDisposition::Collapsed { token }
    }

    /// Insert a paste literally, without registering a display fragment.
    pub fn ingest_raw_paste(text: &str) -> PasteDisposition {
        PasteDisposition::Inline(text.to_string())
    }

    /// Expand every intact token into its exact source text for submission.
    pub fn expand(&self, visible_text: &str) -> String {
        let expanded = self
            .fragments
            .values()
            .fold(visible_text.to_string(), |text, fragment| {
                text.replace(&fragment.token, &fragment.content)
            });
        self.attachments
            .values()
            .fold(expanded, |text, attachment| {
                text.replace(&attachment.token, "")
            })
    }

    /// Return the character-index range of a token containing or immediately
    /// preceding the cursor on one visible line.
    pub fn token_touching_cursor(&self, line: &str, cursor: usize) -> Option<Range<usize>> {
        self.fragments
            .values()
            .map(|fragment| fragment.token.as_str())
            .chain(
                self.attachments
                    .values()
                    .map(|attachment| attachment.token.as_str()),
            )
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
            .fragments
            .iter()
            .find_map(|(id, fragment)| (fragment.token == token).then_some(*id));
        if id.and_then(|id| self.fragments.remove(&id)).is_some() {
            return true;
        }
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
        self.next_fragment_id = self.next_fragment_id.saturating_add(1);
        let id = self.next_fragment_id;
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
        self.fragments.is_empty() && self.attachments.is_empty()
    }

    #[cfg(test)]
    pub fn fragment_count(&self) -> usize {
        self.fragments.len()
    }

    pub fn clear(&mut self) {
        self.fragments.clear();
        self.attachments.clear();
    }

    /// Drop hidden atoms whose visible token was removed by an external editor.
    pub fn retain_visible_tokens(&mut self, visible_text: &str) {
        self.fragments
            .retain(|_, fragment| visible_text.contains(&fragment.token));
        self.attachments
            .retain(|_, attachment| visible_text.contains(&attachment.token));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_large_multiline_paste_collapses_and_expands_exactly() {
        let mut draft = ComposerDraft::default();
        let source = (0..11)
            .map(|index| format!("line {index}"))
            .collect::<Vec<_>>()
            .join("\n");

        let PasteDisposition::Collapsed { token } = draft.ingest_paste(&source) else {
            panic!("large paste should collapse");
        };

        assert!(token.contains("11 lines"));
        assert_eq!(
            draft.expand(&format!("before {token} after")),
            format!("before {source} after")
        );
    }

    #[test]
    fn test_large_single_line_paste_collapses_by_character_count() {
        let mut draft = ComposerDraft::default();
        let source = "x".repeat(1001);

        assert!(matches!(
            draft.ingest_paste(&source),
            PasteDisposition::Collapsed { .. }
        ));
    }

    #[test]
    fn test_small_paste_stays_inline_and_is_not_registered() {
        let mut draft = ComposerDraft::default();

        assert_eq!(
            draft.ingest_paste("small\npaste"),
            PasteDisposition::Inline("small\npaste".to_string())
        );
        assert_eq!(draft.fragment_count(), 0);
    }

    #[test]
    fn test_raw_paste_bypasses_collapsing() {
        let draft = ComposerDraft::default();
        let source = "x".repeat(2000);

        assert_eq!(
            ComposerDraft::ingest_raw_paste(&source),
            PasteDisposition::Inline(source)
        );
        assert_eq!(draft.fragment_count(), 0);
    }

    #[test]
    fn test_token_range_is_detected_at_cursor_and_removal_drops_fragment() {
        let mut draft = ComposerDraft::default();
        let PasteDisposition::Collapsed { token } = draft.ingest_paste(&"x".repeat(1001)) else {
            panic!("large paste should collapse");
        };
        let line = format!("prefix {token} suffix");
        let end = "prefix ".chars().count() + token.chars().count();

        let range = draft.token_touching_cursor(&line, end).unwrap();
        assert_eq!(range.start, "prefix ".chars().count());
        assert_eq!(range.end, end);
        assert!(draft.remove_token(&token));
        assert_eq!(draft.fragment_count(), 0);
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
    fn external_editor_token_removal_discards_hidden_payload() {
        let mut draft = ComposerDraft::default();
        let PasteDisposition::Collapsed { token: _ } = draft.ingest_paste(&"x".repeat(1001)) else {
            panic!("large paste should collapse");
        };
        assert_eq!(draft.fragment_count(), 1);

        draft.retain_visible_tokens("kept without token");

        assert_eq!(draft.fragment_count(), 0);
    }
}
