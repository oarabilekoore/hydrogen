use crate::domain::ocr::layout::format_markdown;
use crate::error::{GoodSearchError, Result};
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
        self.with_clipboard_image(clip_id, |path| self.process_image(path, window_addr))
    }

    pub fn ocr_clipboard_math(&self, clip_id: &str, window_addr: &str) -> Result<()> {
        self.with_clipboard_image(clip_id, |path| self.process_math_image(path, window_addr))
    }

    fn with_clipboard_image<F>(&self, clip_id: &str, process: F) -> Result<()>
    where
        F: FnOnce(&Path) -> Result<()>,
    {
        if clip_id.trim().is_empty() {
            self.desktop
                .notify("OCR Failed", "No clipboard ID provided.", "critical");
            return Err(GoodSearchError::InvalidInput("empty clip_id".into()));
        }
        let runtime_dir = env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
        let ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let tmp_img = Path::new(&runtime_dir).join(format!("qs_ocr_{clip_id}_{ms}.png"));
        let res = (|| -> Result<()> {
            self.desktop.decode_clipboard_image(clip_id, &tmp_img)?;
            process(&tmp_img)
        })();
        let _ = fs::remove_file(&tmp_img);
        if let Err(ref e) = res {
            let msg: String = e.to_string().chars().take(140).collect();
            self.desktop.notify("OCR Failed", &msg, "critical");
        }
        res
    }

    pub fn ocr_file(&self, path: &Path, window_addr: &str) -> Result<()> {
        self.process_file(path, window_addr, |p| self.process_image(p, window_addr))
    }

    pub fn ocr_file_math(&self, path: &Path, window_addr: &str) -> Result<()> {
        self.process_file(path, window_addr, |p| {
            self.process_math_image(p, window_addr)
        })
    }

    fn process_file<F>(&self, path: &Path, _window_addr: &str, process: F) -> Result<()>
    where
        F: FnOnce(&Path) -> Result<()>,
    {
        if !path.is_file() {
            let msg = format!("File not found: {}", path.display());
            self.desktop.notify("OCR Failed", &msg, "critical");
            return Err(GoodSearchError::InvalidInput(msg));
        }
        let res = process(path);
        if let Err(ref e) = res {
            let msg: String = e.to_string().chars().take(140).collect();
            self.desktop.notify("OCR Failed", &msg, "critical");
        }
        res
    }

    fn process_image(&self, image_path: &Path, window_addr: &str) -> Result<()> {
        let markdown = format_markdown(&self.ocr.ocr_boxes(image_path)?);
        self.copy_and_paste(&markdown, window_addr, "OCR Complete")
    }

    fn process_math_image(&self, image_path: &Path, window_addr: &str) -> Result<()> {
        let latex = self.ocr.ocr_math_latex(image_path)?;
        let latex = latex.trim();
        if latex.is_empty() {
            self.desktop.notify(
                "OCR Warning",
                "No mathematical expression detected.",
                "normal",
            );
            return Ok(());
        }
        let markdown = format!("$$\n{latex}\n$$");
        self.copy_and_paste(&markdown, window_addr, "Math OCR Complete")
    }

    fn copy_and_paste(&self, text: &str, window_addr: &str, title: &str) -> Result<()> {
        if text.trim().is_empty() {
            self.desktop
                .notify("OCR Warning", "No text detected in image.", "normal");
            return Ok(());
        }
        self.desktop.copy_text_to_clipboard(text)?;
        if !window_addr.trim().is_empty() {
            self.desktop.focus_and_paste(window_addr.trim())?;
        }
        self.desktop
            .notify(title, "Copied OCR result to clipboard.", "normal");
        Ok(())
    }
}
