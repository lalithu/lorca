use crate::agent_types::*;
use futures_util::StreamExt;
use reqwest::{Client, Url, redirect::Policy};
use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};
use tokio::sync::watch;
pub(super) fn public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            !v.is_private()
                && !v.is_loopback()
                && !v.is_link_local()
                && !v.is_multicast()
                && !v.is_unspecified()
                && !v.is_broadcast()
                && !v.is_documentation()
                && v.octets()[0] != 0
                && v.octets()[0] < 224
                && !(v.octets()[0] == 100 && (64..=127).contains(&v.octets()[1]))
                && !(v.octets()[0] == 198 && (18..=19).contains(&v.octets()[1]))
        }
        IpAddr::V6(v) => v
            .to_ipv4_mapped()
            .map(|v| public(IpAddr::V4(v)))
            .unwrap_or_else(|| {
                (v.segments()[0] & 0xe000) == 0x2000
                    && !(v.segments()[0] == 0x2001 && v.segments()[1] == 0x0db8)
            }),
    }
}
pub(super) fn url(raw: &str) -> Result<Url> {
    let u = Url::parse(raw).map_err(|_| "Invalid URL")?;
    let host = u.host_str().ok_or("URL requires a host")?;
    if u.scheme() != "https"
        || u.port_or_known_default() != Some(443)
        || !u.username().is_empty()
        || u.password().is_some()
        || host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.ends_with(".internal")
    {
        return Err("Only public HTTPS URLs without credentials on port 443 are allowed".into());
    }
    if let Ok(ip) = host.trim_matches(['[', ']']).parse::<IpAddr>()
        && !public(ip)
    {
        return Err("Private/local network URLs are unavailable".into());
    }
    Ok(u)
}
async fn client(u: &Url) -> Result<Client> {
    let host = u.host_str().ok_or("Missing host")?;
    let addresses: Vec<SocketAddr> = tokio::net::lookup_host((host, 443))
        .await
        .map_err(|e| format!("DNS: {e}"))?
        .collect();
    if addresses.is_empty() || addresses.iter().any(|a| !public(a.ip())) {
        return Err("URL resolves to a private or unsupported address".into());
    }
    // Pin checked DNS answers to this request, including each separately validated redirect.
    Client::builder()
        .redirect(Policy::none())
        .no_proxy()
        .timeout(Duration::from_secs(20))
        .resolve_to_addrs(host, &addresses)
        .build()
        .map_err(|e| e.to_string())
}
pub(super) async fn request(
    raw: &str,
    body: Option<serde_json::Value>,
    token: Option<&str>,
    mut cancel: watch::Receiver<bool>,
) -> Result<(String, String, String, bool)> {
    let mut current = url(raw)?;
    for redirects in 0..=3 {
        if *cancel.borrow() {
            return Err("Cancelled".into());
        }
        let operation = async {
            let client = client(&current).await?;
            let mut request = if let Some(body) = &body {
                client.post(current.clone()).json(body)
            } else {
                client.get(current.clone())
            };
            if let Some(token) = token {
                request = request.bearer_auth(token);
            }
            let response = request
                .header("User-Agent", "lorca/0.4")
                .send()
                .await
                .map_err(|e| format!("Web request: {e}"))?;
            if response.status().is_redirection() {
                if body.is_some() || redirects == 3 {
                    return Err(
                        "Search redirects are disabled; page redirect limit is three".into(),
                    );
                }
                let location = response
                    .headers()
                    .get("location")
                    .and_then(|h| h.to_str().ok())
                    .ok_or("Redirect missing location")?;
                return Ok((
                    Some(url(current
                        .join(location)
                        .map_err(|e| e.to_string())?
                        .as_str())?),
                    String::new(),
                    String::new(),
                    false,
                ));
            }
            if !response.status().is_success() {
                return Err(format!("Web HTTP {}", response.status()));
            }
            let content_type = response
                .headers()
                .get("content-type")
                .and_then(|h| h.to_str().ok())
                .unwrap_or("")
                .to_string();
            if !content_type.starts_with("text/")
                && !content_type.contains("json")
                && !content_type.contains("xml")
            {
                return Err("Unsupported page type; expected text, HTML, XML or JSON".into());
            }
            let mut bytes = Vec::new();
            let mut truncated = false;
            let mut stream = response.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|e| e.to_string())?;
                let n = chunk
                    .len()
                    .min((128 * 1024_usize).saturating_sub(bytes.len()));
                bytes.extend_from_slice(&chunk[..n]);
                if n < chunk.len() {
                    truncated = true;
                    break;
                }
            }
            Ok((
                None,
                content_type,
                String::from_utf8_lossy(&bytes).into_owned(),
                truncated,
            ))
        };
        let (redirect, ty, text, truncated) = tokio::select! {biased;_=cancel.changed()=>return Err("Cancelled".into()),r=tokio::time::timeout(Duration::from_secs(25),operation)=>r.map_err(|_|"Web request timed out")??};
        if let Some(next) = redirect {
            current = next;
        } else {
            return Ok((current.to_string(), ty, text, truncated));
        }
    }
    Err("Page redirect limit reached".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checked_addresses_exclude_private_and_reserved_ranges() {
        for address in [
            "127.0.0.1",
            "10.0.0.1",
            "169.254.169.254",
            "192.168.1.1",
            "0.0.0.0",
            "100.64.0.1",
            "198.18.0.1",
            "224.0.0.1",
            "::1",
            "fe80::1",
            "fc00::1",
            "::ffff:127.0.0.1",
            "2001:db8::1",
        ] {
            assert!(!public(address.parse().unwrap()), "{address}");
        }
        for address in ["1.1.1.1", "8.8.8.8", "2606:4700:4700::1111"] {
            assert!(public(address.parse().unwrap()), "{address}");
        }
    }
    #[tokio::test]
    async fn redirects_cannot_bypass_local_address_validation() {
        let (_tx, c) = watch::channel(false);
        assert!(request("https://127.0.0.1", None, None, c).await.is_err());
    }
}

#[cfg(test)]
mod network_tests {
    use super::*;
    #[tokio::test]
    #[ignore = "requires public HTTPS network access"]
    async fn fetches_a_real_public_page() {
        let (_tx, c) = watch::channel(false);
        let (url, ty, text, _) = request("https://example.com/", None, None, c)
            .await
            .unwrap();
        assert!(url.starts_with("https://example.com/"));
        assert!(ty.contains("text/html"));
        assert!(text.contains("Example Domain"));
    }
}
