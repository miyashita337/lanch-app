// clipboard_history.rs - クリップボード監視 & 履歴管理
//
// 責務:
// - Win32 AddClipboardFormatListener でクリップボード変更をリアルタイム監視
// - 変更検出時に arboard でテキスト/画像を読み取り、ClipboardStore に保存
// - 定期的にローテーション（1週間超のエントリを削除）
//
// アーキテクチャ:
// - バックグラウンドスレッドで隠しウィンドウを作成し、メッセージループを回す
// - Arc<Mutex<ClipboardStore>> で UI スレッドと共有

use std::sync::{Arc, Mutex};
use std::thread;

use crate::clipboard_store::ClipboardStore;

/// クリップボード履歴の保持日数（デフォルト: 7日）
const DEFAULT_RETENTION_DAYS: i64 = 7;

/// ローテーション間隔（秒）: 1時間ごとに古いエントリを削除
const ROTATION_INTERVAL_SECS: u64 = 3600;

/// 共有ストアの型エイリアス
pub type SharedStore = Arc<Mutex<ClipboardStore>>;

/// クリップボード監視を開始する
///
/// バックグラウンドスレッドを起動し、クリップボード変更を監視する。
/// 戻り値の SharedStore を clipboard_ui に渡して履歴UIから参照する。
pub fn start_monitoring() -> SharedStore {
    let store = ClipboardStore::new(DEFAULT_RETENTION_DAYS);
    let shared = Arc::new(Mutex::new(store));

    // ローテーション用スレッド
    let rotation_store = shared.clone();
    thread::spawn(move || {
        loop {
            thread::sleep(std::time::Duration::from_secs(ROTATION_INTERVAL_SECS));
            if let Ok(mut store) = rotation_store.lock() {
                store.rotate();
            }
        }
    });

    // クリップボード監視スレッド
    let monitor_store = shared.clone();
    thread::spawn(move || {
        run_clipboard_monitor(monitor_store);
    });

    shared
}

/// Windows: クリップボード監視メッセージループ
#[cfg(windows)]
fn run_clipboard_monitor(store: SharedStore) {
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::System::DataExchange::AddClipboardFormatListener;
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    // WM_CLIPBOARDUPDATE = 0x031D
    const WM_CLIPBOARDUPDATE: u32 = 0x031D;

    // ウィンドウプロシージャ用のグローバル状態
    // (ウィンドウプロシージャから SharedStore にアクセスするため)
    thread_local! {
        static THREAD_STORE: std::cell::RefCell<Option<SharedStore>> = const { std::cell::RefCell::new(None) };
    }

    unsafe extern "system" fn wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: usize,
        lparam: isize,
    ) -> isize {
        if msg == WM_CLIPBOARDUPDATE {
            THREAD_STORE.with(|cell| {
                if let Some(ref store) = *cell.borrow() {
                    on_clipboard_update(store);
                }
            });
            return 0;
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }

    // thread_local にストアをセット
    THREAD_STORE.with(|cell| {
        *cell.borrow_mut() = Some(store);
    });

    unsafe {
        let h_instance = GetModuleHandleW(std::ptr::null());

        // ウィンドウクラス登録
        let class_name: Vec<u16> = "LanchAppClipboardMonitor\0"
            .encode_utf16()
            .collect();

        let wc = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: h_instance,
            hIcon: std::ptr::null_mut(),
            hCursor: std::ptr::null_mut(),
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: class_name.as_ptr(),
        };

        if RegisterClassW(&wc) == 0 {
            eprintln!("[clipboard_history] ウィンドウクラスの登録に失敗");
            return;
        }

        // メッセージ専用ウィンドウを作成（HWND_MESSAGE = -3）
        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            std::ptr::null(),
            0,
            0,
            0,
            0,
            0,
            -3isize as HWND, // HWND_MESSAGE
            std::ptr::null_mut(),
            h_instance,
            std::ptr::null(),
        );

        if hwnd.is_null() {
            eprintln!("[clipboard_history] メッセージウィンドウの作成に失敗");
            return;
        }

        // クリップボード変更リスナーを登録
        if AddClipboardFormatListener(hwnd) == 0 {
            eprintln!("[clipboard_history] AddClipboardFormatListener に失敗");
            return;
        }

        eprintln!("[clipboard_history] クリップボード監視を開始しました");

        // メッセージループ
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// クリップボード変更時の処理
#[cfg(windows)]
fn on_clipboard_update(store: &SharedStore) {
    // arboard でクリップボード内容を読み取る
    let mut cb = match arboard::Clipboard::new() {
        Ok(cb) => cb,
        Err(_) => return,
    };

    // テキストを試行
    if let Ok(text) = cb.get_text() {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            // 内部マーカーは無視（copy_selected_text のマーカー）
            if trimmed.starts_with("__QT_MARKER_") {
                return;
            }
            if let Ok(mut s) = store.lock() {
                s.add_text(trimmed);
            }
            return;
        }
    }

    // 画像を試行
    if let Ok(img) = cb.get_image() {
        // arboard::ImageData を PNG にエンコード
        if let Some(png_data) = encode_rgba_to_png(
            &img.bytes,
            img.width as u32,
            img.height as u32,
        ) {
            if let Ok(mut s) = store.lock() {
                s.add_image(&png_data);
            }
        }
    }
}

