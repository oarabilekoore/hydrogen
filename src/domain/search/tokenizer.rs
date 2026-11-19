const STOPWORDS: &[&str] = &[
"the","and","for","with","this","that","from","are","was","were","have","has","had","not","but","you","your","can","will","into","also","than","then","them","they","their","there","here","what","when","where","which","while","about","after","before","between","each","other","some","such","only","same","very","just","more","most","use","used","using","its","out","all","any","both","few","how","why","who","whom","been","being","does","did","doing","would","could","should","may","might","must","shall","over","under","again","further","once","during","through","above","below","import","def","return","const","let","var","function","class","true","false","null","none","void","int","str","self","public","private","static","export","default","include","endif","ifdef",
];
fn math_alias(ch: char) -> Option<&'static str> {
    Some(match ch {
        '∫'=>"integral",'∑'=>"summation",'∏'=>"product",'√'=>"sqrt",'π'=>"pi",
        'θ'|'Θ'=>"theta",'λ'|'Λ'=>"lambda",'μ'|'Μ'=>"mu",'σ'|'Σ'=>"sigma",
        'Δ'|'δ'=>"delta",'∞'=>"infinity",'∂'=>"partial",'∇'=>"nabla",
        '≤'=>"leq",'≥'=>"geq",'≠'=>"neq",'≈'=>"approx",'×'=>"times",'÷'=>"divide",
        'α'|'Α'=>"alpha",'β'|'Β'=>"beta",'γ'|'Γ'=>"gamma",'ω'|'Ω'=>"omega",
        'φ'|'Φ'=>"phi",'ρ'|'Ρ'=>"rho",'ε'|'Ε'=>"epsilon",'η'|'Η'=>"eta",'τ'|'Τ'=>"tau",
        _=>return None,
    })
}
pub fn extract_raw_tokens(text: &str) -> Vec<String> {
    let mut out=Vec::new(); let mut buf=String::new();
    let flush=|buf:&mut String,out:&mut Vec<String>| { if buf.chars().count()>=2 { out.push(std::mem::take(buf)); } else { buf.clear(); } };
    for ch in text.chars() {
        if ch.is_alphanumeric() { for c in ch.to_lowercase() { buf.push(c); } }
        else { flush(&mut buf,&mut out); if let Some(alias)=math_alias(ch) { out.push(alias.to_string()); } }
    }
    flush(&mut buf,&mut out); out
}
pub fn tokenize_content(text: &str) -> Vec<String> {
    let raw=extract_raw_tokens(text);
    let filtered:Vec<String>=raw.iter().filter(|t|!STOPWORDS.contains(&t.as_str())).cloned().collect();
    if filtered.is_empty(){raw}else{filtered}
}
#[cfg(test)]
mod tests {
use super::*;
#[test] fn filters_stopwords_and_short_tokens(){assert_eq!(tokenize_content("The quick brown fox and a 123 diagram"),vec!["quick","brown","fox","123","diagram"]);}
#[test] fn falls_back_when_all_stopwords(){assert_eq!(tokenize_content("the and for"),vec!["the","and","for"]);}
#[test] fn preserves_greek_and_math_symbols(){let t=extract_raw_tokens("∫ f(θ) dθ = π + √x ≥ 0");for s in ["integral","theta","pi","sqrt","geq"]{assert!(t.iter().any(|v|v==s));}}
}
