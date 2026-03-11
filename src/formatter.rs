// formatter.rs - Markdown整形（ハイブリッド方式）
//
// 選択テキストを Claude に送信し、Markdown形式に整形して返す。
//
// 認証の優先順位:
//   1. ANTHROPIC_API_KEY 環境変数 → HTTP API 直接呼び出し（高速: 2-3秒）
//   2. Claude Code CLI (`claude -p`) → Max Plan サブスクリプション枠（低速: 20-30秒）
//
// フロー:
//   1. ホットキー (Ctrl+Shift+F) で選択テキストをコピー
//   2. Claude に整形リクエストを送信
//   3. 整形結果をクリップボードにコピー

use crate::config::Config;
use std::io::Write;
use std::process::{Command, Stdio};

/// 整形結果を格納する構造体
#[derive(Debug, Clone)]
pub struct FormatResult {
    /// 整形されたMarkdownテキスト
    pub formatted: String,
}

/// システムプロンプト（Markdown整形用）
const FORMAT_SYSTEM_PROMPT: &str = r#"あなたはテキスト整形の専門家です。
与えられたテキストをMarkdown形式に整形してください。

ルール:
- コードブロックがあれば適切な言語タグ付きのfenced code blockにする（言語の自動検出）
- テーブルデータ（TSV、CSV、スペース区切り、崩れた表）を検出してMarkdownテーブルに復元する
- 箇条書きや番号付きリストを適切に整形する
- 見出しレベルを適切に付与する
- 不要な空白や改行を整理する
- 元のテキストの内容は変更しない（整形のみ）
- 説明や前置きは不要。整形結果のみを返す
- URLがあればMarkdownリンクとして整形する
- インラインコード（変数名、コマンド、ファイル名等）は`バッククォート`で囲む
- JSON、YAML、XML等の構造化データはコードブロックで整形する
- ログ出力やスタックトレースはコードブロック（text or log）で整形する"#;

/// 利用可能なバックエンドの種類
#[derive(Debug, Clone, PartialEq)]
pub enum Backend {
    /// Anthropic HTTP API 直接呼び出し（高速）
    Api,
    /// Claude Code CLI 経由（Max Plan 枠、低速）
    Cli,
    /// どちらも利用不可
    None,
}

/// 利用可能なバックエンドを判定する
pub fn detect_backend() -> Backend {
    // 1. API キーがあれば API 直接（高速）
    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
        if !key.trim().is_empty() {
            return Backend::Api;
        }
    }

    // 2. Claude CLI があれば CLI 経由（低速だが無料）
    if check_cli_available() {
        return Backend::Cli;
    }

    Backend::None
}

/// バックエンドの表示名を返す
pub fn backend_label(backend: &Backend) -> &'static str {
    match backend {
        Backend::Api => "API直接（高速）",
        Backend::Cli => "Claude CLI（Max Plan）",
        Backend::None => "利用不可",
    }
}

/// Claude Code CLI が利用可能か確認する
pub fn check_cli_available() -> bool {
    Command::new("claude")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// テキストをMarkdown整形する（バックエンドを自動選択）
///
/// ANTHROPIC_API_KEY が設定されていれば高速な API 直接呼び出し、
/// なければ Claude CLI フォールバック。
pub fn format_markdown(text: &str, config: &Config) -> Result<FormatResult, Box<dyn std::error::Error>> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(FormatResult {
            formatted: String::new(),
        });
    }

    let backend = detect_backend();
    eprintln!("[format] バックエンド: {}", backend_label(&backend));

    let formatted = match backend {
        Backend::Api => call_api_direct(text, config)?,
        Backend::Cli => call_claude_cli(text, &config.claude_model)?,
        Backend::None => {
            return Err("Markdown整形を利用するには:\n\
                ① ANTHROPIC_API_KEY 環境変数を設定（高速）\n\
                ② または Claude Code をインストール（claude login）".into());
        }
    };

    Ok(FormatResult { formatted })
}

// ============================================================
// バックエンド A: Anthropic HTTP API 直接呼び出し（高速）
// ============================================================

