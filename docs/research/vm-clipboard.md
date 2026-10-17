# Sanal makinede pano paylaşımı (X11 ↔ Wayland)

Tarih: 2026-10-16. İstek: kullanıcı.
Sonuç ve tasarım: `docs/design/vm-clipboard.md`.

## Sorun

Kullanıcı, Nuxut'u bir QEMU/KVM (SPICE) konuğunda çalıştırırken ana makine ile kopyala-yapıştır
yapamıyordu ve bunun için `~/hypr-bridge.sh` yazdı (`exec-once` ile her oturumda):

- Wayland → X11: `wl-paste --type text --watch xclip -selection clipboard` (olay tabanlı).
- X11 → Wayland: 0,1 sn'de bir `xclip -o` yoklaması + `wl-copy` (boşta saniyede 10 süreç
  başlatma, yalnızca metin, görsel ve diğer türler yok, içerik her turda kabukta değişkende).

Bu VM'de ölçülen durum (2026-10-16): Hyprland 0.56.2, `spice-vdagent 0.23.0-1` (Arch),
`spice-vdagent -x` oturum ajanı + `spice-vdagentd -x` sistem daemon'u çalışıyor,
`xorg-xwayland 24.1.14`, `DISPLAY=:1`, `systemd-detect-virt` → `kvm`.

## 1. spice-vdagent ve Wayland

- spice-vdagent'in oturum ajanı panoyu **yalnızca X11 seçimleriyle** konuşur (XFixes ile
  `CLIPBOARD`/`PRIMARY` sahibini izler, `TARGETS` ile türleri ana makineye bildirir, içeriği
  ana makine yapıştırınca dönüştürür). Wayland oturumunda Xwayland üzerinden çalışır; X11
  istemcisi olarak penceresi görünmez, **klavye odağı hiç ona geçmez**.
- Upstream'de yerli Wayland desteği **yok** (Arch 0.23.0). freedesktop `vd_agent` sorun #26
  "Add support for wayland clipboard" açık; `ext-data-control-v1` kullanan bir dal
  (`v-dermichev/vd_agent`, `vd_agent_proposal_wlr`) Temmuz 2026 itibarıyla birleşmemiş
  (Whonix forumu, 2026-07-16; Fedora COPR'da `spice-vdagent-wayland` paketi: Xwayland + ext-data-control
  ister, KWin 6.4+, sway 1.11+, niri 25.08+ ile denenmiş). Sorun #26'ya göre GNOME'da Mutter
  sayesinde çalışıyor, sway'de çalışmıyor.
- Sonuç: yakın vadede konuk ajanı X11 konuşmaya devam edecek; köprü compositor (ya da kabuk)
  tarafında kurulmalı.

## 2. Hyprland'in Xwayland pano eşitlemesi ve neden yetmediği

Kaynak: Hyprland v0.56.2 `src/xwayland/XWM.cpp` (incelendi 2026-10-16).

- **Wayland → X11:** `SXSelection::onSelection` Wayland seçimi değişince X11 `CLIPBOARD`'ı
  XWM penceresiyle almak ister, ama Xwayland klavye odağında değilse `notifyOnFocus` ile
  **ertelenir**; `onKeyboardFocus` yalnızca odak Xwayland istemcisine geçince sahipliği alır.
  Ayrıca `CXWM::handleSelectionRequest`, X11 istemcisinden gelen isteği Xwayland odakta değilse
  reddeder ("Ignoring clipboard access: xwayland not in focus").
