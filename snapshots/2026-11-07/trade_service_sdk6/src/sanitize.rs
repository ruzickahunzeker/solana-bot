pub fn redact_urls(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut idx = 0;
    while idx < input.len() {
        let rest = &input[idx..];
        if rest.starts_with("https://") || rest.starts_with("http://") {
            output.push_str("<redacted-url>");
            idx += if rest.starts_with("https://") { 8 } else { 7 };
            while idx < input.len() {
                let ch = input[idx..].chars().next().unwrap();
                if ch.is_whitespace() || matches!(ch, ')' | '"' | '\'' | ',') {
                    break;
                }
                idx += ch.len_utf8();
            }
            continue;
        }
        let ch = rest.chars().next().unwrap();
        output.push(ch);
        idx += ch.len_utf8();
    }
    output
}
