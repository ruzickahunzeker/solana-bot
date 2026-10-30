use axum::http::HeaderMap;

pub fn authorized(headers: &HeaderMap, token: &str) -> bool {
    let expected = token.trim();
    if expected.is_empty() {
        return false;
    }
    let Some(value) = headers.get(axum::http::header::AUTHORIZATION) else {
        return false;
    };
    let Ok(value) = value.to_str() else {
        return false;
    };
    let mut parts = value.split_whitespace();
    let Some(scheme) = parts.next() else {
        return false;
    };
    let Some(actual) = parts.next() else {
        return false;
    };
    if parts.next().is_some() {
        return false;
    }
    scheme.eq_ignore_ascii_case("Bearer") && actual == expected
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn accepts_standard_bearer_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer abc123"),
        );
        assert!(authorized(&headers, "abc123"));
    }

    #[test]
    fn accepts_extra_spaces_and_case_insensitive_scheme() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("bearer   abc123"),
        );
        assert!(authorized(&headers, " abc123 "));
    }

    #[test]
    fn rejects_wrong_token() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer wrong"),
        );
        assert!(!authorized(&headers, "abc123"));
    }
}
