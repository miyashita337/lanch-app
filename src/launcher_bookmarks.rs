// launcher_bookmarks.rs - Chromeブックマーク検索
//
// Chrome の Bookmarks JSON を読み取り、部分一致で検索する。
// Chrome 未インストールなら空リスト（エラーなし）。

use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

/// ブックマークエントリ
#[derive(Debug, Clone)]
pub struct BookmarkEntry {
    pub name: String,
    pub url: String,
}

/// Chrome Bookmarks JSON のルート
#[derive(Deserialize)]
struct BookmarksFile {
    roots: std::collections::HashMap<String, BookmarkNode>,
}

/// ブックマークノード（フォルダ or URL）
#[derive(Deserialize)]
struct BookmarkNode {
    #[serde(default)]
    name: String,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    children: Option<Vec<BookmarkNode>>,
}

/// 走査結果（読み込めたエントリと、読めなかったファイルの内訳）
#[derive(Debug, Default)]
struct ScanResult {
    entries: Vec<BookmarkEntry>,
    /// ファイルが存在しない（Chrome 未使用・未作成プロファイルなど。正常ケースもある）
    missing: Vec<PathBuf>,
    /// 存在するが読めない・JSON が壊れている（警告対象）
    failed: Vec<(PathBuf, String)>,
}

/// Chrome ブックマークを読み込む
///
/// `profiles` が空なら Default と Profile 1〜5 を走査する。
/// 読み込みに失敗してもランチャーは継続し、原因を stderr（ログファイル）に残す。
pub fn load_bookmarks(profiles: &[String]) -> Vec<BookmarkEntry> {
    let Some(base) = chrome_user_data_dir() else {
        eprintln!("[launcher] info: Chrome のユーザーデータ dir を特定できないためブックマークを読み込みません");
        return Vec::new();
    };
    let result = scan_profiles(&base, profiles);
    if !result.missing.is_empty() {
        eprintln!(
            "[launcher] info: Bookmarks ファイルなし（{} 件、Chrome 未使用または未作成プロファイル）: {}",
            result.missing.len(),
            join_paths(&result.missing)
        );
    }
    for (path, reason) in &result.failed {
        eprintln!(
            "[launcher] warn: Bookmarks を読み込めません: {} ({})",
            path.display(),
            reason
        );
    }
    result.entries
}

fn join_paths(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// 走査対象のプロファイルディレクトリ名。空指定なら従来どおりの固定リスト
fn profile_dir_names(profiles: &[String]) -> Vec<String> {
    if profiles.is_empty() {
        std::iter::once("Default".to_string())
            .chain((1..=5).map(|i| format!("Profile {}", i)))
            .collect()
    } else {
        profiles.to_vec()
    }
}

/// `base` 配下の各プロファイルの Bookmarks を読み、プロファイル間の重複 URL を除去して返す
fn scan_profiles(base: &Path, profiles: &[String]) -> ScanResult {
    let mut result = ScanResult::default();
    for name in profile_dir_names(profiles) {
        let path = base.join(&name).join("Bookmarks");
        match fs::read_to_string(&path) {
            Ok(content) => match serde_json::from_str::<BookmarksFile>(&content) {
                Ok(file) => {
                    for node in file.roots.values() {
                        flatten_bookmarks(node, &mut result.entries);
                    }
                }
                Err(e) => result.failed.push((path, format!("JSON が不正: {}", e))),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => result.missing.push(path),
            Err(e) => result.failed.push((path, format!("読み込みエラー: {}", e))),
        }
    }
    // 重複URL除去（挿入順を保ったまま、プロファイル間の重複も除去する）
    // dedup_by は隣接要素しか除去できないため HashSet で全体の重複を弾く
    let mut seen = std::collections::HashSet::new();
    result.entries.retain(|e| seen.insert(e.url.clone()));
    result
}

/// ブックマークを部分一致検索（AND検索、名前優先スコアリング）
pub fn search(bookmarks: &[BookmarkEntry], query: &str, limit: usize) -> Vec<BookmarkEntry> {
    let query_lower = query.to_lowercase();
    let terms: Vec<&str> = query_lower.split_whitespace().collect();
    if terms.is_empty() {
        return Vec::new();
    }

    let mut scored: Vec<(i32, &BookmarkEntry)> = bookmarks
        .iter()
        .filter_map(|entry| {
            let name_lower = entry.name.to_lowercase();
            let url_lower = entry.url.to_lowercase();
            // 全termsがname or urlのいずれかに含まれること
            let all_match = terms
                .iter()
                .all(|term| name_lower.contains(term) || url_lower.contains(term));
            if !all_match {
                return None;
            }
            // スコア: 名前一致は高得点
            let mut score = 0;
            for term in &terms {
                if name_lower.contains(term) {
                    score += 10;
                }
                if url_lower.contains(term) {
                    score += 1;
                }
                // 先頭一致ボーナス
                if name_lower.starts_with(term) {
                    score += 20;
                }
            }
            Some((score, entry))
        })
        .collect();

    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored
        .into_iter()
        .take(limit)
        .map(|(_, e)| e.clone())
        .collect()
}

fn flatten_bookmarks(node: &BookmarkNode, out: &mut Vec<BookmarkEntry>) {
    if let Some(url) = &node.url {
        if !url.is_empty() {
            out.push(BookmarkEntry {
                name: node.name.clone(),
                url: url.clone(),
            });
        }
    }
    if let Some(children) = &node.children {
        for child in children {
            flatten_bookmarks(child, out);
        }
    }
}

fn chrome_user_data_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let local_app_data = std::env::var("LOCALAPPDATA").ok()?;
        Some(
            PathBuf::from(local_app_data)
                .join("Google")
                .join("Chrome")
                .join("User Data"),
        )
    }

    #[cfg(not(windows))]
    {
        dirs::home_dir().map(|home| home.join(".config/google-chrome"))
    }
}

