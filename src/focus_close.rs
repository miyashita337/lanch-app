// フォーカス喪失で閉じるポップアップ（ランチャー・クリップボード履歴）の共通判定
//
// Snipping Tool はフォーカスを奪ってから画面を撮るため、フォーカス喪失で即座に閉じると
// ポップアップが写らない。前面が画面キャプチャツールの間は閉じず、それ以外へ移ったときも
// 少し待ってから閉じる。

use eframe::egui;
use std::time::{Duration, Instant};

/// フォーカスを失ってから閉じるまでの猶予
const CLOSE_DELAY: Duration = Duration::from_secs(1);
/// キャプチャツールが前面の間、前面ウィンドウを確認し直す間隔
/// （非フォーカス中は入力イベントが来ず再描画されないため、明示的に起こす）
const CAPTURE_POLL: Duration = Duration::from_millis(250);

/// 前面にあるとき閉じないプロセス（Win11 の Snipping Tool と、Win+Shift+S の切り取り画面）
const SCREEN_CAPTURE_PROCESSES: [&str; 2] = ["SnippingTool.exe", "ScreenClippingHost.exe"];

#[derive(Default)]
pub struct FocusLossClose {
    lost_at: Option<Instant>,
}

impl FocusLossClose {
    /// 前面に戻ったら猶予の計測をやり直す
    pub fn on_focused(&mut self) {
        self.lost_at = None;
    }

    /// 前面から外れている間に毎フレーム呼ぶ。閉じるべきなら true
    pub fn on_unfocused(&mut self, ctx: &egui::Context) -> bool {
        if screen_capture_in_foreground() {
            self.lost_at = None;
            ctx.request_repaint_after(CAPTURE_POLL);
            return false;
        }
        let lost_at = *self.lost_at.get_or_insert_with(Instant::now);
        let remaining = CLOSE_DELAY.saturating_sub(lost_at.elapsed());
        if remaining.is_zero() {
            return true;
        }
        ctx.request_repaint_after(remaining);
        false
    }
}

fn screen_capture_in_foreground() -> bool {
    crate::clipboard::get_foreground_process_info()
        .is_some_and(|(_, name)| is_screen_capture_process(&name))
}

fn is_screen_capture_process(name: &str) -> bool {
    SCREEN_CAPTURE_PROCESSES
        .iter()
        .any(|p| p.eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snipping_tool_is_screen_capture() {
        assert!(is_screen_capture_process("SnippingTool.exe"));
        assert!(is_screen_capture_process("screenclippinghost.exe"));
    }

    #[test]
    fn other_apps_are_not_screen_capture() {
        assert!(!is_screen_capture_process("chrome.exe"));
        assert!(!is_screen_capture_process("<query image failed>"));
    }
}
