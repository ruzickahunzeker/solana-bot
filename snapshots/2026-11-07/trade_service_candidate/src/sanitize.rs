pub fn redact_urls(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut idx = 0;
    while idx < input.len() {
        let rest = &input[idx..];
        let scheme_len = if rest.starts_with("https://") || rest.starts_with("wss://") {
            Some(8)
        } else if rest.starts_with("http://") || rest.starts_with("ws://") {
            Some(7)
        } else {
            None
        };
        if let Some(scheme_len) = scheme_len {
            output.push_str("<redacted-url>");
            idx += scheme_len;
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

#[cfg(test)]
mod tests {
    use super::redact_urls;

    #[test]
    fn redacts_rpc_urls_and_preserves_error_context() {
        let value = redact_urls(
            "request failed for https://mainnet.example/?api-key=secret, retrying lane 2",
        );
        assert_eq!(value, "request failed for <redacted-url>, retrying lane 2");
        assert!(!value.contains("secret"));
    }

    #[test]
    fn redacts_http_urls_terminated_by_error_punctuation() {
        let value = redact_urls("endpoint http://127.0.0.1:8899/path). next");
        assert_eq!(value, "endpoint <redacted-url>). next");
    }

    #[test]
    fn redacts_websocket_endpoints() {
        let value = redact_urls("quic fallback wss://mev.example/submit?token=secret");
        assert_eq!(value, "quic fallback <redacted-url>");
        assert!(!value.contains("secret"));
    }
}
