//! WireGuard `.conf` (wg-quick biçimi) ayrıştırıcısı ve NM `wireguard` bağlantı ayarları.
//!
//! Dış araç yok (`wg`, `nmcli` çağrılmaz). Desteklenen anahtarlar (büyük/küçük harf duyarsız):
//! `[Interface]` `PrivateKey`, `Address`, `DNS`, `ListenPort`, `FwMark`, `MTU`; `[Peer]`
//! `PublicKey`, `PresharedKey`, `AllowedIPs`, `Endpoint`, `PersistentKeepalive`. wg-quick'e
//! özgü `Table`, `PreUp`/`PostUp`/`PreDown`/`PostDown`, `SaveConfig` **çalıştırılmaz ve
//! içe aktarılmaz** ([`WireGuardConfig::ignored`]; Ayarlar uyarı gösterebilir). Bilinmeyen
//! anahtar/bölüm hatadır.
//!
//! **Gizli bilgi.** Özel anahtar ve ön paylaşımlı anahtar [`Secret`] içinde (sıfırlanır);
//! hata türleri değer içermez (yalnız satır numarası ve tür). NM'ye giden ayarlarda değerler
//! anahtardan ödünç alınır.

use std::collections::HashMap;
use std::net::IpAddr;

use zbus::zvariant::Value;

use crate::Secret;

/// Kabul edilen en büyük dosya.
pub const MAX_CONF_BYTES: usize = 64 * 1024;
/// En fazla eş.
pub const MAX_PEERS: usize = 64;
/// Liste (adres, DNS, izinli IP) başına en fazla öğe.
const MAX_LIST: usize = 256;

/// `adres/önek`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpNet {
    pub addr: IpAddr,
    pub prefix: u8,
}

impl IpNet {
    /// `a.b.c.d/n`, `x::y/n` ya da öneksiz adres (tam önek).
    pub fn parse(s: &str) -> Option<Self> {
        let (addr, prefix) = match s.split_once('/') {
            Some((a, p)) => (a.trim(), Some(p.trim())),
            None => (s.trim(), None),
        };
        let addr: IpAddr = addr.parse().ok()?;
        let max = if addr.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            Some(p) if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) => p.parse::<u8>().ok()?,
            Some(_) => return None,
            None => max,
        };
        (prefix <= max).then_some(Self { addr, prefix })
    }
}

impl std::fmt::Display for IpNet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.addr, self.prefix)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireGuardPeer {
    pub public_key: String,
    pub preshared_key: Option<Secret>,
    pub allowed_ips: Vec<IpNet>,
    /// `host:port` ya da `[v6]:port`.
    pub endpoint: Option<String>,
    pub persistent_keepalive: Option<u16>,
}

/// Ayrıştırılmış yapılandırma. `Debug` anahtarları göstermez ([`Secret`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireGuardConfig {
    pub private_key: Secret,
    pub listen_port: Option<u16>,
    pub fwmark: Option<u32>,
    pub mtu: Option<u32>,
    pub addresses: Vec<IpNet>,
    pub dns: Vec<IpAddr>,
    /// `DNS` satırındaki IP olmayan öğeler (arama alanları; wg-quick kuralı).
    pub dns_search: Vec<String>,
    pub peers: Vec<WireGuardPeer>,
    /// İçe aktarılmayan wg-quick anahtarları (kanonik adlarıyla, tekrarsız).
    pub ignored: Vec<&'static str>,
}

