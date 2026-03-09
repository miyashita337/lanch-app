// notification.rs - Windows トースト通知
//
// PowerShell の System.Windows.Forms.NotifyIcon を使って
// バルーン通知を表示する。追加のクレート不要。
//
// 制約:
// - Windows 10 以降でのみ動作
// - PowerShell の起動に ~200ms かかるが、非同期なのでブロックしない

use std::process::Command;
use std::thread;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// バルーン通知を表示する（非同期・ノンブロッキング）
///
/// PowerShell を裏で起動してシステム通知を出す。
/// 失敗しても無視する（通知は必須機能ではないため）。
#[cfg(windows)]
pub fn show(title: &str, message: &str) {
    let title = title.replace('\'', "''").replace('\n', " ");
    let message = message.replace('\'', "''").replace('\n', " ");

    let script = format!(
        r#"Add-Type -AssemblyName System.Windows.Forms
$n = New-Object System.Windows.Forms.NotifyIcon
$n.Icon = [System.Drawing.SystemIcons]::Information
$n.Visible = $true
$n.ShowBalloonTip(3000, '{}', '{}', 'Info')
Start-Sleep -Seconds 4
$n.Dispose()"#,
        title, message
    );

    thread::spawn(move || {
        let _ = Command::new("powershell")
            .args([
                "-WindowStyle", "Hidden",
                "-ExecutionPolicy", "Bypass",
                "-Command", &script,
            ])
            .creation_flags(0x08000000) // CREATE_NO_WINDOW
            .spawn();
    });
}

#[cfg(not(windows))]
pub fn show(title: &str, message: &str) {
    eprintln!("[notification] {}: {}", title, message);
}

/// エラー通知を表示する
pub fn show_error(title: &str, message: &str) {
    #[cfg(windows)]
    {
        let title = title.replace('\'', "''").replace('\n', " ");
        let message = message.replace('\'', "''").replace('\n', " ");

        let script = format!(
            r#"Add-Type -AssemblyName System.Windows.Forms
$n = New-Object System.Windows.Forms.NotifyIcon
$n.Icon = [System.Drawing.SystemIcons]::Warning
$n.Visible = $true
$n.ShowBalloonTip(5000, '{}', '{}', 'Warning')
Start-Sleep -Seconds 6
$n.Dispose()"#,
            title, message
        );

        thread::spawn(move || {
            let _ = Command::new("powershell")
                .args([
                    "-WindowStyle", "Hidden",
                    "-ExecutionPolicy", "Bypass",
                    "-Command", &script,
                ])
                .creation_flags(0x08000000)
                .spawn();
        });
    }

    #[cfg(not(windows))]
    {
        eprintln!("[notification error] {}: {}", title, message);
    }
}
