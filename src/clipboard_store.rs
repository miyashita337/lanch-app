// clipboard_store.rs - クリップボード履歴ストレージ
//
// 責務: エントリの永続化（JSON index + バイナリファイル）、検索、ローテーション
//
// ストレージ構造:
//   ~/.lanch-app/clipboard-history/
//   ├── index.json          # メタデータインデックス
//   └── blobs/              # 画像等のバイナリファイル
//       ├── Image 2026-03-12 10-30-00.png
//       └── ...

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// クリップボードエントリの種類
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum EntryType {
    Text,
    Image,
    Json,
    // 将来拡張: File, Binary
}

impl std::fmt::Display for EntryType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EntryType::Text => write!(f, "Text"),
            EntryType::Image => write!(f, "Image"),
            EntryType::Json => write!(f, "JSON"),
        }
    }
}

/// クリップボード履歴の1エントリ
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipboardEntry {
    /// ユニークID
    pub id: String,
    /// コピーされた日時
    pub timestamp: DateTime<Local>,
    /// エントリの種類
    pub entry_type: EntryType,
    /// テキスト内容（Text/Json の場合）
    pub text_content: Option<String>,
    /// バイナリファイル名（Image の場合、blobs/ 内の相対パス）
    pub blob_file: Option<String>,
    /// プレビュー文字列（検索・一覧表示用、最大200文字）
    pub preview: String,
    /// データサイズ（バイト）
    pub size_bytes: usize,
}

impl ClipboardEntry {
    /// テキストエントリを作成
    pub fn new_text(text: &str) -> Self {
        let now = Local::now();
        let entry_type = if looks_like_json(text) {
            EntryType::Json
        } else {
            EntryType::Text
        };
        let preview = truncate_preview(text, 200);
        let size = text.len();

        Self {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: now,
            entry_type,
            text_content: Some(text.to_string()),
            blob_file: None,
            preview,
            size_bytes: size,
        }
    }

    /// 画像エントリを作成（PNG バイト列を受け取る）
    pub fn new_image(png_data: &[u8], store_dir: &PathBuf) -> std::io::Result<Self> {
        let now = Local::now();
        let filename = format!("Image {}.png", now.format("%Y-%m-%d %H-%M-%S"));
        let blobs_dir = store_dir.join("blobs");
        fs::create_dir_all(&blobs_dir)?;

        let file_path = blobs_dir.join(&filename);
        fs::write(&file_path, png_data)?;

        let preview = format!(
            "Image {} ({})",
            now.format("%Y-%m-%d %H:%M:%S"),
            format_bytes(png_data.len())
        );

        Ok(Self {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: now,
            entry_type: EntryType::Image,
            text_content: None,
            blob_file: Some(filename),
            preview,
            size_bytes: png_data.len(),
        })
    }

    /// 検索クエリにマッチするか判定
    pub fn matches(&self, query: &str) -> bool {
        if query.is_empty() {
            return true;
        }

        let query_lower = query.to_lowercase();

        // テキスト内容で検索
        if let Some(ref text) = self.text_content {
            if text.to_lowercase().contains(&query_lower) {
                return true;
            }
        }

        // プレビューで検索
        if self.preview.to_lowercase().contains(&query_lower) {
            return true;
        }

        // 日付検索（YYYY-MM-DD 形式）
        let date_str = self.timestamp.format("%Y-%m-%d").to_string();
        if date_str.contains(&query_lower) {
            return true;
        }

        // エントリ種別で検索
        let type_str = self.entry_type.to_string().to_lowercase();
        if type_str.contains(&query_lower) {
            return true;
        }

        false
    }
}

/// クリップボード履歴ストア
pub struct ClipboardStore {
    /// エントリ一覧（新しい順）
    entries: Vec<ClipboardEntry>,
    /// ストレージディレクトリ
    store_dir: PathBuf,
    /// 保持期間（日数）
    retention_days: i64,
}

impl ClipboardStore {
    /// 新しいストアを作成し、既存のインデックスがあれば読み込む
    pub fn new(retention_days: i64) -> Self {
        let store_dir = dirs::home_dir()
            .expect("ホームディレクトリが見つかりません")
            .join(".lanch-app")
            .join("clipboard-history");

        let mut store = Self {
            entries: Vec::new(),
            store_dir,
            retention_days,
        };
        store.load_index();
        store
    }

    /// ストレージディレクトリのパスを返す
    #[allow(dead_code)]
    pub fn store_dir(&self) -> &PathBuf {
        &self.store_dir
    }

    /// テキストエントリを追加（重複チェック付き）
    pub fn add_text(&mut self, text: &str) {
        // 空文字・空白のみは無視
        if text.trim().is_empty() {
            return;
        }

        // 直前のエントリと同一内容なら無視（連続コピー防止）
        if let Some(last) = self.entries.first() {
            if let Some(ref last_text) = last.text_content {
                if last_text == text {
                    return;
                }
            }
        }

        let entry = ClipboardEntry::new_text(text);
        self.entries.insert(0, entry);
        self.save_index();
    }