/// Ayrıştırma hatası: satır (1'den; dosya geneli için 0) ve tür. Değer içermez.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WireGuardError {
    pub line: usize,
    pub kind: WireGuardErrorKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum WireGuardErrorKind {
    /// Dosya [`MAX_CONF_BYTES`]'tan büyük.
    TooLarge,
    /// `anahtar = değer` ya da `[Bölüm]` değil.
    Syntax,
    UnknownSection,
    /// Anahtar bir bölümden önce.
    KeyOutsideSection,
    UnknownKey,
    /// Tekrarlanamayan anahtar ikinci kez.
    DuplicateKey,
    MissingInterface,
    DuplicateInterface,
    MissingPrivateKey,
    /// `[Peer]`'de `PublicKey` yok.
    MissingPublicKey,
    /// Anahtar 32 baytlık base64 değil.
    InvalidKey,
    InvalidAddress,
    InvalidNumber,
    InvalidEndpoint,
    /// Liste ya da eş sayısı sınırı aşıldı.
    TooMany,
    /// Bağlantı/arayüz adı geçersiz (1–15 bayt, `[A-Za-z0-9_.=+-]`).
    InvalidName,
}

impl std::fmt::Display for WireGuardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "WireGuard yapılandırması, satır {}: {:?}", self.line, self.kind)
    }
}

impl std::error::Error for WireGuardError {}

fn err(line: usize, kind: WireGuardErrorKind) -> WireGuardError {
    WireGuardError { line, kind }
}

/// WireGuard anahtarı: 32 baytın standart base64'ü (44 karakter, `=` ile biter).
pub fn valid_key(s: &str) -> bool {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let b = s.as_bytes();
    if b.len() != 44 || b[43] != b'=' {
        return false;
    }
    let idx = |c: u8| ALPHABET.iter().position(|&a| a == c);
    // 32 bayt = 256 bit; 43. karakter yalnız 4 anlamlı bit taşır (alt 2 bit sıfır).
    b[..42].iter().all(|&c| idx(c).is_some()) && idx(b[42]).is_some_and(|v| v & 0b11 == 0)
}

/// Bağlantı ve arayüz adı (Linux arayüz adı: en fazla 15 bayt).
pub fn valid_name(name: &str) -> bool {
    (1..=15).contains(&name.len())
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'=' | b'+' | b'-'))
        && name != "."
        && name != ".."
}

