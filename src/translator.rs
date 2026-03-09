// translator.rs - 翻訳エンジン
//
// Google翻訳の無料エンドポイント / DeepL API を使ってテキストを翻訳する。

use crate::config::Config;
use crate::lang::detect_target_lang;

/// 翻訳結果を格納する構造体
#[derive(Debug, Clone)]
pub struct TranslationResult {
    /// 翻訳されたテキスト
    pub translated: String,
    /// 翻訳先言語コード
    #[allow(dead_code)]
    pub target_lang: String,
}

fn has_any_line_break(text: &str) -> bool {
    text.contains('\n') || text.contains('\r')
}

fn is_whitespace_only(text: &str) -> bool {
    text.chars().all(char::is_whitespace)
}

fn split_nonempty_lines(text: &str) -> Vec<&str> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect()
}

fn normalize_spaces(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn source_line_weights(lines: &[&str]) -> Vec<usize> {
    lines
        .iter()
        .map(|line| line.chars().count().max(1))
        .collect()
}

fn distribute_indices(total_units: usize, weights: &[usize]) -> Vec<usize> {
    if weights.is_empty() {
        return vec![0];
    }
    if total_units == 0 {
        return vec![0; weights.len() + 1];
    }

    let total_weight: usize = weights.iter().sum();
    if total_weight == 0 {
        let mut indices = Vec::with_capacity(weights.len() + 1);
        indices.push(0);
        for i in 1..=weights.len() {
            indices.push(((total_units * i) / weights.len()).min(total_units));
        }
        return indices;
    }

    let mut indices = Vec::with_capacity(weights.len() + 1);
    indices.push(0);
    let mut cumulative = 0usize;
    for weight in weights {
        cumulative += *weight;
        let idx = ((total_units as f64) * (cumulative as f64) / (total_weight as f64)).round() as usize;
        indices.push(idx.min(total_units));
    }
    indices
}

fn reflow_by_source_lines(source: &str, translated: &str) -> String {
    let source_lines = split_nonempty_lines(source);
    if source_lines.len() <= 1 {
        return translated.trim().to_string();
    }

    if has_any_line_break(translated) {
        return translated.trim().to_string();
    }

    let weights = source_line_weights(&source_lines);
    let normalized = normalize_spaces(translated);
    if normalized.is_empty() {
        return String::new();
    }

    if normalized.contains(' ') {
        let units: Vec<&str> = normalized.split(' ').filter(|s| !s.is_empty()).collect();
        let indices = distribute_indices(units.len(), &weights);
        let mut lines = Vec::with_capacity(weights.len());

        for i in 0..weights.len() {
            let mut start = indices[i];
            let mut end = indices[i + 1];
            if i > 0 && start < indices[i - 1] {
                start = indices[i - 1];
            }
            if end < start {
                end = start;
            }
            if i + 1 == weights.len() {
                end = units.len();
            }

            if start >= units.len() {
                lines.push(String::new());
                continue;
            }

            let end = end.min(units.len());
            lines.push(units[start..end].join(" "));
        }

        let joined = lines
            .into_iter()
            .filter(|line| !is_whitespace_only(line))
            .collect::<Vec<_>>()
            .join("\n");
        return if joined.is_empty() {
            normalized
        } else {
            joined
        };
    }

    let units: Vec<char> = normalized.chars().collect();
    let indices = distribute_indices(units.len(), &weights);
    let mut out = String::new();
    for i in 0..weights.len() {
        let mut start = indices[i];
        let mut end = indices[i + 1];
        if i > 0 && start < indices[i - 1] {
            start = indices[i - 1];
        }
        if end < start {
            end = start;
        }
        if i + 1 == weights.len() {
            end = units.len();
        }
        if start >= units.len() {
            continue;
        }
        let end = end.min(units.len());
        if !out.is_empty() {
            out.push('\n');
        }
        for ch in &units[start..end] {
            out.push(*ch);
        }
    }

    if out.is_empty() {
        normalized
    } else {
        out
    }
}

fn wrap_cjk_line(line: &str, max_chars: usize) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    if chars.len() <= max_chars {
        return vec![line.to_string()];
    }

    let break_chars = ['。', '、', '！', '？', '：', '；', ')', '）'];
    let mut out = Vec::new();
    let mut start = 0usize;

    while start < chars.len() {
        let mut end = (start + max_chars).min(chars.len());
        if end < chars.len() {
            let search_start = start.saturating_add(max_chars / 2);
            let mut candidate = None;
            for i in (search_start..end).rev() {
                if break_chars.contains(&chars[i]) {
                    candidate = Some(i + 1);
                    break;
                }
            }
            if let Some(c) = candidate {
                end = c;
            }
        }
        out.push(chars[start..end].iter().collect());
        start = end;
    }

    out
}

