use crate::domain::ocr::layout::OcrBox;
use crate::error::{HydrogenError, Result};
use crate::ports::extractor::{CaptionProvider, OcrProvider};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub struct RapidOcrWorker {
    python_bin: PathBuf,
    document_python: Option<PathBuf>,
    worker_script: Option<PathBuf>,
    engine: String,
}

impl RapidOcrWorker {
    /// Worker used by background image indexing. Keeps the lightweight RapidOCR venv.
    pub fn new(python_bin: PathBuf) -> Self {
        Self {
            python_bin,
            document_python: None,
            worker_script: None,
            engine: "rapid".into(),
        }
    }

    /// Worker used for interactive OCR. The selected engine is loaded on demand
    /// and runs in its own CPU-only environment.
    pub fn new_document(
        rapid_python: PathBuf,
        document_python: PathBuf,
        worker_script: PathBuf,
        engine: String,
    ) -> Self {
        Self {
            python_bin: rapid_python,
            document_python: Some(document_python),
            worker_script: Some(worker_script),
            engine,
        }
    }
}

impl OcrProvider for RapidOcrWorker {
    fn batch_ocr_text(&self, paths: &[String]) -> HashMap<String, String> {
        if paths.is_empty() || !self.python_bin.is_file() {
            return HashMap::new();
        }
        let code = r#"
import json, sys
from rapidocr_onnxruntime import RapidOCR
paths = json.loads(sys.stdin.read() or '[]')
ocr = RapidOCR()
out = {}
for p in paths:
    try:
        res, _ = ocr(p)
        if res:
            out[p] = ' '.join(item[1].strip() for item in res if item and len(item) >= 2 and item[1])
    except Exception:
        pass
print(json.dumps(out))
"#;
        let Ok(payload) = serde_json::to_string(paths) else {
            return HashMap::new();
        };
        let Ok(mut child) = Command::new(&self.python_bin)
            .args(["-c", code])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        else {
            return HashMap::new();
        };

        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(payload.as_bytes());
        }
        let Ok(out) = child.wait_with_output() else {
            return HashMap::new();
        };
        if out.status.success() {
            serde_json::from_slice(&out.stdout).unwrap_or_default()
        } else {
            HashMap::new()
        }
    }

    fn ocr_boxes(&self, image_path: &Path) -> Result<Vec<OcrBox>> {
        if let (Some(python), Some(script)) =
            (self.document_python.as_ref(), self.worker_script.as_ref())
        {
            if !python.is_file() {
                return Err(HydrogenError::ExternalTool(format!(
                    "Hydrogen OCR environment missing at {}. Run make ocr-deps in the Hydrogen repository.",
                    python.display()
                )));
            }
            if !script.is_file() {
                return Err(HydrogenError::ExternalTool(format!(
                    "Hydrogen OCR worker missing at {}. Reinstall Hydrogen with make install.",
                    script.display()
                )));
            }

            let output = Command::new(python)
                .arg(script)
                .args(["--engine", self.engine.as_str(), "--image"])
                .arg(image_path)
                .output()?;

            if !output.status.success() {
                let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
                return Err(HydrogenError::ExternalTool(if err.is_empty() {
                    "Hydrogen OCR worker failed".into()
                } else {
                    err
                }));
            }

            return Ok(serde_json::from_slice(&output.stdout)?);
        }

        // Compatibility path for callers that only construct the legacy worker.
        if !self.python_bin.is_file() {
            return Err(HydrogenError::ExternalTool(format!(
                "RapidOCR venv missing at {}",
                self.python_bin.display()
            )));
        }
        let code = r#"
import json, sys
from rapidocr_onnxruntime import RapidOCR
res, _ = RapidOCR()(sys.argv[1])
out = []
if res:
    for box, text, score in res:
        out.append({"box_pts": [[float(pt[0]), float(pt[1])] for pt in box], "text": str(text), "score": float(score)})
print(json.dumps(out))
"#;
        let output = Command::new(&self.python_bin)
            .args(["-c", code])
            .arg(image_path)
            .output()?;

        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(HydrogenError::ExternalTool(if err.is_empty() {
                "RapidOCR process failed".into()
            } else {
                err
            }));
        }

        let boxes: Vec<OcrBox> = serde_json::from_slice(&output.stdout)?;
        Ok(boxes)
    }
}

pub struct CaptionWorker {
    python_bin: PathBuf,
    script_path: PathBuf,
}

impl CaptionWorker {
    pub fn new(python_bin: PathBuf, script_path: PathBuf) -> Self {
        Self {
            python_bin,
            script_path,
        }
    }
}

impl CaptionProvider for CaptionWorker {
    fn batch_captions(&self, paths: &[String]) -> HashMap<String, String> {
        if paths.is_empty() || !self.script_path.is_file() {
            return HashMap::new();
        }
        let py = if self.python_bin.is_file() {
            self.python_bin.as_os_str()
        } else {
            std::ffi::OsStr::new("python3")
        };

        let Ok(payload) = serde_json::to_string(paths) else {
            return HashMap::new();
        };
        let Ok(mut child) = Command::new(py)
            .arg(&self.script_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        else {
            return HashMap::new();
        };

        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(payload.as_bytes());
        }
        let Ok(out) = child.wait_with_output() else {
            return HashMap::new();
        };
        if out.status.success() {
            serde_json::from_slice(&out.stdout).unwrap_or_default()
        } else {
            HashMap::new()
        }
    }
}