- **X11 → Wayland:** `handleSelectionXFixesNotify` her sahip değişiminde `TARGETS` ister, ama
  `handleSelectionNotify` odakta bir X11 yüzeyi yoksa reddeder ("denying access to write to
  clipboard because no X client is in focus"); `setClipboardToWayland` çağrılmaz.
- Yani iki yön de **odak korumalı** (arka plandaki X11 istemcisinin panoyu okumasını/yazmasını
  engelleyen bilinçli bir gizlilik kararı). spice-vdagent'in penceresi olmadığı için koşul hiç
  sağlanmaz: VM'de pano iki yönde de akmaz. Kullanıcının betiği bu yüzden gerekiyordu.

## 3. GNOME ve KDE nasıl çözüyor

- **GNOME (Mutter)**, `src/x11/meta-x11-selection.c` (main, 2026-10-16): X11 `CLIPBOARD`/`PRIMARY`
  sahip değişimi (XFixes) **koşulsuz** compositor seçimine alınır
  (`meta_x11_selection_handle_xfixes_selection_notify` → `meta_selection_source_x11_new_async`
  → `meta_selection_set_owner`), compositor seçimi değişince X11 seçimi **koşulsuz** Mutter'ın
  penceresiyle alınır (`notify_selection_owner` → `XSetSelectionOwner`). Odak ya da etkin
  pencere denetimi yok. Bu yüzden GNOME'da spice-vdagent kutudan çıktığı gibi çalışır.
- **KDE (KWin)**, `src/xwayland/clipboard.cpp` (master, 2026-10-16): X11 → Wayland yönünde odak
  denetimi görülmedi (`selectionClaimed` → `targetsReceived` → `seat()->setSelection`). Wayland →
  X11 yönü `x11ClientsCanAccessSelection()` ile **etkin pencere X11 ise** açılır
  (`onActiveWindowChanged` ile güncellenir). Yani KDE'de ana makine → konuk yönü çalışmalı,
  konuk → ana makine yönü bir X11 penceresi etkin değilken çalışmayabilir. KWin 6.4+ ayrıca
  ext-data-control sunar (yukarıdaki Wayland dalı için). Gerçek bir KDE konuğunda **doğrulanmadı**.
- Kullanıcının "KDE ve GNOME'da gerek yok" gözlemi GNOME için tam, KDE için kısmen doğru.

## 4. VirtualBox ve VMware konuk araçları

- **VMware (open-vm-tools):** pano `vmtoolsd -n vmusr` (kullanıcı oturumu eklentisi) içinde, GTK
  X11 panosuyla. Wayland oturumunda Xwayland üzerinden; aynı odak sorunu Hyprland'de geçerli.
  Süreç adı `vmtoolsd`.
- **VirtualBox Guest Additions:** pano `VBoxClient --clipboard` (X11). 7.x sürümlerinde
  Wayland için deneysel bir yol (`VBoxClient --wayland` / GNOME ve KDE'ye özel yardımcılar)
  duyuruldu; Hyprland için kullanılabilir olduğu **doğrulanmadı**. Süreç adı `VBoxClient`.
- **QEMU guest agent (`qemu-ga`)** panoyla ilgilenmez; SPICE yolu spice-vdagent'tir.
- Ortak nokta: bugün üç ajan da X11 seçimini konuşur; tek bir X11 ↔ Wayland köprüsü üçünü de
  kapsar.

## 5. Seçenekler

| Seçenek | Artı | Eksi |
|---|---|---|
| Kullanıcı betiği (`wl-paste --watch` + `xclip` yoklaması) | Basit | 0,1 sn yoklama (boşta sürekli süreç), yalnız metin, döngü ve yarış riski, içerik kabuk değişkeninde |
| Hyprland'i yamalamak / ayar istemek | Doğru katman | Bizim elimizde değil; odak koruması bilinçli |
| Konuk ajanının Wayland dalı | Kökten çözüm | Birleşmemiş, paketlenmemiş, Hyprland ext-data-control sunsa da Xwayland ister |
| **Kabukta olay tabanlı köprü (seçilen)** | Boşta sıfır iş, tüm türler, döngü önleme, gizlilik kuralları, kabuğun data-control bağlantısını kullanır | Hyprland'in odak korumasını (yalnızca köprü açıkken) kaldırır → yalnızca VM'de varsayılan |

## Kaynaklar

- Hyprland v0.56.2 `src/xwayland/XWM.cpp` — https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/xwayland/XWM.cpp
- Mutter `src/x11/meta-x11-selection.c` — https://gitlab.gnome.org/GNOME/mutter/-/blob/main/src/x11/meta-x11-selection.c
- KWin `src/xwayland/clipboard.cpp` — https://invent.kde.org/plasma/kwin/-/blob/master/src/xwayland/clipboard.cpp
- spice vd_agent sorun #26 — https://gitlab.freedesktop.org/spice/linux/vd_agent/-/issues/26
- Whonix forumu (Wayland dalı, 2026-07) — https://forums.whonix.org/t/replacing-spice-vdagent-spice-vdagentd-to-fix-clipboard-sharing/23409
- Fedora COPR `spice-vdagent-wayland` — https://copr-be.cloud.fedoraproject.org/results/jackyzy823/myown/srpm-builds/09512584/spice-vdagent-wayland.spec
- ICCCM §2 (seçimler, INCR) — https://x.org/releases/X11R7.6/doc/xorg-docs/specs/ICCCM/icccm.html
