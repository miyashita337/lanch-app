## 概要
Alfred の Clipboard History に相当する機能を lanch-app に統合する。
テキスト・画像・バイナリ・JSON 等あらゆるクリップボード形式を保持し、検索・ページャー付きで履歴を参照できるようにする。

## 既存ツール調査結果
| ツール | 評価 |
|---|---|
| Windows標準 (Win+V) | 25件まで、再起動で消失、検索なし → 不十分 |
| Ditto (OSS/C++) | 機能的に最も近い。検索(正規表現)、画像サムネ、無制限保持 → ただし別アプリ |
| CopyQ (OSS/Qt) | タブ+スクリプト機能豊富 → ただし別アプリ |

**自前実装の利点**: lanch-app統合（別アプリ不要）、翻訳→整形→履歴が1アプリで完結、日本語日時命名規則

## 機能要件

### クリップボード監視
- [ ] Win32 `AddClipboardFormatListener` でリアルタイム監視
- [ ] `clipboard-win` クレート (monitor feature) + `arboard` を使用
- [ ] テキスト、画像(PNG/BMP)、ファイルリスト、JSON、カスタム形式を保持

### 履歴ストレージ
- [ ] 保持期間: 最大1週間、ローテーションで古いものから削除
- [ ] バイナリ（画像等）のファイル名フォーマット: `Image YYYY-MM-DD hh:mm:ss`
- [ ] ストレージ: `~/.lanch-app/clipboard-history/` にJSON index + バイナリファイル
- [ ] メモリ効率: インデックスのみメモリ保持、バイナリはディスクから遅延読み込み

### 検索UI（egui ポップアップ）
- [ ] ホットキー（例: `Ctrl+Shift+V`）で履歴ウィンドウを開く
- [ ] 検索窓: キーワードでテキスト内容を検索
- [ ] 日付検索: `YYYY-MM-DD` フォーマットでバイナリを日付絞り込み
- [ ] テキストはプレビュー表示、画像はサムネイル表示
- [ ] 選択でクリップボードにコピー & ウィンドウを閉じる

### ページャー
- [ ] 1ページ100件表示
- [ ] 末尾に「...see more」リンクで次の100件を表示
- [ ] 逆順（新しい順）でスクロール

### ホットキー統合
- [ ] config.json に `hotkey_clipboard_history` を追加
- [ ] デフォルト: `ctrl+shift+v` (Windows標準と同じキー)
- [ ] tray.rs のホットキー登録に追加

## 技術設計
```
src/
├── clipboard_history.rs   # 履歴管理（監視、保存、ローテーション）
├── clipboard_store.rs     # ストレージ（JSON index + バイナリファイル）
├── clipboard_ui.rs        # egui 検索UI + ページャー
└── (tray.rs に統合)       # ホットキー登録
```

### 依存クレート追加
- `clipboard-win` (features = ["monitor"]) — クリップボード監視
- `chrono` — タイムスタンプ管理
- `image` (optional) — サムネイル生成

## 参考
- [Ditto Clipboard Manager](https://ditto-cp.sourceforge.io/)
- [CopyQ](https://github.com/hluk/CopyQ)
- [clipboard-win crate](https://crates.io/crates/clipboard-win)
- [Win32 AddClipboardFormatListener](https://learn.microsoft.com/en-us/windows/win32/dataxchg/using-the-clipboard)
