//! StatusNotifierItem verisi: `org.kde.StatusNotifierItem` özelliklerinin sınırlı, tipli hali.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use zbus::zvariant::{OwnedValue, Value};

use super::value::{as_array, as_bool, as_bytes, as_fields, as_i64, as_str, clip, clip_line};

/// Kimlik, başlık, ikon adı gibi tek satırlık alanların en fazla karakter sayısı.
pub const MAX_NAME_CHARS: usize = 256;
/// İpucu açıklamasının en fazla karakter sayısı.
pub const MAX_DESCRIPTION_CHARS: usize = 512;
/// Yol alanlarının (ikon tema yolu, menü nesne yolu) en fazla bayt sayısı.
pub const MAX_PATH_BYTES: usize = 1024;
/// Bir pixmap'in en büyük kenarı.
pub const MAX_PIXMAP_DIM: u32 = 512;
/// Bir ikon için tutulan en fazla pixmap boyutu.
pub const MAX_PIXMAPS: usize = 8;
/// Bir ikonun tüm pixmap'lerinin toplam bayt sınırı.
pub const MAX_PIXMAP_BYTES: usize = 4 * 1024 * 1024;

/// Öğe kimliği: `<benzersiz bus adı><nesne yolu>`, ör. `:1.42/StatusNotifierItem`. Aynı
/// uygulama aynı nesneyi iki kez (iyi bilinen ad ve yol ile) kaydetse de tek öğedir.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ItemId(Arc<str>);

