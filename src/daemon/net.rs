//! 局域网候选 IP 枚举（design.md 决策 13）：默认路由网卡优先，其余非回环 IPv4 随后。

use std::net::IpAddr;

/// 候选局域网 IPv4 地址：默认路由网卡地址排首位，去重，排除回环地址。
pub fn candidate_ips() -> Vec<String> {
    let default_route = local_ip_address::local_ip()
        .ok()
        .filter(|ip| matches!(ip, IpAddr::V4(v4) if !v4.is_loopback()));
    let mut rest: Vec<IpAddr> = local_ip_address::list_afinet_netifas()
        .map(|ifs| {
            ifs.into_iter()
                .map(|(_, ip)| ip)
                .filter(|ip| matches!(ip, IpAddr::V4(v4) if !v4.is_loopback()))
                .filter(|ip| Some(*ip) != default_route)
                .collect()
        })
        .unwrap_or_default();
    rest.sort();
    rest.dedup();
    default_route
        .into_iter()
        .chain(rest)
        .map(|ip| ip.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_route_ip_comes_first() {
        let ips = candidate_ips();
        if let Ok(default) = local_ip_address::local_ip() {
            if default.is_ipv4() && !default.is_loopback() {
                assert_eq!(
                    ips.first().map(String::as_str),
                    Some(default.to_string().as_str())
                );
            }
        }
        // 无回环、无重复
        assert!(ips.iter().all(|ip| !ip.starts_with("127.")));
        let mut sorted = ips.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(ips.len(), sorted.len());
    }
}