fn valid_endpoint(s: &str) -> bool {
    let Some((host, port)) = s.rsplit_once(':') else { return false };
    if !port.bytes().all(|b| b.is_ascii_digit()) || port.parse::<u16>().map_or(true, |p| p == 0) {
        return false;
    }
    if let Some(inner) = host.strip_prefix('[') {
        return inner.strip_suffix(']').is_some_and(|v6| v6.parse::<std::net::Ipv6Addr>().is_ok());
    }
    !host.is_empty()
        && host.len() <= 253
        && host.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

fn number<T: std::str::FromStr>(line: usize, v: &str) -> Result<T, WireGuardError> {
    if v.is_empty() || !v.bytes().all(|b| b.is_ascii_digit()) {
        return Err(err(line, WireGuardErrorKind::InvalidNumber));
    }
    v.parse().map_err(|_| err(line, WireGuardErrorKind::InvalidNumber))
}

fn list(v: &str) -> impl Iterator<Item = &str> {
    v.split(',').map(str::trim).filter(|s| !s.is_empty())
}

fn push_limited<T>(line: usize, to: &mut Vec<T>, item: T) -> Result<(), WireGuardError> {
    if to.len() >= MAX_LIST {
        return Err(err(line, WireGuardErrorKind::TooMany));
    }
    to.push(item);
    Ok(())
}

fn set_once<T>(line: usize, slot: &mut Option<T>, value: T) -> Result<(), WireGuardError> {
    if slot.is_some() {
        return Err(err(line, WireGuardErrorKind::DuplicateKey));
    }
    *slot = Some(value);
    Ok(())
}

fn key_secret(line: usize, v: &str) -> Result<Secret, WireGuardError> {
    if !valid_key(v) {
        return Err(err(line, WireGuardErrorKind::InvalidKey));
    }
    Ok(Secret::new(v))
}

#[derive(Default)]
struct PeerDraft {
    line: usize,
    public_key: Option<String>,
    preshared_key: Option<Secret>,
    allowed_ips: Vec<IpNet>,
    endpoint: Option<String>,
    persistent_keepalive: Option<Option<u16>>,
}

enum Section {
    None,
    Interface,
    Peer,
}

/// wg-quick `.conf` metnini ayrıştırır. Metin genelde özel anahtar içerir: çağıran onu
/// [`Secret`] içinde tutar ve `expose` etmeden buraya ödünç verir ([`crate::NetworkService::import_wireguard`]).
pub fn parse(text: &str) -> Result<WireGuardConfig, WireGuardError> {
    use WireGuardErrorKind as K;
    if text.len() > MAX_CONF_BYTES {
        return Err(err(0, K::TooLarge));
    }
    let mut section = Section::None;
    let mut seen_interface = false;
    let mut private_key: Option<Secret> = None;
    let mut listen_port = None;
    let mut fwmark = None;
    let mut mtu = None;
    let mut addresses = Vec::new();
    let mut dns = Vec::new();
    let mut dns_search = Vec::new();
    let mut ignored: Vec<&'static str> = Vec::new();
    let mut peers: Vec<PeerDraft> = Vec::new();

    for (i, raw) in text.lines().enumerate() {
        let line = i + 1;
        let content = raw.split_once('#').map_or(raw, |(c, _)| c).trim();
        if content.is_empty() {
            continue;
        }
        if let Some(name) = content.strip_prefix('[') {
            let name = name.strip_suffix(']').ok_or(err(line, K::Syntax))?.trim();
            if name.eq_ignore_ascii_case("interface") {
                if seen_interface {
                    return Err(err(line, K::DuplicateInterface));
                }
                seen_interface = true;
                section = Section::Interface;
            } else if name.eq_ignore_ascii_case("peer") {
                if peers.len() >= MAX_PEERS {
                    return Err(err(line, K::TooMany));
                }
                peers.push(PeerDraft { line, ..PeerDraft::default() });
                section = Section::Peer;
            } else {
                return Err(err(line, K::UnknownSection));
            }
            continue;
        }
        let (key, value) = content.split_once('=').ok_or(err(line, K::Syntax))?;
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();
        if key.is_empty() {
            return Err(err(line, K::Syntax));
        }
        match section {
            Section::None => return Err(err(line, K::KeyOutsideSection)),
            Section::Interface => match key.as_str() {
                "privatekey" => {
                    let k = key_secret(line, value)?;
                    set_once(line, &mut private_key, k)?;
                }
                "listenport" => set_once(line, &mut listen_port, number::<u16>(line, value)?)?,
                "fwmark" => {
                    let mark = if value.eq_ignore_ascii_case("off") {
                        0
                    } else if let Some(hex) = value.strip_prefix("0x").or_else(|| value.strip_prefix("0X")) {
                        u32::from_str_radix(hex, 16).map_err(|_| err(line, K::InvalidNumber))?
                    } else {
                        number::<u32>(line, value)?
                    };
                    set_once(line, &mut fwmark, mark)?;
                }
                "mtu" => set_once(line, &mut mtu, number::<u32>(line, value)?)?,
                "address" => {
                    for a in list(value) {
                        let net = IpNet::parse(a).ok_or(err(line, K::InvalidAddress))?;
                        push_limited(line, &mut addresses, net)?;
                    }
                }
                "dns" => {
                    for d in list(value) {
                        match d.parse::<IpAddr>() {
                            Ok(ip) => push_limited(line, &mut dns, ip)?,
                            Err(_) => {
                                let ok = d.len() <= 253
                                    && d.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'));
                                if !ok {
                                    return Err(err(line, K::InvalidAddress));
                                }
                                push_limited(line, &mut dns_search, d.to_owned())?;
                            }
                        }
                    }
                }
                "table" | "preup" | "postup" | "predown" | "postdown" | "saveconfig" => {
                    let canonical = match key.as_str() {
                        "table" => "Table",
                        "preup" => "PreUp",
                        "postup" => "PostUp",
                        "predown" => "PreDown",
                        "postdown" => "PostDown",
                        _ => "SaveConfig",
                    };
                    if !ignored.contains(&canonical) {
                        ignored.push(canonical);
                    }
                }
                _ => return Err(err(line, K::UnknownKey)),
            },
            Section::Peer => {
                let peer = peers.last_mut().expect("Peer bölümünde eş var");
                match key.as_str() {
                    "publickey" => {
                        if !valid_key(value) {
                            return Err(err(line, K::InvalidKey));
                        }
                        set_once(line, &mut peer.public_key, value.to_owned())?;
                    }
                    "presharedkey" => {
                        let k = key_secret(line, value)?;
                        set_once(line, &mut peer.preshared_key, k)?;
                    }
                    "allowedips" => {
                        for a in list(value) {
                            let net = IpNet::parse(a).ok_or(err(line, K::InvalidAddress))?;
                            push_limited(line, &mut peer.allowed_ips, net)?;
                        }
                    }
                    "endpoint" => {
                        if !valid_endpoint(value) {
                            return Err(err(line, K::InvalidEndpoint));
                        }
                        set_once(line, &mut peer.endpoint, value.to_owned())?;
                    }
                    "persistentkeepalive" => {
                        let k = if value.eq_ignore_ascii_case("off") { None } else { Some(number::<u16>(line, value)?) };
                        set_once(line, &mut peer.persistent_keepalive, k)?;
                    }
                    _ => return Err(err(line, K::UnknownKey)),
                }
            }
        }
    }
    if !seen_interface {
        return Err(err(0, K::MissingInterface));
    }
    let private_key = private_key.ok_or(err(0, K::MissingPrivateKey))?;
    let peers = peers
        .into_iter()
        .map(|p| {
            Ok(WireGuardPeer {
                public_key: p.public_key.ok_or(err(p.line, K::MissingPublicKey))?,
                preshared_key: p.preshared_key,
                allowed_ips: p.allowed_ips,
                endpoint: p.endpoint,
                persistent_keepalive: p.persistent_keepalive.flatten(),
            })
        })
        .collect::<Result<Vec<_>, WireGuardError>>()?;
    Ok(WireGuardConfig { private_key, listen_port, fwmark, mtu, addresses, dns, dns_search, peers, ignored })
}

type SettingsMap<'a> = HashMap<&'static str, HashMap<&'static str, Value<'a>>>;

fn address_data<'a>(nets: impl Iterator<Item = &'a IpNet>) -> Value<'static> {
    let list: Vec<HashMap<&'static str, Value<'static>>> = nets
        .map(|n| HashMap::from([("address", Value::from(n.addr.to_string())), ("prefix", Value::U32(n.prefix.into()))]))
        .collect();
    Value::from(list)
}

