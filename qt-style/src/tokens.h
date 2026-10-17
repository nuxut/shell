// Nuxut Qt stili: shell'in yazdığı token dosyası (`$XDG_CONFIG_HOME/nuxut/qt-style.conf`).
// Biçim satır başına `anahtar=değer`; `#` ve `;` yorum. Bilinmeyen anahtar yok sayılır, her sayı
// sınırlanır, dosya en çok MaxBytes okunur: bozuk dosya uygulamayı çökertmez, varsayılana düşer.
#pragma once

#include <QByteArray>
#include <QColor>
#include <QString>

namespace nuxut {

struct Tokens {
    // Biçim sürümü; tanınmayan sürümde dosya yok sayılır.
    static constexpr int Version = 1;
    static constexpr int MaxBytes = 4096;

    int radiusSmall = 8;   // düğme, alan, açılır kutu
    int radiusMedium = 14; // çerçeve, sekme paneli
    int focusRing = 2;     // klavye odak halkası kalınlığı (framework FOCUS_RING)
    QColor focusColor;     // geçersizse paletin Highlight rengi
    int spacing = 4;       // boşluk ızgarası birimi (framework SPACING_GRID)

    bool operator==(const Tokens &o) const;
    bool operator!=(const Tokens &o) const { return !(*this == o); }

    // Metni çözer; geçersiz satırlar atlanır, `ok` dosyanın kullanılıp kullanılmadığını söyler.
    static Tokens parse(const QByteArray &text, bool *ok = nullptr);
    // Dosyadan okur (yoksa/bozuksa varsayılanlar).
    static Tokens load(const QString &path);
    // `$XDG_CONFIG_HOME/nuxut/qt-style.conf` (yoksa `~/.config/...`).
    static QString path();
};

} // namespace nuxut
