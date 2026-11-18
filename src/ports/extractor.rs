use crate::domain::document::FileEntry;
use crate::domain::ocr::layout::OcrBox;
use crate::error::Result;
use std::collections::HashMap;
use std::path::Path;

pub trait FileScanner {
    fn scan(&self, max_depth: usize, max_results: usize) -> Vec<FileEntry>;
}

pub trait TextExtractor {
    fn extract_text(&self, path: &Path, size: u64) -> String;
    fn is_indexable_image(&self, path: &Path, size: u64) -> bool;
}

pub trait OcrProvider {
    fn batch_ocr_text(&self, paths: &[String]) -> HashMap<String, String>;
    fn ocr_boxes(&self, image_path: &Path) -> Result<Vec<OcrBox>>;
}

pub trait CaptionProvider {
    fn batch_captions(&self, paths: &[String]) -> HashMap<String, String>;
}