impl WireGuardConfig {
    /// NM `AddConnection` ayarları (`type wireguard`). `name` hem bağlantı adı hem arayüz
    /// adıdır ([`valid_name`]). Otomatik bağlanma kapalı: VPN kullanıcı açınca bağlanır. Anahtar
    /// değerleri bu yapıdan ödünçtür.
    pub fn nm_settings(&self, name: &str) -> SettingsMap<'_> {
        let mut s: SettingsMap<'_> = HashMap::new();
        s.insert(
            "connection",
            HashMap::from([
                ("id", Value::from(name.to_owned())),
                ("type", Value::from("wireguard")),
                ("interface-name", Value::from(name.to_owned())),
                ("autoconnect", Value::Bool(false)),
            ]),
        );
        let mut wg: HashMap<&'static str, Value<'_>> =
            HashMap::from([("private-key", self.private_key.value()), ("private-key-flags", Value::U32(0))]);
        if let Some(p) = self.listen_port {
            wg.insert("listen-port", Value::U32(p.into()));
        }
        if let Some(m) = self.fwmark {
            wg.insert("fwmark", Value::U32(m));
        }
        if let Some(m) = self.mtu {
            wg.insert("mtu", Value::U32(m));
        }
        let peers: Vec<HashMap<&'static str, Value<'_>>> = self
            .peers
            .iter()
            .map(|p| {
                let mut m: HashMap<&'static str, Value<'_>> = HashMap::from([
                    ("public-key", Value::from(p.public_key.as_str())),
                    ("allowed-ips", Value::from(p.allowed_ips.iter().map(ToString::to_string).collect::<Vec<_>>())),
                ]);
                if let Some(k) = &p.preshared_key {
                    m.insert("preshared-key", k.value());
                    m.insert("preshared-key-flags", Value::U32(0));
                }
                if let Some(e) = &p.endpoint {
                    m.insert("endpoint", Value::from(e.as_str()));
                }
                if let Some(k) = p.persistent_keepalive {
                    m.insert("persistent-keepalive", Value::U32(k.into()));
                }
                m
            })
            .collect();
        wg.insert("peers", Value::from(peers));
        s.insert("wireguard", wg);

