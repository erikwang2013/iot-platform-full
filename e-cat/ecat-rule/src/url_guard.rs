// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 出站 URL SSRF 守卫：通知渠道与规则 webhook 在发请求前校验目标。
//! 写库时只校验 http(s) 前缀（store::validate_*），DNS 解析放发送前——
//! 写入路径不阻塞解析，也能覆盖存量行。
use std::net::IpAddr;

/// 非公网地址判定：loopback、私有、链路本地、未指定、组播、广播、CGNAT，
/// 以及内嵌这些地址的 IPv4-mapped IPv6。
pub fn ip_is_forbidden(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local() // 169.254.0.0/16
                || v4.is_unspecified() // 0.0.0.0
                || v4.is_multicast()
                || v4.is_broadcast() // 255.255.255.255
                || v4.octets()[0] == 0 // 0.0.0.0/8
                || is_cgnat(v4) // 100.64.0.0/10
        }
        IpAddr::V6(v6) => {
            // IPv4-mapped（::ffff:a.b.c.d）：按内嵌 IPv4 判定
            if let Some(v4) = v6.to_ipv4_mapped() {
                return ip_is_forbidden(IpAddr::V4(v4));
            }
            let seg = v6.segments();
            v6.is_loopback() // ::1
                || v6.is_unspecified() // ::
                || v6.is_multicast() // ff00::/8
                || (seg[0] & 0xfe00) == 0xfc00 // fc00::/7 ULA
                || (seg[0] & 0xffc0) == 0xfe80 // fe80::/10 link-local
        }
    }
}

fn is_cgnat(v4: std::net::Ipv4Addr) -> bool {
    let o = v4.octets();
    o[0] == 100 && (o[1] & 0xc0) == 64
}

/// 日志安全的目标描述：只保留 scheme + host（+ 非默认端口），
/// 绝不带 userinfo/path/query（webhook token 常在 query/path 中）。
pub fn redact_url(url: &str) -> String {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return "<invalid-url>".into();
    };
    let Some(host) = parsed.host_str() else {
        return "<invalid-url>".into();
    };
    let mut out = format!("{}://{host}", parsed.scheme());
    let default_port = match parsed.scheme() {
        "http" => Some(80),
        "https" => Some(443),
        _ => None,
    };
    if let Some(port) = parsed.port()
        && Some(port) != default_port
    {
        out.push_str(&format!(":{port}"));
    }
    out
}

/// 发请求前校验：仅 http/https；拒绝 localhost；解析出的任一地址非公网即拒绝。
/// shortcut: DNS 解析与 reqwest 连接分两次进行，DNS rebinding 仍可能绕过；
/// 需要强保证时改为固定解析结果直连（connect-to）。
pub async fn ensure_safe_http_url(url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("invalid url: {e}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(format!("unsupported scheme: {}", parsed.scheme()));
    }
    let host = parsed.host_str().ok_or("url has no host")?;
    // host_str 对 IPv6 字面量带方括号
    let host = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    if let Ok(ip) = host.parse::<IpAddr>() {
        return if ip_is_forbidden(ip) {
            Err(format!("refusing non-public address: {ip}"))
        } else {
            Ok(())
        };
    }
    if host.eq_ignore_ascii_case("localhost") {
        return Err("refusing localhost target".into());
    }
    let port = parsed.port_or_known_default().ok_or("url has no port")?;
    let mut addrs = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| format!("dns resolve {host} failed: {e}"))?;
    let mut resolved = false;
    for addr in addrs.by_ref() {
        resolved = true;
        if ip_is_forbidden(addr.ip()) {
            return Err(format!("refusing target {host} -> {}", addr.ip()));
        }
    }
    if !resolved {
        return Err(format!("dns resolve {host}: no addresses"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn forbids_non_public_ipv4() {
        for s in [
            "127.0.0.1",
            "127.1.2.3",
            "10.0.0.1",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "100.127.255.255",
            "0.0.0.0",
            "255.255.255.255",
            "224.0.0.1",
        ] {
            assert!(ip_is_forbidden(ip(s)), "must forbid {s}");
        }
    }

    #[test]
    fn forbids_non_public_ipv6_and_mapped_v4() {
        for s in [
            "::1",
            "::",
            "fe80::1",
            "fc00::1",
            "fd12:3456::1",
            "ff02::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
        ] {
            assert!(ip_is_forbidden(ip(s)), "must forbid {s}");
        }
    }

    #[test]
    fn allows_public_addresses() {
        for s in ["8.8.8.8", "1.1.1.1", "2606:4700::1111"] {
            assert!(!ip_is_forbidden(ip(s)), "must allow {s}");
        }
    }

    #[tokio::test]
    async fn rejects_private_literal_urls_without_dns() {
        for url in [
            "http://127.0.0.1/hook",
            "https://10.0.0.1:8443/x",
            "http://[::1]:8080/hook",
            "http://169.254.169.254/latest/meta-data",
            "http://localhost:8080/x",
        ] {
            assert!(
                ensure_safe_http_url(url).await.is_err(),
                "must reject {url}"
            );
        }
    }

    #[tokio::test]
    async fn rejects_non_http_schemes() {
        assert!(ensure_safe_http_url("file:///etc/passwd").await.is_err());
        assert!(ensure_safe_http_url("ftp://example.com/x").await.is_err());
    }

    #[tokio::test]
    async fn accepts_public_literal_ip() {
        // 字面 IP 不触发 DNS，纯本地判定
        assert!(ensure_safe_http_url("https://8.8.8.8/x").await.is_ok());
    }

    #[test]
    fn redact_url_strips_token_path_query_and_userinfo() {
        let redacted = redact_url(
            "https://user:p4ss@hooks.example.com:8443/services/T000/B000/secret-token?access_token=abc123&sign=xyz",
        );
        assert_eq!(redacted, "https://hooks.example.com:8443");
        for secret in [
            "user",
            "p4ss",
            "services",
            "secret-token",
            "access_token",
            "abc123",
            "sign",
        ] {
            assert!(!redacted.contains(secret), "leaked {secret}: {redacted}");
        }
    }

    #[test]
    fn redact_url_keeps_scheme_host_and_omits_default_port() {
        assert_eq!(
            redact_url("https://example.com:443/hook?token=x"),
            "https://example.com"
        );
        assert_eq!(
            redact_url("http://example.com:80/hook"),
            "http://example.com"
        );
        assert_eq!(redact_url("http://[::1]:8080/x"), "http://[::1]:8080");
    }

    #[test]
    fn redact_url_invalid_input_is_placeholder() {
        for bad in ["", "not a url", "mailto:x@example.com", "https://"] {
            assert_eq!(redact_url(bad), "<invalid-url>", "input: {bad:?}");
        }
    }
}
