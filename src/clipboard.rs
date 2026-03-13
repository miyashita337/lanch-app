// clipboard.rs - クリップボード操作 & キー入力シミュレーション
//
// 選択テキスト翻訳 / Markdown整形 のフロー:
// 1. ホットキーの修飾キー（Ctrl+Shift等）が物理的に離されるのを待つ
// 2. keybd_event で Ctrl+C をシミュレーション（選択テキストをコピー）
// 3. 少し待つ（クリップボードが更新されるのを待つ）
// 4. arboard でクリップボードからテキストを読む

use std::thread;
use std::time::Duration;

// Win32 API
#[cfg(windows)]
extern "system" {
    fn GetAsyncKeyState(vKey: i32) -> i16;
}

/// キーイベントのフラグ
const KEYEVENTF_KEYUP: u32 = 0x0002;

/// 仮想キーコード
const VK_CONTROL: u8 = 0x11;
const VK_C: u8 = 0x43;
const VK_INSERT: u8 = 0x2D;
const VK_ESCAPE: u8 = 0x1B;
const VK_MENU: u8 = 0x12;    // Alt (generic)
const VK_LMENU: u8 = 0xA4;   // 左 Alt
const VK_RMENU: u8 = 0xA5;   // 右 Alt
const VK_SHIFT: u8 = 0x10;   // Shift (generic)
const VK_LSHIFT: u8 = 0xA0;  // 左 Shift
const VK_RSHIFT: u8 = 0xA1;  // 右 Shift
const VK_LCONTROL: u8 = 0xA2;// 左 Ctrl
const VK_RCONTROL: u8 = 0xA3;// 右 Ctrl

/// 指定キーが物理的に押されているかチェック
#[cfg(windows)]
fn is_key_pressed(vk: i32) -> bool {
    unsafe { GetAsyncKeyState(vk) & (0x8000u16 as i16) != 0 }
}

/// 全ての修飾キー（Alt/Ctrl/Shift）が物理的に離されるまで待つ
#[cfg(windows)]
fn wait_for_modifiers_release() {
    let start = std::time::Instant::now();
    loop {
        let alt_pressed = is_key_pressed(VK_MENU as i32)
            || is_key_pressed(VK_LMENU as i32)
            || is_key_pressed(VK_RMENU as i32);
        let ctrl_pressed = is_key_pressed(VK_CONTROL as i32)
            || is_key_pressed(VK_LCONTROL as i32)
            || is_key_pressed(VK_RCONTROL as i32);
        let shift_pressed = is_key_pressed(VK_SHIFT as i32)
            || is_key_pressed(VK_LSHIFT as i32)
            || is_key_pressed(VK_RSHIFT as i32);

        if !alt_pressed && !ctrl_pressed && !shift_pressed {
            break;
        }

        if start.elapsed().as_millis() > 1000 {
            eprintln!("[clipboard] 修飾キーの解放待ちがタイムアウトしました");
            break;
        }

        thread::sleep(Duration::from_millis(10));
    }
    thread::sleep(Duration::from_millis(50));
}

/// Ctrl+C をシミュレートして選択テキストをクリップボードにコピーする
#[cfg(windows)]
pub fn simulate_copy() {
    let alt_was_pressed = is_key_pressed(VK_MENU as i32)
        || is_key_pressed(VK_LMENU as i32)
        || is_key_pressed(VK_RMENU as i32);

    wait_for_modifiers_release();

    if alt_was_pressed {
        let _ = send_key(VK_ESCAPE);
        thread::sleep(Duration::from_millis(30));
    }

    let _ = send_ctrl_combo(VK_C);
}

#[cfg(not(windows))]
pub fn simulate_copy() {
    eprintln!("simulate_copy は Windows でのみ動作します");
}

fn debug_enabled() -> bool {
    std::env::var("QT_DEBUG").ok().as_deref() == Some("1")
}

fn debug_log(msg: &str) {
    if debug_enabled() {
        eprintln!("[clipboard] {}", msg);
    }
}

#[cfg(windows)]
fn get_foreground_process_info() -> Option<(u32, String)> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return None;
        }

        let mut pid: u32 = 0;
        let _ = GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == 0 {
            return None;
        }

        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return Some((pid, "<open process failed>".to_string()));
        }

        let mut buf = [0u16; 260];
        let mut size = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut size) != 0;
        let _ = CloseHandle(handle);

        if !ok || size == 0 {
            return Some((pid, "<query image failed>".to_string()));
        }

        let full_path = String::from_utf16_lossy(&buf[..size as usize]);
        let name = std::path::Path::new(&full_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("<unknown>")
            .to_string();
        Some((pid, name))
    }
}

#[cfg(windows)]
fn is_process_elevated(pid: u32) -> Option<bool> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return None;
        }

        let mut token = std::ptr::null_mut();
        if OpenProcessToken(process, TOKEN_QUERY, &mut token) == 0 {
            let _ = CloseHandle(process);
            return None;
        }

        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut out_size = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            &mut elevation as *mut _ as *mut _,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut out_size,
        ) != 0;

        let _ = CloseHandle(token);
        let _ = CloseHandle(process);

        if ok {
            Some(elevation.TokenIsElevated != 0)
        } else {
            None
        }
    }
}

#[cfg(windows)]
fn is_current_process_elevated() -> Option<bool> {
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    is_process_elevated(unsafe { GetCurrentProcessId() })
}