        let v4: Vec<&IpNet> = self.addresses.iter().filter(|n| n.addr.is_ipv4()).collect();
        let v6: Vec<&IpNet> = self.addresses.iter().filter(|n| n.addr.is_ipv6()).collect();
        let dns4: Vec<u32> = self
            .dns
            .iter()
            .filter_map(|d| match d {
                // NM `ipv4.dns`: ağ bayt sırasında u32.
                IpAddr::V4(a) => Some(u32::from_ne_bytes(a.octets())),
                IpAddr::V6(_) => None,
            })
            .collect();
        let dns6: Vec<Vec<u8>> = self
            .dns
            .iter()
            .filter_map(|d| match d {
                IpAddr::V6(a) => Some(a.octets().to_vec()),
                IpAddr::V4(_) => None,
            })
            .collect();
        let search = (!self.dns_search.is_empty()).then(|| Value::from(self.dns_search.clone()));

        let mut ip4: HashMap<&'static str, Value<'_>> = HashMap::new();
        if v4.is_empty() {
            ip4.insert("method", Value::from("disabled"));
        } else {
            ip4.insert("method", Value::from("manual"));
            ip4.insert("address-data", address_data(v4.into_iter()));
            if !dns4.is_empty() {
                ip4.insert("dns", Value::from(dns4));
            }
        }
        let mut ip6: HashMap<&'static str, Value<'_>> = HashMap::new();
        if v6.is_empty() {
            ip6.insert("method", Value::from("disabled"));
        } else {
            ip6.insert("method", Value::from("manual"));
            ip6.insert("address-data", address_data(v6.into_iter()));
            if !dns6.is_empty() {
                ip6.insert("dns", Value::from(dns6));
            }
        }
        if let Some(search) = search {
            // Arama alanları IPv4 varsa orada, yoksa IPv6'da.
            if ip4.contains_key("address-data") {
                ip4.insert("dns-search", search);
            } else {
                ip6.insert("dns-search", search);
            }
        }
        s.insert("ipv4", ip4);
        s.insert("ipv6", ip6);
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use WireGuardErrorKind as K;

    // wg(8) kılavuzundaki yayımlanmış örnek anahtar çifti: gerçek gizli bilgi değil.
    const PRIV: &str = "bnV4dXQtd2lyZWd1YXJkLXRlc3QtcHJpdmF0ZS1rMDE=";
    const PUB: &str = "bnV4dXQtd2lyZWd1YXJkLXRlc3QtcHVibGljLWtleTE=";
    // Uydurma örnek anahtarlar (32 baytlık okunur ASCII metnin base64'ü; hiçbir yerde
    // kullanılmaz, gerçek gizli bilgi değil).
    const PSK: &str = "bnV4dXQtd2lyZWd1YXJkLXRlc3QtcHJlc2hhcmVkLWs=";

