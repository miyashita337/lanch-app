# lanch-app GitHub Issue 一括作成スクリプト
#
# 前提: gh CLI がインストール済み & ログイン済み
#   winget install GitHub.cli
#   gh auth login
#
# 実行方法:
#   cd lanch-app
#   powershell -ExecutionPolicy Bypass -File scripts/create-issues.ps1

$ErrorActionPreference = "Stop"
$repo = "miyashita337/lanch-app"

Write-Host "=== lanch-app Issue 一括作成 ===" -ForegroundColor Cyan
Write-Host "Repository: $repo"
Write-Host ""

# gh CLI チェック
try {
    $null = gh --version
} catch {
    Write-Host "ERROR: gh CLI がインストールされていません" -ForegroundColor Red
    Write-Host "  winget install GitHub.cli"
    Write-Host "  gh auth login"
    exit 1
}

# 認証チェック
$authStatus = gh auth status 2>&1
if ($LASTEXITCODE -ne 0) {
    Write-Host "ERROR: gh にログインしていません" -ForegroundColor Red
    Write-Host "  gh auth login"
    exit 1
}

# --- Issue定義 ---
$issues = @(
    @{
        title = "docs: README.md を Claude CLI 方式に更新"
        labels = "documentation"
        body = @"
## 概要
README.md がまだ旧 API キー方式の記述になっている。
ハイブリッド方式（API直接 + Claude CLI フォールバック）に更新する。

## やること
- [ ] セットアップ手順を更新（ANTHROPIC_API_KEY は任意、Claude CLIフォールバック説明追加）
- [ ] Markdown整形の動作フロー説明をハイブリッド方式に更新
- [ ] プロジェクト構成の formatter.rs 説明を更新
- [ ] config.json の説明で claude_model のデフォルトが haiku に変わったことを反映
"@
    },
    @{
        title = "fix: main.rs のヘルプテキストが旧方式のまま"
        labels = "bug"
        body = @"
## 概要
``lanch-app --help`` の出力にまだ「Claude API: 環境変数 ANTHROPIC_API_KEY を設定」と表示される。

## 該当箇所
src/main.rs L124

## やること
- [ ] ヘルプテキストをハイブリッド方式の説明に更新
- [ ] 古い quick-tools の名称参照をすべて lanch-app に統一（コメント含む）
"@
    },
    @{
        title = "test: ユニットテスト・結合テストの追加"
        labels = "enhancement"
        body = @"
## 概要
現在テストが一切ない。基本的なテストを追加する。

## やること
- [ ] config.rs: デフォルト設定の生成、JSON読み書きのテスト
- [ ] lang.rs: 日本語判定の正確性テスト
- [ ] formatter.rs: 空文字列入力、バックエンド検出のテスト
- [ ] notification.rs: sanitize_for_balloon のエッジケーステスト
- [ ] tray.rs: parse_hotkey のテスト
- [ ] CI (GitHub Actions) でのテスト自動実行
"@
    },
    @{
        title = "feat: Windows ログイン時の自動起動対応"
        labels = "enhancement"
        body = @"
## 概要
PCを起動するたびに手動で lanch-app を起動する必要がある。

## やること
- [ ] レジストリ（HKCU\Software\Microsoft\Windows\CurrentVersion\Run）に登録する機能
- [ ] config.json に auto_start: bool オプション追加
- [ ] トレイメニューに「自動起動 ON/OFF」トグル追加
"@
    },
    @{
        title = "feat: ホットキー競合時の自動代替キー提案"
        labels = "enhancement"
        body = @"
## 概要
ホットキーが他アプリと競合した場合、警告のみで機能が無効化される。

## 現状
- 競合: 警告ログ → その機能は使えない
- 設定変更: config.json 手動編集が必要

## やること
- [ ] 競合検出時に代替ホットキーを自動試行する仕組み
- [ ] トレイメニューからホットキー設定を変更できるサブメニュー
- [ ] hotkey_selected のデフォルトが config によって alt+z になる場合がある問題を修正
"@
    },
    @{
        title = "improve: Windows通知を PowerShell から WinRT に移行"
        labels = "enhancement"
        body = @"
## 概要
現在の通知はPowerShellプロセスを毎回起動する方式で約200msのオーバーヘッドがある。

## やること
- [ ] windows-sys クレートで Shell_NotifyIconW を直接呼ぶ方式に変更
- [ ] または winrt-notification クレートの導入を検討
- [ ] PowerShell方式をフォールバックとして残す
"@
    },
    @{
        title = "fix: CLI モード（--format, --translate）の動作検証と修正"
        labels = "bug"
        body = @"
## 概要
``lanch-app --format "text"`` のCLIモードがハイブリッド方式移行後に正しく動作するか未検証。

## やること
- [ ] --format の動作確認（API直接 / CLI両方）
- [ ] --translate の動作確認
- [ ] CLAUDECODE 環境変数がある場合のハンドリング（ネスト防止）
- [ ] エラー時の終了コードを適切に設定
"@
    },
    @{
        title = "chore: config.json から claude_api_key を段階的に廃止"
        labels = ""
        body = @"
## 概要
ハイブリッド方式では ANTHROPIC_API_KEY 環境変数を直接参照するため、
config.json の claude_api_key フィールドは不要になった。

## やること
- [ ] 新規生成時に claude_api_key を含めない
- [ ] 既存config読み込み時の後方互換は維持
- [ ] マイグレーション時の警告メッセージ追加
"@
    }
)

# --- Issue作成 ---
$created = 0
foreach ($issue in $issues) {
    Write-Host -NoNewline "  Creating: $($issue.title) ... "

    $args = @("issue", "create", "--repo", $repo, "--title", $issue.title, "--body", $issue.body)
    if ($issue.labels -ne "") {
        $args += "--label"
        $args += $issue.labels
    }

    try {
        $result = & gh @args 2>&1
        if ($LASTEXITCODE -eq 0) {
            Write-Host "OK $result" -ForegroundColor Green
            $created++
        } else {
            Write-Host "FAIL" -ForegroundColor Red
            Write-Host "    $result" -ForegroundColor Yellow
        }
    } catch {
        Write-Host "ERROR: $_" -ForegroundColor Red
    }

    Start-Sleep -Seconds 1
}

Write-Host ""
Write-Host "=== 完了: $created / $($issues.Count) 件作成 ===" -ForegroundColor Cyan
