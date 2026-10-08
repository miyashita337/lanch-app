// launcher_apps.rs - インストール済みアプリケーション検索
//
// Windows のスタートメニューフォルダをスキャンして .lnk を、
// PowerShell の Get-StartApps で Microsoft Store 形式（MSIX/UWP）のアプリを
// アプリケーション候補として列挙する。
//
// Get-StartApps は PowerShell の起動込みで数秒かかるため、ランチャーの表示を
// 止めないよう「前回の結果をキャッシュから読む + バックグラウンドで更新」とする
// （ランチャーは開くたびに別プロセスで起動するので、キャッシュはファイルに置く）。

use serde::Deserialize;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};

/// shell:AppsFolder 経由で起動するときの接頭辞
const APPS_FOLDER_PREFIX: &str = "shell:AppsFolder\\";

/// 起動対象
#[derive(Debug, Clone, PartialEq)]
pub enum AppTarget {
    /// スタートメニューの .lnk ファイルのフルパス
    Lnk(String),
    /// Store 形式アプリの AppID（Get-StartApps の AppID）
    AppId(String),
}

impl AppTarget {
    /// 履歴（launcher_history）に保存する文字列
    pub fn history_key(&self) -> String {
        match self {
            AppTarget::Lnk(path) => path.clone(),
            AppTarget::AppId(id) => format!("{}{}", APPS_FOLDER_PREFIX, id),
        }
    }

    /// 履歴の文字列から復元する。接頭辞が無ければ従来どおり .lnk パスとみなす
    pub fn from_history_key(key: &str) -> Self {
        match key.strip_prefix(APPS_FOLDER_PREFIX) {
            Some(id) => AppTarget::AppId(id.to_string()),
            None => AppTarget::Lnk(key.to_string()),
        }
    }
}

/// アプリケーションエントリ
#[derive(Debug, Clone)]
pub struct AppEntry {
    /// 表示名（.lnk 拡張子除去済み）
    pub name: String,
    /// 起動対象
    pub target: AppTarget,
}

/// スタートメニューの .lnk を列挙し、キャッシュ済みの Store アプリと合わせて返す
pub fn scan_apps() -> Vec<AppEntry> {
    merge_apps(scan_lnk_apps(), load_cached_store_apps())
}

/// スタートメニューの .lnk だけを列挙する（高速）
pub fn scan_lnk_apps() -> Vec<AppEntry> {
    let mut entries = Vec::new();
    for dir in &start_menu_dirs() {
        scan_dir(dir, &mut entries, 0);
    }
    entries
}

/// .lnk と Store アプリを統合する。同名（大文字小文字無視）は先に来た方を残す
/// ので、.lnk を優先する。名前順に並べて返す
pub fn merge_apps(lnk: Vec<AppEntry>, store: Vec<AppEntry>) -> Vec<AppEntry> {
    let mut seen = HashSet::new();
    let mut merged: Vec<AppEntry> = lnk
        .into_iter()
        .chain(store)
        .filter(|app| seen.insert(app.name.to_lowercase()))
        .collect();
    merged.sort_by_key(|a| a.name.to_lowercase());
    merged
}

/// Get-StartApps の ConvertTo-Json 出力を解析する
pub fn parse_start_apps(json: &str) -> Result<Vec<AppEntry>, String> {
    #[derive(Deserialize)]
    struct StartApp {
        #[serde(rename = "Name")]
        name: String,
        #[serde(rename = "AppID")]
        app_id: String,
    }
    // ConvertTo-Json は 1 件だけのとき配列ではなく単体オブジェクトを返す
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        Many(Vec<StartApp>),
        One(StartApp),
    }

    let json = json.trim_start_matches('\u{feff}').trim();
    if json.is_empty() {
        return Ok(Vec::new());
    }
    let items = match serde_json::from_str::<OneOrMany>(json).map_err(|e| e.to_string())? {
        OneOrMany::Many(v) => v,
        OneOrMany::One(a) => vec![a],
    };
    Ok(items
        .into_iter()
        .filter(|a| !a.name.is_empty() && !a.app_id.is_empty() && !is_uninstaller(&a.name))
        .map(|a| AppEntry {
            name: a.name,
            target: AppTarget::AppId(a.app_id),
        })
        .collect())
}

