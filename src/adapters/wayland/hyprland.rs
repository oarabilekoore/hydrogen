use crate::error::{HydrogenError, Result};
use crate::ports::desktop::DesktopEnvironment;
use std::fs::File;
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

pub struct HyprlandDesktop;

impl DesktopEnvironment for HyprlandDesktop {
    fn decode_clipboard_image(&self, clip_id: &str, dest_path: &Path) -> Result<()> {
        let out_file = File::create(dest_path)?;
        let status = Command::new("cliphist")
            .args(["decode", clip_id])
            .stdout(Stdio::from(out_file))
            .status()?;
        if !status.success() {
            return Err(HydrogenError::ExternalTool("cliphist decode failed".into()));
        }
        Ok(())
    }

    fn copy_text_to_clipboard(&self, text: &str) -> Result<()> {
        let mut child = Command::new("wl-copy")
            .args(["--type", "text/plain;charset=utf-8"])
            .stdin(Stdio::piped())
            .spawn()?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(text.as_bytes())?;
        }
        let status = child.wait()?;
        if !status.success() {
            return Err(HydrogenError::ExternalTool("wl-copy failed".into()));
        }
        Ok(())
    }

    fn focus_and_paste(&self, window_addr: &str) -> Result<()> {
        let _ = Command::new("hyprctl")
            .args([
                "dispatch",
                "focuswindow",
                &format!("address:{window_addr}"),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        thread::sleep(Duration::from_millis(150));
        let _ = Command::new("wtype")
            .args(["-M", "ctrl", "v", "-m", "ctrl"])
            .status();
        Ok(())
    }

    fn notify(&self, title: &str, body: &str, urgency: &str) {
        let _ = Command::new("notify-send")
            .args([
                "-u",
                urgency,
                "-a",
                "Quickshell OCR",
                "-i",
                "accessories-character-map",
                title,
                body,
            ])
            .status();
    }

    fn open_file_detached(&self, path: &Path) -> Result<()> {
        Command::new("xdg-open")
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()?;
        Ok(())
    }

    fn open_default_calendar(&self, date: Option<&str>) -> Result<()> {
        let handler = Command::new("xdg-mime")
            .args(["query", "default", "text/calendar"])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();

        if handler.is_empty() || handler.contains("gnome-calendar") {
            let mut cmd = Command::new("gnome-calendar");
            if let Some(d) = date {
                if !d.trim().is_empty() {
                    cmd.args(["--date", d.trim()]);
                }
            }
            if cmd
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .process_group(0)
                .spawn()
                .is_ok()
            {
                return Ok(());
            }
        }

        if !handler.is_empty() {
            Command::new("gtk-launch")
                .arg(&handler)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .process_group(0)
                .spawn()?;
            return Ok(());
        }

        Err(HydrogenError::ExternalTool(
            "No default calendar application found (xdg-mime query default text/calendar is empty)"
                .into(),
        ))
    }
}