    /// 画像エントリを追加
    pub fn add_image(&mut self, png_data: &[u8]) {
        if png_data.is_empty() {
            return;
        }

        // 直前が画像で同サイズなら重複とみなす（簡易チェック）
        if let Some(last) = self.entries.first() {
            if last.entry_type == EntryType::Image && last.size_bytes == png_data.len() {
                return;
            }
        }

        match ClipboardEntry::new_image(png_data, &self.store_dir) {
            Ok(entry) => {
                self.entries.insert(0, entry);
                self.save_index();
            }
            Err(e) => {
                eprintln!("[clipboard_store] 画像の保存に失敗: {}", e);
            }
        }
    }

    /// 検索してページ単位で返す
    ///
    /// `page`: 0始まりのページ番号
    /// `per_page`: 1ページあたりの件数
    ///
    /// 戻り値: (エントリ一覧, 合計マッチ数)
    pub fn search(&self, query: &str, page: usize, per_page: usize) -> (Vec<ClipboardEntry>, usize) {
        let matched: Vec<&ClipboardEntry> = self
            .entries
            .iter()
            .filter(|e| e.matches(query))
            .collect();

        let total = matched.len();
        let start = page * per_page;

        if start >= total {
            return (Vec::new(), total);
        }

        let end = (start + per_page).min(total);
        let page_entries = matched[start..end].iter().map(|e| (*e).clone()).collect();

        (page_entries, total)
    }

    /// エントリ総数を返す
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 古いエントリを削除（ローテーション）
    pub fn rotate(&mut self) {
        let cutoff = Local::now() - chrono::Duration::days(self.retention_days);
        let before = self.entries.len();

        // 期限切れエントリの blob ファイルを削除
        let expired: Vec<ClipboardEntry> = self
            .entries
            .iter()
            .filter(|e| e.timestamp < cutoff)
            .cloned()
            .collect();

        for entry in &expired {
            if let Some(ref blob) = entry.blob_file {
                let path = self.store_dir.join("blobs").join(blob);
                let _ = fs::remove_file(path);
            }
        }

        self.entries.retain(|e| e.timestamp >= cutoff);

        let removed = before - self.entries.len();
        if removed > 0 {
            eprintln!(
                "[clipboard_store] {}件の古いエントリを削除しました（{}日以上前）",
                removed, self.retention_days
            );
            self.save_index();
        }
    }

    /// 画像の blob ファイルパスを返す
    pub fn blob_path(&self, filename: &str) -> PathBuf {
        self.store_dir.join("blobs").join(filename)
    }

    // --- 永続化 ---

    fn index_path(&self) -> PathBuf {
        self.store_dir.join("index.json")
    }

    fn load_index(&mut self) {
        let path = self.index_path();
        if !path.exists() {
            return;
        }

        match fs::read_to_string(&path) {
            Ok(json) => match serde_json::from_str::<Vec<ClipboardEntry>>(&json) {
                Ok(entries) => {
                    self.entries = entries;
                    eprintln!(
                        "[clipboard_store] {}件のエントリを読み込みました",
                        self.entries.len()
                    );
                }
                Err(e) => {
                    eprintln!("[clipboard_store] インデックスのパースに失敗: {}", e);
                }
            },
            Err(e) => {
                eprintln!("[clipboard_store] インデックスの読み込みに失敗: {}", e);
            }
        }
    }

    fn save_index(&self) {
        if let Err(e) = fs::create_dir_all(&self.store_dir) {
            eprintln!("[clipboard_store] ディレクトリ作成に失敗: {}", e);
            return;
        }

        match serde_json::to_string_pretty(&self.entries) {
            Ok(json) => {
                if let Err(e) = fs::write(self.index_path(), json) {
                    eprintln!("[clipboard_store] インデックスの保存に失敗: {}", e);
                }
            }
            Err(e) => {
                eprintln!("[clipboard_store] JSONシリアライズに失敗: {}", e);
            }
        }
    }
}

// --- ヘルパー関数 ---

/// テキストが JSON っぽいかどうか簡易判定
fn looks_like_json(text: &str) -> bool {
    let trimmed = text.trim();
    (trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']'))
}

/// プレビュー用に文字列を切り詰める
fn truncate_preview(text: &str, max_chars: usize) -> String {
    let single_line = text.replace('\n', " ").replace('\r', "");
    if single_line.chars().count() > max_chars {
        let truncated: String = single_line.chars().take(max_chars).collect();
        format!("{}...", truncated)
    } else {
        single_line
    }
}

/// バイト数を人間が読みやすい形式に変換
fn format_bytes(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}