// =============================================================================
// テスト
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_bookmarks() -> Vec<BookmarkEntry> {
        vec![
            BookmarkEntry {
                name: "Rust Programming".into(),
                url: "https://www.rust-lang.org".into(),
            },
            BookmarkEntry {
                name: "GitHub".into(),
                url: "https://github.com".into(),
            },
            BookmarkEntry {
                name: "Rust by Example".into(),
                url: "https://doc.rust-lang.org/rust-by-example".into(),
            },
            BookmarkEntry {
                name: "Google".into(),
                url: "https://www.google.com".into(),
            },
            BookmarkEntry {
                name: "Zenn".into(),
                url: "https://zenn.dev".into(),
            },
        ]
    }

    #[test]
    fn test_search_single_term() {
        let bm = sample_bookmarks();
        let results = search(&bm, "rust", 5);
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(
            |r| r.name.to_lowercase().contains("rust") || r.url.to_lowercase().contains("rust")
        ));
    }

    #[test]
    fn test_search_multi_term() {
        let bm = sample_bookmarks();
        let results = search(&bm, "rust example", 5);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "Rust by Example");
    }

    #[test]
    fn test_search_url_match() {
        let bm = sample_bookmarks();
        let results = search(&bm, "github", 5);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_search_empty_query() {
        let bm = sample_bookmarks();
        let results = search(&bm, "", 5);
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_no_match() {
        let bm = sample_bookmarks();
        let results = search(&bm, "nonexistent", 5);
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_limit() {
        let bm = sample_bookmarks();
        let results = search(&bm, "o", 2); // matches Google, rust-lang.org, etc.
        assert!(results.len() <= 2);
    }

    #[test]
    fn test_load_bookmarks_no_crash() {
        // Chrome未インストール環境でもパニックしない
        let _ = load_bookmarks(&[]);
    }

    const VALID_JSON: &str = r#"{"roots":{"bookmark_bar":{"name":"bar","children":[
        {"name":"Rust","url":"https://www.rust-lang.org"}]}}}"#;

    fn write_profile(base: &Path, profile: &str, content: &str) {
        let dir = base.join(profile);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("Bookmarks"), content).unwrap();
    }

    #[test]
    fn test_profile_dir_names_empty_uses_fixed_list() {
        let names = profile_dir_names(&[]);
        assert_eq!(
            names,
            ["Default", "Profile 1", "Profile 2", "Profile 3", "Profile 4", "Profile 5"]
        );
    }

    #[test]
    fn test_profile_dir_names_explicit() {
        let names = profile_dir_names(&["Profile 9".to_string()]);
        assert_eq!(names, ["Profile 9"]);
    }

    #[test]
    fn test_scan_missing_file_is_reported_not_failed() {
        let tmp = tempfile::tempdir().unwrap();
        let r = scan_profiles(tmp.path(), &[]);
        assert!(r.entries.is_empty());
        assert_eq!(r.missing.len(), 6);
        assert!(r.failed.is_empty());
    }

    #[test]
    fn test_scan_broken_json_is_failed_and_others_continue() {
        let tmp = tempfile::tempdir().unwrap();
        write_profile(tmp.path(), "Default", "{ not json");
        write_profile(tmp.path(), "Profile 1", VALID_JSON);
        let r = scan_profiles(tmp.path(), &[]);
        assert_eq!(r.failed.len(), 1);
        assert!(r.failed[0].0.starts_with(tmp.path().join("Default")));
        assert_eq!(r.entries.len(), 1);
        assert_eq!(r.entries[0].url, "https://www.rust-lang.org");
    }

    #[test]
    fn test_scan_unreadable_path_is_failed() {
        // Bookmarks がディレクトリだと NotFound 以外の読み込みエラーになる
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("Default").join("Bookmarks")).unwrap();
        let r = scan_profiles(tmp.path(), &["Default".to_string()]);
        assert_eq!(r.failed.len(), 1);
        assert!(r.missing.is_empty());
    }

    #[test]
    fn test_scan_explicit_profiles_only() {
        let tmp = tempfile::tempdir().unwrap();
        write_profile(tmp.path(), "Default", VALID_JSON);
        write_profile(
            tmp.path(),
            "Work",
            r#"{"roots":{"other":{"name":"o","children":[{"name":"GH","url":"https://github.com"}]}}}"#,
        );
        let r = scan_profiles(tmp.path(), &["Work".to_string()]);
        assert_eq!(r.entries.len(), 1);
        assert_eq!(r.entries[0].url, "https://github.com");
        assert!(r.missing.is_empty() && r.failed.is_empty());
    }

    #[test]
    fn test_scan_dedups_across_profiles() {
        let tmp = tempfile::tempdir().unwrap();
        write_profile(tmp.path(), "Default", VALID_JSON);
        write_profile(tmp.path(), "Profile 1", VALID_JSON);
        let r = scan_profiles(tmp.path(), &[]);
        assert_eq!(r.entries.len(), 1);
    }
}