/// Anthropic Messages API を直接呼び出す
fn call_api_direct(
    text: &str,
    config: &Config,
) -> Result<String, Box<dyn std::error::Error>> {
    let api_key = std::env::var("ANTHROPIC_API_KEY")
        .map_err(|_| "ANTHROPIC_API_KEY が未設定です")?;

    // Markdown整形は単純タスク → Haiku で十分（高速＆安価）
    let model = if config.claude_model.is_empty() {
        "claude-haiku-4-5-20251001".to_string()
    } else {
        config.claude_model.clone()
    };

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let body = serde_json::json!({
        "model": model,
        "max_tokens": 4096,
        "system": FORMAT_SYSTEM_PROMPT,
        "messages": [
            {
                "role": "user",
                "content": text
            }
        ]
    });

    let response = match client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
    {
        Ok(resp) => resp,
        Err(e) => {
            if e.is_timeout() {
                return Err("API タイムアウト（30秒）".into());
            } else if e.is_connect() {
                return Err("API接続エラー。ネットワークを確認してください".into());
            } else {
                return Err(format!("API通信エラー: {}", e).into());
            }
        }
    };

    let status = response.status();
    if !status.is_success() {
        let error_body = response.text().unwrap_or_default();

        let user_msg = if error_body.contains("credit balance is too low") {
            "クレジット残高不足。console.anthropic.com でチャージしてください"
        } else {
            match status.as_u16() {
                400 => "リクエストエラー",
                401 => "APIキーが無効です",
                403 => "APIアクセス拒否",
                429 => "レート制限。しばらく待ってください",
                500..=599 => "APIサーバーエラー",
                _ => "APIエラー",
            }
        };
        eprintln!("[format] API エラー詳細 ({}): {}", status, error_body);
        return Err(format!("{} ({})", user_msg, status).into());
    }

    let json: serde_json::Value = response.json()?;

    let formatted = json
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|block| block.get("text"))
        .and_then(|t| t.as_str())
        .ok_or("API レスポンスのパースに失敗")?;

    Ok(formatted.to_string())
}

// ============================================================
// バックエンド B: Claude Code CLI 経由（Max Plan 枠）
// ============================================================

/// Claude Code CLI を呼び出してテキストを整形する
fn call_claude_cli(
    text: &str,
    model: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let model_arg = normalize_model_name(model);

    let mut child = Command::new("claude")
        .args([
            "-p",
            "--model", &model_arg,
            "--system-prompt", FORMAT_SYSTEM_PROMPT,
            "--no-session-persistence",
        ])
        // ANTHROPIC_API_KEY が設定されていると Claude CLI が
        // Max Plan ではなく API キーを使ってしまうため、明示的に除外
        .env_remove("ANTHROPIC_API_KEY")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "claude コマンドが見つかりません。Claude Code をインストールしてください".to_string()
            } else {
                format!("claude コマンドの起動に失敗: {}", e)
            }
        })?;

    // stdin にテキストを書き込んでクローズ
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(text.as_bytes())?;
    }

    let output = child.wait_with_output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);

        let err_text = format!("{}{}", stderr, stdout);
        let user_msg = if err_text.contains("not logged in") || err_text.contains("authentication") {
            "Claude Code にログインしてください: claude login"
        } else if err_text.contains("rate limit") || err_text.contains("too many") {
            "レート制限。しばらく待ってから再試行してください"
        } else if err_text.contains("credit balance") {
            "CLI経由でもクレジット不足。claude login で正しいアカウントにログインしてください"
        } else {
            "Claude CLI でエラーが発生しました"
        };

        eprintln!("[format] CLI エラー (exit={})", output.status);
        eprintln!("[format]   stderr: {}", stderr.trim());
        eprintln!("[format]   stdout: {}", stdout.trim());
        return Err(user_msg.into());
    }

    let result = String::from_utf8(output.stdout)?;
    let trimmed = result.trim().to_string();

    if trimmed.is_empty() {
        return Err("Claude CLI: 空の応答が返されました".into());
    }

    Ok(trimmed)
}

/// モデル名を CLI 用に正規化する
fn normalize_model_name(model: &str) -> String {
    let m = model.trim();
    if m.is_empty() {
        "haiku".to_string()
    } else {
        m.to_string()
    }
}