/// "Uninstall" 系は候補から除外する
fn is_uninstaller(name: &str) -> bool {
    name.to_lowercase().contains("uninstall")
}

/// 前回取得して保存した Store アプリ一覧を読む。無い・壊れている場合は空
pub fn load_cached_store_apps() -> Vec<AppEntry> {
    let Ok(json) = std::fs::read_to_string(store_cache_path()) else {
        return Vec::new();
    };
    parse_start_apps(&json).unwrap_or_else(|e| {
        eprintln!("[launcher_apps] Store アプリのキャッシュを読めません: {}", e);
        Vec::new()
    })
}

/// Store アプリ一覧をバックグラウンドで取得する。結果は Receiver に 1 回だけ届く
/// （失敗時は警告を出して何も送らず、ランチャーは .lnk だけで動き続ける）
pub fn spawn_store_refresh() -> Receiver<Vec<AppEntry>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let json = match fetch_start_apps_json() {
            Ok(json) => json,
            Err(e) => {
                eprintln!("[launcher_apps] Store アプリの一覧取得に失敗: {}", e);
                return;
            }
        };
        match parse_start_apps(&json) {
            Ok(apps) => {
                if let Err(e) = std::fs::write(store_cache_path(), &json) {
                    eprintln!("[launcher_apps] Store アプリのキャッシュ保存に失敗: {}", e);
                }
                let _ = tx.send(apps);
            }
            Err(e) => eprintln!("[launcher_apps] Get-StartApps の出力を解析できません: {}", e),
        }
    });
    rx
}

fn store_cache_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".lanch-app")
        .join("store-apps.json")
}

