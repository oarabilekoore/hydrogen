use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrBox {
    pub box_pts: Vec<[f64; 2]>,
    pub text: String,
    pub score: f64,
}

/// Reconstruct spatial OCR bounding boxes into clean Markdown paragraphs.
pub fn format_markdown(boxes: &[OcrBox]) -> String {
    if boxes.is_empty() {
        return String::new();
    }

    #[derive(Clone)]
    struct Item {
        y_top: f64,
        x_left: f64,
        height: f64,
        text: String,
    }

    let mut items: Vec<Item> = boxes
        .iter()
        .filter_map(|b| {
            let trimmed = b.text.trim();
            if trimmed.is_empty() || b.box_pts.is_empty() {
                return None;
            }
            let y_top = b
                .box_pts
                .iter()
                .map(|p| p[1])
                .fold(f64::INFINITY, f64::min);
            let y_bot = b
                .box_pts
                .iter()
                .map(|p| p[1])
                .fold(f64::NEG_INFINITY, f64::max);
            let x_left = b
                .box_pts
                .iter()
                .map(|p| p[0])
                .fold(f64::INFINITY, f64::min);
            let height = (y_bot - y_top).max(1.0);
            Some(Item {
                y_top,
                x_left,
                height,
                text: trimmed.to_string(),
            })
        })
        .collect();

    items.sort_by(|a, b| {
        a.y_top
            .total_cmp(&b.y_top)
            .then_with(|| a.x_left.total_cmp(&b.x_left))
    });

    let mut lines: Vec<String> = Vec::new();
    let mut current_line: Vec<(f64, String)> = Vec::new();
    let mut current_y: Option<f64> = None;
    let mut current_h: f64 = 1.0;

    for item in items {
        match current_y {
            None => {
                current_y = Some(item.y_top);
                current_h = item.height;
                current_line.push((item.x_left, item.text));
            }
            Some(cy) if (item.y_top - cy).abs() <= current_h.max(item.height) * 0.55 => {
                current_line.push((item.x_left, item.text));
            }
            Some(_) => {
                current_line.sort_by(|a, b| a.0.total_cmp(&b.0));
                let joined = current_line
                    .iter()
                    .map(|(_, t)| t.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                lines.push(joined);
                current_line.clear();
                current_y = Some(item.y_top);
                current_h = item.height;
                current_line.push((item.x_left, item.text));
            }
        }
    }

    if !current_line.is_empty() {
        current_line.sort_by(|a, b| a.0.total_cmp(&b.0));
        let joined = current_line
            .iter()
            .map(|(_, t)| t.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        lines.push(joined);
    }

    lines.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_same_line_boxes_horizontally() {
        let boxes = vec![
            OcrBox {
                box_pts: vec![[120.0, 10.0], [200.0, 10.0], [200.0, 30.0], [120.0, 30.0]],
                text: "World".into(),
                score: 0.99,
            },
            OcrBox {
                box_pts: vec![[10.0, 12.0], [90.0, 12.0], [90.0, 32.0], [10.0, 32.0]],
                text: "Hello".into(),
                score: 0.99,
            },
            OcrBox {
                box_pts: vec![[10.0, 80.0], [150.0, 80.0], [150.0, 100.0], [10.0, 100.0]],
                text: "Second line".into(),
                score: 0.99,
            },
        ];
        assert_eq!(format_markdown(&boxes), "Hello World\n\nSecond line");
    }
}
