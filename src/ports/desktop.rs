use crate::error::Result;
use std::path::Path;

pub trait DesktopEnvironment {
    fn decode_clipboard_image(&self, clip_id: &str, dest_path: &Path) -> Result<()>;
    fn copy_text_to_clipboard(&self, text: &str) -> Result<()>;
    fn focus_and_paste(&self, window_addr: &str) -> Result<()>;
    fn notify(&self, title: &str, body: &str, urgency: &str);
    fn open_file_detached(&self, path: &Path) -> Result<()>;
    fn open_default_calendar(&self, date: Option<&str>) -> Result<()>;
}
