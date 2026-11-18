use crate::domain::ocr::layout::format_markdown;
use crate::error::{HydrogenError, Result};
use crate::ports::desktop::DesktopEnvironment;
use crate::ports::extractor::OcrProvider;
use std::env;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct OcrService<'a> {
    ocr: &'a dyn OcrProvider,
    desktop: &'a dyn DesktopEnvironment,
}

impl<'a> OcrService<'a> {
    pub fn new(ocr: &'a dyn OcrProvider, desktop: &'a dyn DesktopEnvironment) -> Self {
        Self { ocr, desktop }
    }

    pub fn ocr_clipboard(&self, clip_id: &str, window_addr: &str) -> Result<()> {
        if clip_id.trim().is_empty() {
            self.desktop
                .notify("OCR Failed", "No clipboard ID provided.", "critical");
            return Err(HydrogenError::InvalidInput("empty clip_id".into()));
        }

        let runtime_dir = env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
        let ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let tmp_img = Path::new(&runtime_dir).join(format!("qs_ocr_{clip_id}_{ms}.png"));

        let res = (|| -> Result<()> {
            self.desktop.decode_clipboard_image(clip_id, &tmp_img)?;
            self.process_image(&tmp_img, window_addr)
        })();

        let _ = fs::remove_file(&tmp_img);
        if let Err(ref e) = res {
            let msg: String = e.to_string().chars().take(140).collect();
            self.desktop.notify("OCR Failed", &msg, "critical");
        }
        res
    }

    pub fn ocr_file(&self, path: &Path, window_addr: &str) -> Result<()> {
        if !path.is_file() {
            let msg = format!("File not found: {}", path.display());
            self.desktop.notify("OCR Failed", &msg, "critical");
            return Err(HydrogenError::InvalidInput(msg));
        }

        let res = self.process_image(path, window_addr);
        if let Err(ref e) = res {
            let msg: String = e.to_string().chars().take(140).collect();
            self.desktop.notify("OCR Failed", &msg, "critical");
        }
        res
    }

    fn process_image(&self, image_path: &Path, window_addr: &str) -> Result<()> {
        let boxes = self.ocr.ocr_boxes(image_path)?;
        let markdown = format_markdown(&boxes);

        if markdown.is_empty() {
            self.desktop
                .notify("OCR Warning", "No text detected in image.", "normal");
            return Ok(());
        }

        self.desktop.copy_text_to_clipboard(&markdown)?;
        if !window_addr.trim().is_empty() {
            self.desktop.focus_and_paste(window_addr.trim())?;
        }

        let n = markdown.split("\n\n").count();
        let label = if n == 1 { "line" } else { "lines" };
        self.desktop.notify(
            "OCR Complete",
            &format!("Pasted Markdown ({n} {label})."),
            "normal",
        );
        Ok(())
    }
}