impl ItemId {
    pub(crate) fn new(owner: &str, path: &str) -> Self {
        Self(format!("{owner}{path}").into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Category {
    #[default]
    ApplicationStatus,
    Communications,
    SystemServices,
    Hardware,
}

impl Category {
    fn parse(s: &str) -> Self {
        match s {
            "Communications" => Self::Communications,
            "SystemServices" => Self::SystemServices,
            "Hardware" => Self::Hardware,
            _ => Self::ApplicationStatus,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Status {
    /// Gizlenebilir (ör. ağ bağlı, bildirilecek bir şey yok).
    Passive,
    #[default]
    Active,
    /// Dikkat ikonu gösterilmeli.
    NeedsAttention,
}

impl Status {
    /// Bilinmeyen değer `Active` kabul edilir (öğe kaybolmasın).
    pub fn parse(s: &str) -> Self {
        match s {
            "Passive" => Self::Passive,
            "NeedsAttention" => Self::NeedsAttention,
            _ => Self::Active,
        }
    }
}

/// Düz (premultiplied olmayan) RGBA8 piksel verisi. SNI'nin ağ bayt sıralı ARGB32'sinden
/// çevrilmiştir. `rgba.len() == width * height * 4`.
#[derive(Clone, PartialEq, Eq)]
pub struct Pixmap {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
}

impl fmt::Debug for Pixmap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Pixmap({}x{})", self.width, self.height)
    }
}

impl Pixmap {
    /// `(iiay)` yapısını çözer. Geçersiz boyut, eksik/fazla veri → `None`.
    fn parse(value: &Value<'_>) -> Option<Self> {
        let [w, h, data] = as_fields(value)? else { return None };
        let width = u32::try_from(as_i64(w)?).ok()?;
        let height = u32::try_from(as_i64(h)?).ok()?;
        if !(1..=MAX_PIXMAP_DIM).contains(&width) || !(1..=MAX_PIXMAP_DIM).contains(&height) {
            return None;
        }
        let len = width as usize * height as usize * 4;
        let argb = as_bytes(data, len)?;
        if argb.len() != len {
            return None;
        }
        Some(Self { width, height, rgba: argb_to_rgba(&argb).into() })
    }

    fn size(&self) -> u32 {
        self.width.max(self.height)
    }
}

/// Ağ bayt sıralı ARGB32 (`A R G B`) → `R G B A`.
pub fn argb_to_rgba(argb: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(argb.len());
    for px in argb.chunks_exact(4) {
        out.extend_from_slice(&[px[1], px[2], px[3], px[0]]);
    }
    out
}

/// Bir ikonun farklı boyutlardaki pixmap'leri, küçükten büyüğe.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Pixmaps(Vec<Pixmap>);

impl Pixmaps {
    /// `a(iiay)` dizisini çözer. Geçersiz girdiler atlanır; aynı boyuttan biri tutulur;
    /// sayı ve toplam bayt sınırlanır (büyükler önce atılır).
    pub(crate) fn parse(value: Option<&Value<'_>>) -> Self {
        let Some(items) = value.and_then(as_array) else { return Self::default() };
        // Kötü niyetli uygulama binlerce girdi gönderebilir: yalnızca ilk birkaç düzinesine bakılır.
        let mut list: Vec<Pixmap> = items.iter().take(4 * MAX_PIXMAPS).filter_map(Pixmap::parse).collect();
        list.sort_by_key(|p| (p.size(), p.width, p.height));
        list.dedup_by_key(|p| (p.width, p.height));
        let mut total = 0;
        list.retain(|p| {
            total += p.rgba.len();
            total <= MAX_PIXMAP_BYTES
        });
        list.truncate(MAX_PIXMAPS);
        Self(list)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn all(&self) -> &[Pixmap] {
        &self.0
    }

    /// `size` piksel (fiziksel) için en uygun pixmap: kenarı `size`'a eşit veya büyük olan
    /// en küçüğü (küçültmek büyütmekten iyi görünür); yoksa en büyüğü.
    pub fn best(&self, size: u32) -> Option<&Pixmap> {
        self.0.iter().find(|p| p.size() >= size).or_else(|| self.0.last())
    }
}

/// Ad (tema ikonu) veya pixmap ile verilen ikon. İkisi de boş olabilir.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Icon {
    /// Tema ikon adı veya mutlak dosya yolu (bazı uygulamalar yol verir).
    pub name: String,
    pub pixmaps: Pixmaps,
}

impl Icon {
    pub fn is_empty(&self) -> bool {
        self.name.is_empty() && self.pixmaps.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ToolTip {
    pub icon_name: String,
    pub title: String,
    /// Bazı uygulamalar basit HTML gönderir; çizim tarafı düz metin olarak göstermelidir.
    pub description: String,
}

impl ToolTip {
    fn parse(value: Option<&Value<'_>>) -> Self {
        let Some(fields) = value.and_then(as_fields) else { return Self::default() };
        let text = |i: usize| fields.get(i).and_then(as_str).unwrap_or_default();
        // (s icon_name, a(iiay) icon_pixmap, s title, s description); pixmap tutulmaz.
        Self {
            icon_name: clip_line(text(0), MAX_NAME_CHARS),
            title: clip_line(text(2), MAX_NAME_CHARS),
            description: clip(text(3), MAX_DESCRIPTION_CHARS),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.title.is_empty() && self.description.is_empty()
    }
}

/// Tepsideki bir öğenin anlık görüntüsü.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayItem {
    pub id: ItemId,
    /// Kayıt sırası; model bu sıraya göre dizer (kararlı sıra).
    pub seq: u64,
    /// Öğeyi sunan bağlantının benzersiz bus adı.
    pub bus_name: String,
    pub object_path: String,
    /// Uygulamanın verdiği kimlik (`Id`), ör. `nm-applet`.
    pub app_id: String,
    pub category: Category,
    pub status: Status,
    pub title: String,
    pub icon: Icon,
    pub attention_icon: Icon,
    pub overlay_icon: Icon,
    /// Uygulamaya özel ikon arama dizini (mutlak yol) veya boş.
    pub icon_theme_path: String,
    pub tooltip: ToolTip,
    /// `true`: öğe yalnızca menü sunar; birincil tıklama da menüyü açmalı.
    pub item_is_menu: bool,
    /// DBusMenu nesne yolu (`com.canonical.dbusmenu`), yoksa `None`.
    pub menu: Option<String>,
    /// Çizime hazır ikon (düz RGBA, servisin ikon boyutunda): tema adı arka planda çözülür,
    /// ad yoksa ya da bulunamazsa uygulamanın pixmap'i. Yoksa `None` (UI yer tutucu çizer).
    pub image: Option<Pixmap>,
}

impl TrayItem {
    /// O an gösterilecek ikon: dikkat istiyorsa ve dikkat ikonu varsa o, değilse normal ikon.
    pub fn current_icon(&self) -> &Icon {
        if self.status == Status::NeedsAttention && !self.attention_icon.is_empty() {
            &self.attention_icon
        } else {
            &self.icon
        }
    }

    /// Kullanıcıya gösterilecek ad: başlık, ipucu başlığı, uygulama kimliği sırasıyla.
    pub fn display_name(&self) -> &str {
        [&self.title, &self.tooltip.title, &self.app_id].into_iter().find(|s| !s.is_empty()).map_or("", |s| s)
    }

    /// `GetAll` cevabından öğe kurar. Eksik/yanlış tipli alanlar varsayılan değer alır.
    pub(crate) fn from_properties(
        id: ItemId,
        seq: u64,
        bus_name: &str,
        object_path: &str,
        props: &HashMap<String, OwnedValue>,
    ) -> Self {
        let get = |key: &str| props.get(key).map(|v| &**v);
        let name = |key: &str| get(key).and_then(as_str).map(|s| clip_line(s, MAX_NAME_CHARS)).unwrap_or_default();
        let icon = |name_key: &str, pixmap_key: &str| Icon { name: name(name_key), pixmaps: Pixmaps::parse(get(pixmap_key)) };
        let icon_theme_path = get("IconThemePath")
            .and_then(as_str)
            .filter(|p| p.starts_with('/') && p.len() <= MAX_PATH_BYTES && !p.chars().any(char::is_control))
            .unwrap_or_default()
            .to_owned();
        let menu = get("Menu")
            .and_then(as_str)
            .filter(|p| p.starts_with('/') && *p != "/" && *p != "/NO_DBUSMENU" && p.len() <= MAX_PATH_BYTES)
            .map(str::to_owned);
        Self {
            id,
            seq,
            bus_name: bus_name.to_owned(),
            object_path: object_path.to_owned(),
            app_id: name("Id"),
            category: Category::parse(get("Category").and_then(as_str).unwrap_or_default()),
            status: Status::parse(get("Status").and_then(as_str).unwrap_or_default()),
            title: name("Title"),
            icon: icon("IconName", "IconPixmap"),
            attention_icon: icon("AttentionIconName", "AttentionIconPixmap"),
            overlay_icon: icon("OverlayIconName", "OverlayIconPixmap"),
            icon_theme_path,
            tooltip: ToolTip::parse(get("ToolTip")),
            item_is_menu: get("ItemIsMenu").and_then(as_bool).unwrap_or(false),
            menu,
            image: None,
        }
    }
}

/// `GetAll` desteklemeyen uygulamalar için tek tek okunan özellikler.
pub(crate) const ITEM_PROPERTIES: &[&str] = &[
    "Id",
    "Category",
    "Status",
    "Title",
    "IconName",
    "IconPixmap",
    "IconThemePath",
    "AttentionIconName",
    "AttentionIconPixmap",
    "OverlayIconName",
    "OverlayIconPixmap",
    "ToolTip",
    "ItemIsMenu",
    "Menu",
];

#[cfg(test)]
mod tests {
    use zbus::zvariant::{Array, ObjectPath, Structure};

    use super::*;

    fn pixmap_value(w: i32, h: i32, bytes: Vec<u8>) -> Value<'static> {
        Value::Structure(Structure::from((w, h, Array::from(bytes))))
    }

    fn pixmaps_value(list: Vec<Value<'static>>) -> Value<'static> {
        let mut a = Array::new(&"(iiay)".try_into().unwrap());
        for v in list {
            a.append(v).unwrap();
        }
        Value::Array(a)
    }

    #[test]
    fn argb_network_order_is_converted() {
        assert_eq!(argb_to_rgba(&[0x80, 1, 2, 3, 0xff, 4, 5, 6]), [1, 2, 3, 0x80, 4, 5, 6, 0xff]);
    }

    #[test]
    fn pixmaps_are_validated_sorted_and_bounded() {
        let v = pixmaps_value(vec![
            pixmap_value(2, 2, vec![0xff; 16]),
            pixmap_value(1, 1, vec![0xff, 1, 2, 3]),
            pixmap_value(2, 2, vec![0; 15]),       // eksik veri
            pixmap_value(0, 4, vec![]),            // sıfır boyut
            pixmap_value(-1, 1, vec![0; 4]),       // negatif
            pixmap_value(600, 1, vec![0; 2400]),   // sınır dışı
            pixmap_value(2, 2, vec![0; 16]),       // aynı boyut tekrar
        ]);
        let p = Pixmaps::parse(Some(&v));
        let sizes: Vec<_> = p.all().iter().map(|p| (p.width, p.height)).collect();
        assert_eq!(sizes, [(1, 1), (2, 2)]);
        assert_eq!(&*p.all()[0].rgba, &[1, 2, 3, 0xff]);
        assert_eq!(p.best(1).unwrap().width, 1);
        assert_eq!(p.best(2).unwrap().width, 2);
        assert_eq!(p.best(64).unwrap().width, 2, "büyüğü yoksa en büyüğü");
        assert!(Pixmaps::parse(Some(&Value::from("x"))).is_empty());
        assert!(Pixmaps::parse(None).best(16).is_none());
    }

    #[test]
    fn pixmap_count_and_bytes_are_limited() {
        let list = (1..=20).map(|n| pixmap_value(n, n, vec![0; (n * n * 4) as usize])).collect();
        assert_eq!(Pixmaps::parse(Some(&pixmaps_value(list))).all().len(), MAX_PIXMAPS);
        let big = (0..8).map(|i| pixmap_value(512, 512 - i, vec![0; 512 * (512 - i as usize) * 4])).collect();
        let p = Pixmaps::parse(Some(&pixmaps_value(big)));
        assert!(p.all().iter().map(|p| p.rgba.len()).sum::<usize>() <= MAX_PIXMAP_BYTES);
        assert_eq!(p.all().len(), 4);
    }

    fn props(entries: Vec<(&str, Value<'static>)>) -> HashMap<String, OwnedValue> {
        entries.into_iter().map(|(k, v)| (k.to_owned(), OwnedValue::try_from(v).unwrap())).collect()
    }

    #[test]
    fn item_from_properties() {
        let tooltip = Value::Structure(Structure::from(("tip-icon", Array::new(&"(iiay)".try_into().unwrap()), "Başlık", "açıklama\nikinci")));
        let p = props(vec![
            ("Id", Value::from("nm-applet")),
            ("Category", Value::from("Hardware")),
            ("Status", Value::from("NeedsAttention")),
            ("Title", Value::from("Ağ\u{7}")),
            ("IconName", Value::from("nm-signal-75")),
            ("AttentionIconName", Value::from("nm-alert")),
            ("IconThemePath", Value::from("relative/not/allowed")),
            ("ToolTip", tooltip),
            ("ItemIsMenu", Value::from(true)),
            ("Menu", Value::ObjectPath(ObjectPath::try_from("/MenuBar").unwrap())),
            ("Unknown", Value::from(42u32)),
        ]);
        let item = TrayItem::from_properties(ItemId::new(":1.5", "/SNI"), 3, ":1.5", "/SNI", &p);
        assert_eq!(item.id.as_str(), ":1.5/SNI");
        assert_eq!(item.app_id, "nm-applet");
        assert_eq!(item.category, Category::Hardware);
        assert_eq!(item.status, Status::NeedsAttention);
        assert_eq!(item.title, "Ağ");
        assert_eq!(item.current_icon().name, "nm-alert");
        assert_eq!(item.icon_theme_path, "");
        assert_eq!(item.tooltip, ToolTip { icon_name: "tip-icon".into(), title: "Başlık".into(), description: "açıklama\nikinci".into() });
        assert!(item.item_is_menu);
        assert_eq!(item.menu.as_deref(), Some("/MenuBar"));
        assert_eq!(item.display_name(), "Ağ");
    }

    #[test]
    fn item_tolerates_garbage() {
        let p = props(vec![
            ("Id", Value::from(5i32)),
            ("Status", Value::from("Bogus")),
            ("Title", Value::from("x".repeat(10_000))),
            ("ItemIsMenu", Value::from("yes")),
            ("Menu", Value::ObjectPath(ObjectPath::try_from("/NO_DBUSMENU").unwrap())),
            ("ToolTip", Value::from(1u8)),
            ("IconPixmap", Value::from("nope")),
        ]);
        let item = TrayItem::from_properties(ItemId::new(":1.5", "/SNI"), 0, ":1.5", "/SNI", &p);
        assert_eq!(item.app_id, "");
        assert_eq!(item.status, Status::Active);
        assert_eq!(item.title.chars().count(), MAX_NAME_CHARS);
        assert!(!item.item_is_menu);
        assert_eq!(item.menu, None);
        assert!(item.tooltip.is_empty());
        assert!(item.icon.is_empty());
    }
}