#[cfg(windows)]
fn debug_log_target_context() {
    if !debug_enabled() {
        return;
    }

    let self_elev = is_current_process_elevated()
        .map(|b| b.to_string())
        .unwrap_or_else(|| "unknown".to_string());

    match get_foreground_process_info() {
        Some((pid, name)) => {
            let target_elev = is_process_elevated(pid)
                .map(|b| b.to_string())
                .unwrap_or_else(|| "unknown".to_string());
            eprintln!(
                "[clipboard] target={} pid={} target_elevated={} self_elevated={}",
                name, pid, target_elev, self_elev
            );
        }
        None => {
            eprintln!(
                "[clipboard] target=<unknown> pid=0 target_elevated=unknown self_elevated={}",
                self_elev
            );
        }
    }
}

#[cfg(not(windows))]
fn debug_log_target_context() {}

#[cfg(windows)]
fn send_ctrl_combo(key: u8) -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
    };

    unsafe {
        let mut inputs = [
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VK_CONTROL as u16,
                        wScan: 0,
                        dwFlags: 0,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            },
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: key as u16,
                        wScan: 0,
                        dwFlags: 0,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            },
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: key as u16,
                        wScan: 0,
                        dwFlags: KEYEVENTF_KEYUP,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            },
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VK_CONTROL as u16,
                        wScan: 0,
                        dwFlags: KEYEVENTF_KEYUP,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            },
        ];

        let sent = SendInput(
            inputs.len() as u32,
            inputs.as_mut_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        );
        sent == inputs.len() as u32
    }
}

#[cfg(not(windows))]
fn send_ctrl_combo(_key: u8) -> bool {
    false
}

#[cfg(windows)]
fn send_key(key: u8) -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
    };

    unsafe {
        let mut inputs = [
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: key as u16,
                        wScan: 0,
                        dwFlags: 0,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            },
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: key as u16,
                        wScan: 0,
                        dwFlags: KEYEVENTF_KEYUP,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            },
        ];

        let sent = SendInput(
            inputs.len() as u32,
            inputs.as_mut_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        );
        sent == inputs.len() as u32
    }
}

#[cfg(not(windows))]
fn send_key(_key: u8) -> bool {
    false
}

/// 指定ウィンドウにフォーカスを戻してCtrl+Vをシミュレートする
#[cfg(windows)]
pub fn restore_focus_and_paste(hwnd: isize) {
    use windows_sys::Win32::UI::WindowsAndMessaging::SetForegroundWindow;

    unsafe {
        SetForegroundWindow(hwnd as _);
    }
    thread::sleep(Duration::from_millis(100));
    let _ = send_ctrl_combo(VK_V);
}

#[cfg(not(windows))]
pub fn restore_focus_and_paste(_hwnd: isize) {}

/// 現在のフォアグラウンドウィンドウのハンドルを取得する
#[cfg(windows)]
pub fn get_foreground_hwnd() -> isize {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    unsafe { GetForegroundWindow() as isize }
}

#[cfg(not(windows))]
pub fn get_foreground_hwnd() -> isize {
    0
}

const VK_V: u8 = 0x56;

/// 選択テキストをコピーしてクリップボードから読み取る
///
/// 1. 既存クリップボードを保存
/// 2. クリップボードをクリア（コピー成功/失敗を判別するため）
/// 3. Ctrl+C をシミュレート
/// 4. クリップボードからテキストを読む
/// 5. 元のクリップボードを復元
pub fn copy_selected_text() -> Option<String> {
    debug_log_target_context();

    let old_clipboard = arboard::Clipboard::new()
        .ok()
        .and_then(|mut cb| cb.get_text().ok());

    let marker = format!(
        "__QT_MARKER_{}_{}__",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    );

    let mut marker_set = false;
    for _ in 0..10 {
        if let Ok(mut cb) = arboard::Clipboard::new() {
            if cb.set_text(&marker).is_ok() {
                marker_set = true;
                break;
            }
        }
        thread::sleep(Duration::from_millis(20));
    }

    if !marker_set {
        debug_log("marker の書き込みに失敗しました（他アプリがクリップボードを使用中の可能性）");
    }
    thread::sleep(Duration::from_millis(40));

    for attempt in 1..=3 {
        debug_log(&format!("copy retry attempt {}", attempt));

        simulate_copy();

        if !send_ctrl_combo(VK_INSERT) {
            debug_log("SendInput(Ctrl+Insert) の送信に失敗しました");
        }

        thread::sleep(Duration::from_millis(80));

        let start = std::time::Instant::now();
        while start.elapsed().as_millis() < 1200 {
            let new_text = arboard::Clipboard::new().ok().and_then(|mut cb| cb.get_text().ok());

            if let Some(text) = new_text {
                let trimmed = text.trim();
                let marker_not_replaced = marker_set && trimmed == marker;
                if !trimmed.is_empty() && !marker_not_replaced {
                    if let Some(old) = &old_clipboard {
                        if let Ok(mut cb) = arboard::Clipboard::new() {
                            let _ = cb.set_text(old);
                        }
                    }
                    return Some(trimmed.to_string());
                }
            }

            thread::sleep(Duration::from_millis(30));
        }

        thread::sleep(Duration::from_millis(50));
    }

    if let Some(old) = &old_clipboard {
        if let Ok(mut cb) = arboard::Clipboard::new() {
            let _ = cb.set_text(old);
        }
    }

    debug_log("コピー結果が取得できませんでした");
    None
}