    fn sample() -> String {
        format!(
            "# iş VPN\n[Interface]\nPrivateKey = {PRIV}\nAddress = 10.0.0.2/24, fd00::2/64\nDNS = 1.1.1.1, 2606:4700::1111, corp.example\n\
             ListenPort = 51820\nMTU = 1420\nPostUp = iptables -A FORWARD\n\n[Peer]\npublickey={PUB}\nPresharedKey = {PSK}\n\
             AllowedIPs = 0.0.0.0/0, ::/0\nEndpoint = vpn.example.com:51820\nPersistentKeepalive = 25\n"
        )
    }

    #[test]
    fn keys() {
        assert!(valid_key(PRIV));
        assert!(valid_key(PUB));
        assert!(!valid_key("kısa"));
        assert!(!valid_key(&PRIV.replace('=', "A")));
        // 43. karakterin alt bitleri sıfır olmalı (32 bayt).
        assert!(!valid_key("bnV4dXQtd2lyZWd1YXJkLXRlc3QtcHJpdmF0ZS1rMDF="));
        assert!(valid_name("wg0") && valid_name("is-vpn") && !valid_name("") && !valid_name("çok uzun bir ad ama"));
        assert!(!valid_name("a/b") && !valid_name(".."));
    }

    #[test]
    fn parses_full_config() {
        let c = parse(&sample()).unwrap();
        assert_eq!(c.private_key, Secret::new(PRIV));
        assert_eq!(c.addresses.len(), 2);
        assert_eq!(c.addresses[0].to_string(), "10.0.0.2/24");
        assert_eq!(c.dns.len(), 2);
        assert_eq!(c.dns_search, ["corp.example"]);
        assert_eq!((c.listen_port, c.mtu, c.fwmark), (Some(51820), Some(1420), None));
        assert_eq!(c.ignored, ["PostUp"]);
        let p = &c.peers[0];
        assert_eq!(p.public_key, PUB);
        assert_eq!(p.preshared_key, Some(Secret::new(PSK)));
        assert_eq!(p.allowed_ips.len(), 2);
        assert_eq!(p.endpoint.as_deref(), Some("vpn.example.com:51820"));
        assert_eq!(p.persistent_keepalive, Some(25));
        // Debug anahtar sızdırmaz.
        let dbg = format!("{c:?}");
        assert!(!dbg.contains(PRIV) && !dbg.contains(PSK));
    }

    #[test]
    fn errors_have_line_and_kind() {
        let e = |text: &str| parse(text).unwrap_err();
        assert_eq!(e(""), err(0, K::MissingInterface));
        assert_eq!(e("[Interface]\nAddress = 10.0.0.1/32\n"), err(0, K::MissingPrivateKey));
        assert_eq!(e("PrivateKey = x\n"), err(1, K::KeyOutsideSection));
        assert_eq!(e("[Interface]\nPrivateKey = gizli-değil\n"), err(2, K::InvalidKey));
        assert_eq!(e(&format!("[Interface]\nPrivateKey = {PRIV}\nPrivateKey = {PRIV}\n")), err(3, K::DuplicateKey));
        assert_eq!(e(&format!("[Interface]\nPrivateKey = {PRIV}\n[Interface]\n")), err(3, K::DuplicateInterface));
        assert_eq!(e(&format!("[Interface]\nPrivateKey = {PRIV}\nFoo = 1\n")), err(3, K::UnknownKey));
        assert_eq!(e(&format!("[Interface]\nPrivateKey = {PRIV}\n[Eş]\n")), err(3, K::UnknownSection));
        assert_eq!(e(&format!("[Interface]\nPrivateKey = {PRIV}\nAddress = 10.0.0.1/33\n")), err(3, K::InvalidAddress));
        assert_eq!(e(&format!("[Interface]\nPrivateKey = {PRIV}\nListenPort = 70000\n")), err(3, K::InvalidNumber));
        assert_eq!(e(&format!("[Interface]\nPrivateKey = {PRIV}\njunk\n")), err(3, K::Syntax));
        assert_eq!(e(&format!("[Interface]\nPrivateKey = {PRIV}\n[Peer]\nAllowedIPs = 0.0.0.0/0\n")), err(3, K::MissingPublicKey));
        assert_eq!(
            e(&format!("[Interface]\nPrivateKey = {PRIV}\n[Peer]\nPublicKey = {PUB}\nEndpoint = host\n")),
            err(5, K::InvalidEndpoint)
        );
        assert_eq!(e(&"#".repeat(MAX_CONF_BYTES + 1)), err(0, K::TooLarge));
        // Hata metni değer içermez.
        let shown = e("[Interface]\nPrivateKey = sızmamalı\n").to_string();
        assert!(!shown.contains("sızmamalı"), "{shown}");
    }

