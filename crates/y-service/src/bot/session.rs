//! Bot session binder: deterministic session creation for platform conversations.
//!
//! Maps `(platform, chat_id)` pairs to stable session IDs so each platform
//! chat gets its own session that persists across process restarts.

use y_bot::PlatformKind;
use y_core::types::SessionId;

/// Derive a deterministic session ID from platform + chat ID.
///
/// Format: `bot:<platform>:<chat_id>` -- this ensures each platform chat
/// gets its own session, and the ID is stable across restarts.
pub fn derive_bot_session_id(platform: PlatformKind, chat_id: &str) -> SessionId {
    SessionId(format!("bot:{platform}:{chat_id}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_session_id_feishu() {
        let sid = derive_bot_session_id(PlatformKind::Feishu, "oc_group_123");
        assert_eq!(sid.0, "bot:feishu:oc_group_123");
    }

    #[test]
    fn derive_session_id_telegram() {
        let sid = derive_bot_session_id(PlatformKind::Telegram, "-100123456");
        assert_eq!(sid.0, "bot:telegram:-100123456");
    }

    #[test]
    fn derive_session_id_discord() {
        let sid = derive_bot_session_id(PlatformKind::Discord, "guild_channel_789");
        assert_eq!(sid.0, "bot:discord:guild_channel_789");
    }

    #[test]
    fn derive_session_id_deterministic() {
        let a = derive_bot_session_id(PlatformKind::Feishu, "oc_abc");
        let b = derive_bot_session_id(PlatformKind::Feishu, "oc_abc");
        assert_eq!(a.0, b.0);
    }

    #[test]
    fn derive_session_id_different_platforms() {
        let a = derive_bot_session_id(PlatformKind::Feishu, "chat_123");
        let b = derive_bot_session_id(PlatformKind::Discord, "chat_123");
        assert_ne!(a.0, b.0);
    }
}
