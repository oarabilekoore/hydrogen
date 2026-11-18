/// Extract stable searchable terms from mathematical notation and LaTeX.
/// This is lexical normalization, not equation recognition: OCR supplies LaTeX.
pub fn extract_math_tokens(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            let mut command = String::new();
            while let Some(next) = chars.peek().copied() {
                if next.is_alphabetic() {
                    command.push(next.to_ascii_lowercase());
                    chars.next();
                } else { break; }
            }
            if !command.is_empty() { out.push(format!("math:{command}")); }
            continue;
        }
        let token = match ch {
            'α' | 'Α' => Some("greek:alpha"), 'β' | 'Β' => Some("greek:beta"),
            'γ' | 'Γ' => Some("greek:gamma"), 'δ' | 'Δ' => Some("greek:delta"),
            'θ' | 'Θ' => Some("greek:theta"), 'λ' | 'Λ' => Some("greek:lambda"),
            'μ' | 'Μ' => Some("greek:mu"), 'π' | 'Π' => Some("greek:pi"),
            'ρ' | 'Ρ' => Some("greek:rho"), 'σ' | 'Σ' => Some("greek:sigma"),
            'τ' | 'Τ' => Some("greek:tau"), 'φ' | 'Φ' => Some("greek:phi"),
            'ω' | 'Ω' => Some("greek:omega"), '∫' => Some("op:integral"),
            '∑' => Some("op:sum"), '∏' => Some("op:product"), '√' => Some("op:sqrt"),
            '∞' => Some("op:infinity"), '∂' => Some("op:partial"), '∇' => Some("op:nabla"),
            '≈' => Some("op:approx"), '≠' => Some("op:neq"), '≤' => Some("op:leq"),
            '≥' => Some("op:geq"), '×' => Some("op:times"), '÷' => Some("op:divide"),
            '→' | '⟶' => Some("op:rightarrow"), '↔' => Some("op:leftrightarrow"),
            '∈' => Some("op:in"), '∉' => Some("op:notin"), '∩' => Some("op:intersection"),
            '∪' => Some("op:union"), '∅' => Some("op:emptyset"), '^' => Some("op:superscript"),
            '_' => Some("op:subscript"), '=' => Some("op:equals"), '+' => Some("op:plus"),
            '-' | '−' => Some("op:minus"), '/' => Some("op:divide"), _ => None,
        };
        if let Some(value) = token { out.push(value.to_string()); }
    }
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_greek_operators_and_latex_commands() {
        let tokens = extract_math_tokens(r"\int_0^1 \frac{\alpha}{\sqrt{x}} \leq \infty");
        for expected in ["math:int", "math:frac", "math:alpha", "math:sqrt", "math:leq", "math:infty", "op:integral", "op:subscript", "op:superscript"] {
            assert!(tokens.iter().any(|token| token == expected), "missing {expected}");
        }
    }
}
