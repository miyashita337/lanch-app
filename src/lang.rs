// lang.rs - 言語自動判定
//
// テキストにひらがな・カタカナ・CJK漢字が含まれているかで
// 日本語かどうかを判定する。

use crate::config::Config;

/// テキストが日本語を含むかどうかを判定する
///
/// Unicode の範囲:
/// - ひらがな: U+3040 〜 U+309F
/// - カタカナ: U+30A0 〜 U+30FF
/// - CJK統合漢字: U+4E00 〜 U+9FFF
pub fn is_japanese(text: &str) -> bool {
    text.chars().any(|ch| {
        matches!(ch,
            '\u{3040}'..='\u{309F}' |  // ひらがな
            '\u{30A0}'..='\u{30FF}' |  // カタカナ
            '\u{4E00}'..='\u{9FFF}'    // CJK統合漢字
        )
    })
}

/// テキストの内容から翻訳先言語を自動判定する
pub fn detect_target_lang(text: &str, config: &Config) -> String {
    if is_japanese(text) {
        config.target_lang_ja.clone()
    } else {
        config.target_lang_en.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_japanese_hiragana() {
        assert!(is_japanese("こんにちは"));
    }

    #[test]
    fn test_is_japanese_katakana() {
        assert!(is_japanese("テスト"));
    }

    #[test]
    fn test_is_japanese_kanji() {
        assert!(is_japanese("漢字"));
    }

    #[test]
    fn test_is_japanese_mixed() {
        assert!(is_japanese("Hello こんにちは World"));
    }

    #[test]
    fn test_is_not_japanese() {
        assert!(!is_japanese("Hello World"));
        assert!(!is_japanese("12345"));
        assert!(!is_japanese(""));
    }

    #[test]
    fn test_detect_target_lang() {
        let config = Config::default();
        assert_eq!(detect_target_lang("こんにちは", &config), "en");
        assert_eq!(detect_target_lang("Hello", &config), "ja");
    }
}
