//! Emoji shortcodes — `:smile:` → 😄.
//!
//! Obsidian renders shortcodes inline, and they are the one bit of
//! GitHub-flavoured prose people paste in that had no representation
//! here at all. The full GitHub set runs to ~1800 names; carrying it as
//! a table in a crate four workspaces build on cold is not worth the
//! bytes, so this is the common set plus every name the FTS docs
//! actually use. An unknown shortcode is left as literal text — which is
//! also what stops `10:30 - 11:00` from being mangled.

/// The emoji for a shortcode name (without the surrounding colons), or
/// `None` when the name isn't one we carry.
#[must_use]
pub fn emoji_for(name: &str) -> Option<&'static str> {
    Some(match name {
        // faces
        "smile" => "😄",
        "smiley" => "😃",
        "grin" => "😁",
        "laughing" | "satisfied" => "😆",
        "joy" => "😂",
        "wink" => "😉",
        "blush" => "😊",
        "thinking" => "🤔",
        "neutral_face" => "😐",
        "confused" => "😕",
        "cry" => "😢",
        "sob" => "😭",
        "sunglasses" => "😎",
        "scream" => "😱",
        "sleeping" => "😴",
        "shrug" => "🤷",
        // hands & people
        "+1" | "thumbsup" => "👍",
        "-1" | "thumbsdown" => "👎",
        "ok_hand" => "👌",
        "wave" => "👋",
        "clap" => "👏",
        "pray" => "🙏",
        "muscle" => "💪",
        "point_right" => "👉",
        "eyes" => "👀",
        // status & marks
        "white_check_mark" => "✅",
        "heavy_check_mark" => "✔️",
        "x" => "❌",
        "warning" => "⚠️",
        "no_entry" => "⛔",
        "question" => "❓",
        "exclamation" => "❗",
        "bangbang" => "‼️",
        "recycle" => "♻️",
        // objects & work
        "rocket" => "🚀",
        "fire" => "🔥",
        "sparkles" => "✨",
        "star" => "⭐",
        "zap" => "⚡",
        "bulb" => "💡",
        "bug" => "🐛",
        "wrench" => "🔧",
        "hammer" => "🔨",
        "gear" => "⚙️",
        "lock" => "🔒",
        "key" => "🔑",
        "package" => "📦",
        "books" => "📚",
        "memo" | "pencil" => "📝",
        "clipboard" => "📋",
        "calendar" => "📅",
        "clock" => "🕐",
        "hourglass" => "⏳",
        "chart" => "📈",
        "mag" => "🔍",
        "link" => "🔗",
        "pushpin" => "📌",
        "tada" => "🎉",
        "trophy" => "🏆",
        "art" => "🎨",
        "construction" => "🚧",
        "boom" => "💥",
        "skull" => "💀",
        "ghost" => "👻",
        "robot" => "🤖",
        "heart" => "❤️",
        "broken_heart" => "💔",
        "100" => "💯",
        "eyeglasses" => "👓",
        "coffee" => "☕",
        "beer" => "🍺",
        // music — this is a notation repo
        "musical_note" => "🎵",
        "notes" => "🎶",
        "guitar" => "🎸",
        "microphone" => "🎤",
        "headphones" => "🎧",
        "drum" => "🥁",
        "saxophone" => "🎷",
        "trumpet" => "🎺",
        "violin" => "🎻",
        "musical_keyboard" => "🎹",
        "musical_score" => "🎼",
        "control_knobs" => "🎛️",
        "studio_microphone" => "🎙️",
        "level_slider" => "🎚️",
        "speaker" => "🔊",
        "mute" => "🔇",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_known_shortcode_resolves() {
        assert_eq!(emoji_for("rocket"), Some("🚀"));
        assert_eq!(emoji_for("musical_note"), Some("🎵"));
    }

    #[test]
    fn an_unknown_shortcode_does_not() {
        assert!(emoji_for("nosuchemoji").is_none());
        // The reason a time range survives the inline scanner.
        assert!(emoji_for("30 - 11").is_none());
    }
}