    /// Düşmanca girdiler (review şartı 4): panik yok, her biri tür + satırla reddedilir ya da
    /// zararsızca kabul edilir.
    #[test]
    fn hostile_inputs() {
        let e = |text: &str| parse(text).unwrap_err();
        let head = format!("[Interface]\nPrivateKey = {PRIV}\n");
        // 44 karakter, `=` ile bitiyor ama base64 dışı.
        let bad44 = format!("{}=", "!".repeat(43));
        assert_eq!(bad44.len(), 44);
        assert_eq!(e(&format!("[Interface]\nPrivateKey = {bad44}\n")), err(2, K::InvalidKey));
        assert_eq!(e(&format!("{head}[Peer]\nPublicKey = {}=\n", "é".repeat(21))), err(4, K::InvalidKey));
        // CRLF satır sonları: kabul, değerler `\r` taşımaz.
        let crlf = format!("[Interface]\r\nPrivateKey = {PRIV}\r\nAddress = 10.0.0.2/24\r\n[Peer]\r\nPublicKey = {PUB}\r\n");
        let c = parse(&crlf).unwrap();
        assert_eq!((c.private_key.expose(), c.peers[0].public_key.as_str()), (PRIV, PUB));
        assert_eq!(c.addresses[0].to_string(), "10.0.0.2/24");
        // NUL bayt: anahtarda, bölüm adında, anahtar adında, uç noktada, DNS'te.
        assert_eq!(e(&format!("[Interface]\nPrivateKey = {}\0\n", &PRIV[..43])), err(2, K::InvalidKey));
        assert_eq!(e("[Inter\0face]\n"), err(1, K::UnknownSection));
        assert_eq!(e(&format!("{head}Priv\0ateKey = {PRIV}\n")), err(3, K::UnknownKey));
        assert_eq!(e(&format!("{head}[Peer]\nPublicKey = {PUB}\nEndpoint = ho\0st:51820\n")), err(5, K::InvalidEndpoint));
        assert_eq!(e(&format!("{head}DNS = corp\0.example\n")), err(3, K::InvalidAddress));
        assert_eq!(e(&format!("{head}Address = 10.0.0.1\0/24\n")), err(3, K::InvalidAddress));
        // Sınıra yakın uzun satır (tek değer): hata, panik yok; sınırın hemen üstü TooLarge.
        let long = format!("{head}Address = {}\n", "1".repeat(MAX_CONF_BYTES - head.len() - 20));
        assert!(long.len() <= MAX_CONF_BYTES);
        assert_eq!(e(&long), err(3, K::InvalidAddress));
        let over = format!("{head}# {}\n", "x".repeat(MAX_CONF_BYTES));
        assert_eq!(e(&over), err(0, K::TooLarge));
        // TooMany: eş sayısı ve liste.
        let peers: String = (0..=MAX_PEERS).map(|_| format!("[Peer]\nPublicKey = {PUB}\n")).collect();
        assert_eq!(e(&format!("{head}{peers}")), err(3 + 2 * MAX_PEERS, K::TooMany));
        let many: Vec<String> = (0..=MAX_LIST).map(|i| format!("10.{}.{}.1/32", i / 256, i % 256)).collect();
        assert_eq!(e(&format!("{head}Address = {}\n", many.join(","))), err(3, K::TooMany));
        // Listeler satırlar boyunca toplanır: sınır yine geçerli.
        let half = many[..200].join(",");
        assert_eq!(e(&format!("{head}Address = {half}\nAddress = {half}\n")), err(4, K::TooMany));
        // Kapanışsız bölüm, boş anahtar adı, boş değerler.
        assert_eq!(e("[Interface\n"), err(1, K::Syntax));
        assert_eq!(e(&format!("{head}= değer\n")), err(3, K::Syntax));
        assert_eq!(e("[Interface]\nPrivateKey =\n"), err(2, K::InvalidKey));
        assert_eq!(e(&format!("{head}ListenPort =\n")), err(3, K::InvalidNumber));
        assert_eq!(e(&format!("{head}[Peer]\nPublicKey = \n")), err(4, K::InvalidKey));
        // Boş liste değeri hata değil (wg-quick gibi), işaret/eksi sayılar hata.
        assert!(parse(&format!("{head}Address =\n")).unwrap().addresses.is_empty());
        assert_eq!(e(&format!("{head}MTU = -1\n")), err(3, K::InvalidNumber));
        assert_eq!(e(&format!("{head}ListenPort = +51820\n")), err(3, K::InvalidNumber));
        // Hiçbir hata metni değer içermez.
        assert!(!e(&format!("[Interface]\nPrivateKey = {bad44}\n")).to_string().contains("!!!"));
    }

