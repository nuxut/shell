//! Ağ: NetworkManager istemcisi `nuxut-nm` crate'indedir (Ayarlar ile paylaşılır; UI'sız).
//! Burada yeniden dışa aktarma ve kabuğa özgü ajan kimliği seçimi ([`agent_id_for`]); hata
//! türünün kullanıcı metni `system.rs`'de (`network_error_text` / `bluetooth_error_text`).

pub use nuxut_nm::*;

/// İkincil (test) örneğin gerçek NM'ye ajan kaydetmesine izin veren ortam değişkeni (`1`).
pub const TEST_AGENT_ENV: &str = "NUXUT_TEST_NM_AGENT";

/// Örnek adına göre ajan kimliği. Ana örnek [`AGENT_ID`]. İkincil (test) örnek
/// (`NUXUT_SHELL_INSTANCE`) sistem bus'ı kullanıcıyla ortak olduğundan varsayılan olarak hiç
/// kaydolmaz: aynı kimlikle kullanıcının shell'inin ajanını engeller, ayrı kimlikle de
/// kullanıcının Wi-Fi şifre isteklerini kendine çekebilirdi. Yalnızca `opt_in` ile
/// (`NUXUT_TEST_NM_AGENT=1`) kendi kimliğiyle (`org.nuxut.shell.<ad>`) kaydolur. Ad NM'nin
/// kimlik kuralına (`[A-Za-z0-9_.-]`, 3–255, `..` yok) uymuyorsa `None`.
pub fn agent_id_for(instance: Option<&str>, opt_in: bool) -> Option<String> {
    match instance {
        None | Some("") => Some(AGENT_ID.to_owned()),
        Some(_) if !opt_in => None,
        Some(name) => {
            let ok = name.len() <= 64
                && !name.starts_with('.')
                && !name.ends_with('.')
                && !name.contains("..")
                && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'));
            ok.then(|| format!("{AGENT_ID}.{name}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_id_per_instance() {
        assert_eq!(agent_id_for(None, false).as_deref(), Some(AGENT_ID));
        assert_eq!(agent_id_for(Some(""), true).as_deref(), Some(AGENT_ID));
        // İkincil örnek kullanıcının kimliğini asla almaz; izinsiz hiç kaydolmaz.
        assert_eq!(agent_id_for(Some("test-system"), false), None);
        assert_eq!(agent_id_for(Some("test-system"), true).as_deref(), Some("org.nuxut.shell.test-system"));
        assert_eq!(agent_id_for(Some("a..b"), true), None);
        assert_eq!(agent_id_for(Some("a b"), true), None);
        assert_eq!(agent_id_for(Some("x."), true), None);
    }
}