/// RGBA バイト列を PNG にエンコード（最小限の PNG エンコーダ）
///
/// 外部クレート不要で PNG を生成する。
/// パフォーマンスよりも依存関係の少なさを優先。
#[cfg(windows)]
fn encode_rgba_to_png(rgba: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    // 簡易 PNG エンコード: 非圧縮（deflate stored blocks）
    // 完全な PNG 仕様ではないが、主要ビューアで表示可能

    let w = width as usize;
    let h = height as usize;

    if rgba.len() < w * h * 4 {
        return None;
    }

    // フィルタバイト（0 = None）を各行の先頭に追加した生データ
    let mut raw_data = Vec::with_capacity(h * (1 + w * 4));
    for y in 0..h {
        raw_data.push(0u8); // filter: None
        let row_start = y * w * 4;
        let row_end = row_start + w * 4;
        raw_data.extend_from_slice(&rgba[row_start..row_end]);
    }

    // deflate (stored, non-compressed)
    let deflated = deflate_stored(&raw_data);

    // IDAT chunk
    let idat = make_chunk(b"IDAT", &deflated);

    // IHDR
    let mut ihdr_data = Vec::with_capacity(13);
    ihdr_data.extend_from_slice(&width.to_be_bytes());
    ihdr_data.extend_from_slice(&height.to_be_bytes());
    ihdr_data.push(8); // bit depth
    ihdr_data.push(6); // color type: RGBA
    ihdr_data.push(0); // compression
    ihdr_data.push(0); // filter
    ihdr_data.push(0); // interlace
    let ihdr = make_chunk(b"IHDR", &ihdr_data);

    let iend = make_chunk(b"IEND", &[]);

    let mut png = Vec::new();
    png.extend_from_slice(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]); // PNG signature
    png.extend_from_slice(&ihdr);
    png.extend_from_slice(&idat);
    png.extend_from_slice(&iend);

    Some(png)
}

#[cfg(windows)]
fn make_chunk(chunk_type: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let length = data.len() as u32;
    let mut chunk = Vec::with_capacity(12 + data.len());
    chunk.extend_from_slice(&length.to_be_bytes());
    chunk.extend_from_slice(chunk_type);
    chunk.extend_from_slice(data);

    // CRC32 over type + data
    let mut crc_data = Vec::with_capacity(4 + data.len());
    crc_data.extend_from_slice(chunk_type);
    crc_data.extend_from_slice(data);
    let crc = crc32(&crc_data);
    chunk.extend_from_slice(&crc.to_be_bytes());

    chunk
}

#[cfg(windows)]
fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFFFFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB88320;
            } else {
                crc >>= 1;
            }
        }
    }
    crc ^ 0xFFFFFFFF
}

#[cfg(windows)]
fn deflate_stored(data: &[u8]) -> Vec<u8> {
    // zlib header + stored deflate blocks + adler32
    let mut out = Vec::new();
    out.push(0x78); // CMF: deflate, window size 32K
    out.push(0x01); // FLG: no dict, check bits

    // Split into 65535-byte blocks (max for stored blocks)
    let max_block = 65535usize;
    let mut offset = 0;

    while offset < data.len() {
        let remaining = data.len() - offset;
        let block_size = remaining.min(max_block);
        let is_last = offset + block_size >= data.len();

        out.push(if is_last { 0x01 } else { 0x00 }); // BFINAL + BTYPE=00 (stored)
        let len = block_size as u16;
        let nlen = !len;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&nlen.to_le_bytes());
        out.extend_from_slice(&data[offset..offset + block_size]);

        offset += block_size;
    }

    // Adler-32 checksum
    let adler = adler32(data);
    out.extend_from_slice(&adler.to_be_bytes());

    out
}

#[cfg(windows)]
fn adler32(data: &[u8]) -> u32 {
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// 非Windows環境用スタブ
#[cfg(not(windows))]
fn run_clipboard_monitor(_store: SharedStore) {
    eprintln!("[clipboard_history] クリップボード監視は Windows でのみ動作します");
}