    #[test]
    fn endpoints_and_misc() {
        assert!(valid_endpoint("1.2.3.4:51820"));
        assert!(valid_endpoint("[2001:db8::1]:51820"));
        assert!(!valid_endpoint("[2001:db8::1]"));
        assert!(!valid_endpoint("host:0"));
        assert!(!valid_endpoint(":51820"));
        let c = parse(&format!("[Interface]\nPrivateKey = {PRIV}\nFwMark = 0x10\n[Peer]\nPublicKey = {PUB}\nPersistentKeepalive = off\n"))
            .unwrap();
        assert_eq!(c.fwmark, Some(16));
        assert_eq!(c.peers[0].persistent_keepalive, None);
        assert!(c.addresses.is_empty());
    }

    #[test]
    fn nm_settings_shape() {
        let c = parse(&sample()).unwrap();
        let s = c.nm_settings("is-vpn");
        assert_eq!(s["connection"]["type"], Value::from("wireguard"));
        assert_eq!(s["connection"]["interface-name"], Value::from("is-vpn"));
        assert_eq!(s["connection"]["autoconnect"], Value::Bool(false));
        assert_eq!(s["wireguard"]["private-key"], Value::from(PRIV));
        assert_eq!(s["wireguard"]["listen-port"], Value::U32(51820));
        assert_eq!(s["ipv4"]["method"], Value::from("manual"));
        assert_eq!(s["ipv6"]["method"], Value::from("manual"));
        assert_eq!(s["ipv4"]["dns"], Value::from(vec![u32::from_ne_bytes([1, 1, 1, 1])]));
        assert!(s["ipv4"].contains_key("dns-search"));
        // Özel anahtar ödünç (kopya yok).
        match &s["wireguard"]["private-key"] {
            Value::Str(v) => assert_eq!(v.as_str().as_ptr(), c.private_key.expose().as_ptr()),
            other => panic!("{other:?}"),
        }
        let sig = s["wireguard"]["peers"].value_signature().to_string();
        assert_eq!(sig, "aa{sv}");
        let bare = parse(&format!("[Interface]\nPrivateKey = {PRIV}\n")).unwrap();
        let s = bare.nm_settings("wg0");
        assert_eq!(s["ipv4"]["method"], Value::from("disabled"));
        assert_eq!(s["ipv6"]["method"], Value::from("disabled"));
    }
}