/// PowerShell で Get-StartApps を実行し、UTF-8 の JSON を返す
#[cfg(windows)]
fn fetch_start_apps_json() -> Result<String, String> {
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    // 既定のコンソールコードページ（cp932 等）だと日本語名が文字化けするため UTF-8 に固定する
    let script = "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; \
                  Get-StartApps | ConvertTo-Json -Compress";
    let output = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .output()
        .map_err(|e| format!("powershell を起動できません: {}", e))?;
    if !output.status.success() {
        return Err(format!(
            "powershell が失敗しました ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout).map_err(|e| format!("出力が UTF-8 ではありません: {}", e))
}

#[cfg(not(windows))]
fn fetch_start_apps_json() -> Result<String, String> {
    Err("Windows 以外では Store アプリを取得できません".into())
}

/// アプリケーションを部分一致検索
pub fn search(apps: &[AppEntry], query: &str, limit: usize) -> Vec<AppEntry> {
    let query_lower = query.to_lowercase();
    let terms: Vec<&str> = query_lower.split_whitespace().collect();
    if terms.is_empty() {
        return Vec::new();
    }

    let mut scored: Vec<(i32, &AppEntry)> = apps
        .iter()
        .filter_map(|app| {
            let name_lower = app.name.to_lowercase();
            // 全termsが名前に含まれること
            if !terms.iter().all(|term| name_lower.contains(term)) {
                return None;
            }
            let mut score = 0;
            for term in &terms {
                if name_lower.starts_with(term) {
                    score += 30; // 先頭一致ボーナス
                } else if name_lower.contains(term) {
                    score += 10;
                }
            }
            // 短い名前ほど関連性が高い（"Chrome" vs "Chrome Update Helper"）
            score += (100 - name_lower.len().min(100)) as i32;
            Some((score, app))
        })
        .collect();

    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored
        .into_iter()
        .take(limit)
        .map(|(_, e)| e.clone())
        .collect()
}

/// アプリを起動する
pub fn launch(app: &AppEntry) {
    #[cfg(windows)]
    {
        use std::process::Command;
        let result = match &app.target {
            AppTarget::Lnk(path) => Command::new("cmd").args(["/C", "start", "", path]).spawn(),
            AppTarget::AppId(id) => Command::new("explorer.exe")
                .arg(format!("{}{}", APPS_FOLDER_PREFIX, id))
                .spawn(),
        };
        if let Err(e) = result {
            eprintln!("[launcher_apps] {} を起動できません: {}", app.name, e);
        }
    }
    #[cfg(not(windows))]
    {
        let _ = app; // 非Windows: 何もしない
    }
}

fn start_menu_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    #[cfg(windows)]
    {
        // ユーザーのスタートメニュー
        if let Ok(appdata) = std::env::var("APPDATA") {
            let user_start = PathBuf::from(&appdata)
                .join("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs");
            if user_start.exists() {
                dirs.push(user_start);
            }
        }
        // 全ユーザーのスタートメニュー
        if let Ok(programdata) = std::env::var("PROGRAMDATA") {
            let all_start = PathBuf::from(&programdata)
                .join("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs");
            if all_start.exists() {
                dirs.push(all_start);
            }
        }
    }

    dirs
}

fn scan_dir(dir: &PathBuf, out: &mut Vec<AppEntry>, depth: usize) {
    // Windows のジャンクション（例: "Application Data"）が親を指すと無限再帰し
    // スタックオーバーフローでクラッシュするため、探索深さを制限する
    if depth > 10 {
        return;
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_dir(&path, out, depth + 1);
        } else if let Some(ext) = path.extension() {
            if ext.eq_ignore_ascii_case("lnk") {
                if let Some(stem) = path.file_stem() {
                    let name = stem.to_string_lossy().to_string();
                    if !is_uninstaller(&name) {
                        out.push(AppEntry {
                            name,
                            target: AppTarget::Lnk(path.to_string_lossy().to_string()),
                        });
                    }
                }
            }
        }
    }
}

// =============================================================================
// テスト
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_apps() -> Vec<AppEntry> {
        vec![
            AppEntry {
                name: "Google Chrome".into(),
                target: AppTarget::Lnk("C:\\chrome.lnk".into()),
            },
            AppEntry {
                name: "Visual Studio Code".into(),
                target: AppTarget::Lnk("C:\\code.lnk".into()),
            },
            AppEntry {
                name: "Windows Terminal".into(),
                target: AppTarget::Lnk("C:\\wt.lnk".into()),
            },
            AppEntry {
                name: "Notepad++".into(),
                target: AppTarget::Lnk("C:\\notepad.lnk".into()),
            },
            AppEntry {
                name: "Chrome Remote Desktop".into(),
                target: AppTarget::Lnk("C:\\crd.lnk".into()),
            },
        ]
    }

    #[test]
    fn test_search_single_term() {
        let apps = sample_apps();
        let results = search(&apps, "chrome", 5);
        assert_eq!(results.len(), 2); // Google Chrome + Chrome Remote Desktop
    }

    #[test]
    fn test_search_prefix_bonus() {
        let apps = sample_apps();
        let results = search(&apps, "chrome", 5);
        // "Chrome Remote Desktop" starts with "chrome" → higher score
        // but "Google Chrome" is shorter → also high score
        assert!(!results.is_empty());
    }

    #[test]
    fn test_search_multi_term() {
        let apps = sample_apps();
        let results = search(&apps, "visual code", 5);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "Visual Studio Code");
    }

    #[test]
    fn test_search_empty() {
        let apps = sample_apps();
        assert!(search(&apps, "", 5).is_empty());
    }

    #[test]
    fn test_search_no_match() {
        let apps = sample_apps();
        assert!(search(&apps, "nonexistent", 5).is_empty());
    }

    #[test]
    fn test_search_limit() {
        let apps = sample_apps();
        let results = search(&apps, "e", 2);
        assert!(results.len() <= 2);
    }

    #[test]
    fn test_parse_start_apps_array() {
        let json = r#"[{"Name":"Snipping Tool","AppID":"Microsoft.ScreenSketch_8wekyb3d8bbwe!App"},{"Name":"Settings","AppID":"windows.immersivecontrolpanel_cw5n1h2txyewy!microsoft.windows.immersivecontrolpanel"}]"#;
        let apps = parse_start_apps(json).unwrap();
        assert_eq!(apps.len(), 2);
        assert_eq!(apps[0].name, "Snipping Tool");
        assert_eq!(
            apps[0].target,
            AppTarget::AppId("Microsoft.ScreenSketch_8wekyb3d8bbwe!App".into())
        );
    }

    #[test]
    fn test_parse_start_apps_single_object() {
        // ConvertTo-Json は 1 件だけのとき配列ではなくオブジェクトを返す
        let json = r#"{"Name":"Snipping Tool","AppID":"Microsoft.ScreenSketch_8wekyb3d8bbwe!App"}"#;
        let apps = parse_start_apps(json).unwrap();
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].name, "Snipping Tool");
    }

    #[test]
    fn test_parse_start_apps_empty_output() {
        assert!(parse_start_apps("").unwrap().is_empty());
        assert!(parse_start_apps("  \r\n").unwrap().is_empty());
    }

    #[test]
    fn test_parse_start_apps_bom() {
        let json = "\u{feff}[{\"Name\":\"A\",\"AppID\":\"a!b\"}]";
        assert_eq!(parse_start_apps(json).unwrap().len(), 1);
    }

    #[test]
    fn test_parse_start_apps_invalid() {
        assert!(parse_start_apps("not json").is_err());
    }

    #[test]
    fn test_parse_start_apps_skips_uninstall_and_blank() {
        let json = r#"[{"Name":"Uninstall Foo","AppID":"x"},{"Name":"","AppID":"y"},{"Name":"Foo","AppID":""},{"Name":"Bar","AppID":"z"}]"#;
        let apps = parse_start_apps(json).unwrap();
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].name, "Bar");
    }

    #[test]
    fn test_merge_prefers_lnk_on_same_name() {
        let lnk = vec![AppEntry {
            name: "Notepad++".into(),
            target: AppTarget::Lnk("C:\\n.lnk".into()),
        }];
        let store = vec![
            AppEntry {
                name: "notepad++".into(),
                target: AppTarget::AppId("x!y".into()),
            },
            AppEntry {
                name: "Snipping Tool".into(),
                target: AppTarget::AppId("s!t".into()),
            },
        ];
        let merged = merge_apps(lnk, store);
        assert_eq!(merged.len(), 2);
        let np = merged.iter().find(|a| a.name == "Notepad++").unwrap();
        assert!(matches!(np.target, AppTarget::Lnk(_)));
        assert!(merged.iter().any(|a| a.name == "Snipping Tool"));
    }

    #[test]
    fn test_merge_dedups_within_lnk() {
        let e = |p: &str| AppEntry {
            name: "Dup".into(),
            target: AppTarget::Lnk(p.into()),
        };
        assert_eq!(merge_apps(vec![e("a"), e("b")], vec![]).len(), 1);
    }

    #[test]
    fn test_history_key_roundtrip() {
        let lnk = AppTarget::Lnk("C:\\a.lnk".into());
        let id = AppTarget::AppId("Microsoft.ScreenSketch_8wekyb3d8bbwe!App".into());
        assert_eq!(AppTarget::from_history_key(&lnk.history_key()), lnk);
        assert_eq!(AppTarget::from_history_key(&id.history_key()), id);
        // 既存の履歴（.lnk パスのみ）はそのまま Lnk として読める
        assert_eq!(
            AppTarget::from_history_key("C:\\old.lnk"),
            AppTarget::Lnk("C:\\old.lnk".into())
        );
    }

    #[test]
    fn test_scan_no_crash() {
        let _ = scan_apps();
    }
}