fn wrap_space_line(line: &str, max_chars: usize) -> Vec<String> {
    let words: Vec<&str> = line.split_whitespace().collect();
    if words.is_empty() {
        return vec![String::new()];
    }

    let mut out = Vec::new();
    let mut current = String::new();
    for word in words {
        let candidate_len = if current.is_empty() {
            word.len()
        } else {
            current.len() + 1 + word.len()
        };
        if candidate_len > max_chars && !current.is_empty() {
            out.push(current);
            current = word.to_string();
        } else {
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

fn smart_wrap_text(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if has_any_line_break(trimmed) {
        return trimmed.to_string();
    }

    let has_space = trimmed.contains(' ');
    let units = trimmed.chars().count();
    let limit = if has_space { 72 } else { 42 };
    if units <= limit {
        return trimmed.to_string();
    }

    let wrapped = if has_space {
        wrap_space_line(trimmed, limit)
    } else {
        wrap_cjk_line(trimmed, limit)
    };

    wrapped.join("\n")
}

/// Google翻訳の無料APIを使って翻訳する
fn google_translate(
    text: &str,
    target: &str,
    source: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let client = reqwest::blocking::Client::new();

    let response = client
        .get("https://translate.googleapis.com/translate_a/single")
        .query(&[
            ("client", "gtx"),
            ("sl", source),
            ("tl", target),
            ("dt", "t"),
            ("q", text),
        ])
        .send()?;

    let json: serde_json::Value = response.json()?;

    let mut result = String::new();
    if let Some(sentences) = json.get(0).and_then(|v| v.as_array()) {
        for sentence in sentences {
            if let Some(translated) = sentence.get(0).and_then(|v| v.as_str()) {
                result.push_str(translated);
            }
        }
    }

    if result.is_empty() {
        Err("翻訳結果を取得できませんでした".into())
    } else {
        Ok(result)
    }
}

/// DeepL API を使って翻訳する
fn deepl_translate(
    text: &str,
    target: &str,
    api_key: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let client = reqwest::blocking::Client::new();

    let target_upper = target.to_uppercase();

    let base_url = if api_key.ends_with(":fx") {
        "https://api-free.deepl.com/v2/translate"
    } else {
        "https://api.deepl.com/v2/translate"
    };

    let response = client
        .post(base_url)
        .header("Authorization", format!("DeepL-Auth-Key {}", api_key))
        .form(&[
            ("text", text),
            ("target_lang", &target_upper),
        ])
        .send()?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().unwrap_or_default();
        return Err(format!("DeepL API エラー ({}): {}", status, body).into());
    }

    let json: serde_json::Value = response.json()?;

    let translated = json
        .get("translations")
        .and_then(|t| t.get(0))
        .and_then(|t| t.get("text"))
        .and_then(|t| t.as_str())
        .ok_or("DeepL: 翻訳結果のパースに失敗")?;

    Ok(translated.to_string())
}

/// テキストを翻訳する（メイン関数）
pub fn translate(text: &str, config: &Config) -> Result<TranslationResult, Box<dyn std::error::Error>> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(TranslationResult {
            translated: String::new(),
            target_lang: String::new(),
        });
    }

    let target = detect_target_lang(text, config);
    let source = &config.source_lang;

    let translated_raw = match config.engine.as_str() {
        "deepl" => {
            if config.deepl_api_key.is_empty() {
                return Err("DeepL APIキーが未設定です。~/.quick-tools/config.json を編集してください".into());
            }
            deepl_translate(text, &target, &config.deepl_api_key)?
        }
        _ => {
            google_translate(text, &target, source)?
        }
    };
    let translated = smart_wrap_text(&reflow_by_source_lines(text, &translated_raw));

    Ok(TranslationResult {
        translated,
        target_lang: target,
    })
}
