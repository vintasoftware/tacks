//! Request guards for the local web server: DNS-rebinding and cross-origin (CSRF) protection.

use axum::{
    extract::Request,
    http::{HeaderMap, Method, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};

/// Split a `Host`/`Origin` authority into (hostname, optional port).
/// IPv6 literals keep their brackets, e.g. `[::1]:8080` gives `("[::1]", Some("8080"))`.
fn split_authority(authority: &str) -> Option<(&str, Option<&str>)> {
    if authority.starts_with('[') {
        let end = authority.find(']')?;
        let host = &authority[..=end];
        return match &authority[end + 1..] {
            "" => Some((host, None)),
            rest => Some((host, Some(rest.strip_prefix(':')?))),
        };
    }
    match authority.split_once(':') {
        None => Some((authority, None)),
        Some((h, p)) => Some((h, Some(p))),
    }
}

/// True when `host` (a `Host` header value) names the loopback interface with a numeric
/// port (or none). The port is not compared with the listening port because the router
/// does not know it; any other hostname (DNS rebinding) is rejected.
pub fn is_loopback_host(host: &str) -> bool {
    let Some((name, port)) = split_authority(host) else {
        return false;
    };
    let name_ok = matches!(
        name.to_ascii_lowercase().as_str(),
        "127.0.0.1" | "localhost" | "[::1]"
    );
    let port_ok = port.is_none_or(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    name_ok && port_ok
}

/// Extract the authority (`host[:port]`) of an `Origin` header value.
fn origin_authority(origin: &str) -> Option<&str> {
    let rest = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))?;
    Some(rest.trim_end_matches('/'))
}

/// Decide whether a request is allowed; `Err` carries the rejection status and message.
pub fn check_request(
    method: &Method,
    headers: &HeaderMap,
) -> Result<(), (StatusCode, &'static str)> {
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !is_loopback_host(host) {
        return Err((StatusCode::FORBIDDEN, "forbidden: unexpected host header"));
    }
    let state_changing = matches!(
        *method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    );
    if !state_changing {
        return Ok(());
    }
    if let Some(site) = headers.get("sec-fetch-site")
        && site.as_bytes().eq_ignore_ascii_case(b"cross-site")
    {
        return Err((StatusCode::FORBIDDEN, "forbidden: cross-site request"));
    }
    if let Some(origin) = headers.get(header::ORIGIN) {
        let same = origin
            .to_str()
            .ok()
            .and_then(origin_authority)
            .is_some_and(|a| a.eq_ignore_ascii_case(host));
        if !same {
            return Err((StatusCode::FORBIDDEN, "forbidden: cross-origin request"));
        }
    }
    Ok(())
}

/// Axum middleware applying [`check_request`] to every request.
pub async fn guard(req: Request, next: Next) -> Response {
    match check_request(req.method(), req.headers()) {
        Ok(()) => next.run(req).await,
        Err(rejection) => rejection.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, v.parse().unwrap());
        }
        h
    }

    #[test]
    fn test_loopback_hosts() {
        for ok in ["127.0.0.1:8080", "localhost:3000", "[::1]:80", "localhost"] {
            assert!(is_loopback_host(ok), "{ok}");
        }
        for bad in [
            "evil.example",
            "evil.example:80",
            "127.0.0.1.evil.com:80",
            "",
            "localhost:abc",
            "[::1]x",
        ] {
            assert!(!is_loopback_host(bad), "{bad}");
        }
    }

    #[test]
    fn test_origin_rules() {
        let post = Method::POST;
        assert!(check_request(&post, &headers(&[("host", "127.0.0.1:1")])).is_ok());
        assert!(
            check_request(
                &post,
                &headers(&[("host", "127.0.0.1:1"), ("origin", "http://127.0.0.1:1")])
            )
            .is_ok()
        );
        assert!(
            check_request(
                &post,
                &headers(&[("host", "127.0.0.1:1"), ("origin", "http://evil.example")])
            )
            .is_err()
        );
        assert!(
            check_request(
                &post,
                &headers(&[("host", "127.0.0.1:1"), ("origin", "http://127.0.0.1:2")])
            )
            .is_err()
        );
        assert!(
            check_request(
                &post,
                &headers(&[("host", "127.0.0.1:1"), ("sec-fetch-site", "cross-site")])
            )
            .is_err()
        );
        assert!(
            check_request(
                &post,
                &headers(&[("host", "127.0.0.1:1"), ("sec-fetch-site", "same-origin")])
            )
            .is_ok()
        );
        // GET ignores Origin but still checks Host
        assert!(
            check_request(
                &Method::GET,
                &headers(&[("host", "127.0.0.1:1"), ("origin", "http://evil.example")])
            )
            .is_ok()
        );
        assert!(check_request(&Method::GET, &headers(&[("host", "evil.example")])).is_err());
    }
}
