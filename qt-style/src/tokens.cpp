#include "tokens.h"

#include <QDir>
#include <QFile>

namespace nuxut {

namespace {

bool toInt(const QByteArray &v, int lo, int hi, int *out)
{
    bool ok = false;
    const int n = v.toInt(&ok);
    if (!ok)
        return false;
    *out = qBound(lo, n, hi);
    return true;
}

// Yalnızca `#rrggbb` ya da `#rrggbbaa` (shell'in yazdığı biçim; Qt'nin `#aarrggbb` sırası değil).
bool toColor(const QByteArray &v, QColor *out)
{
    if (!v.startsWith('#') || (v.size() != 7 && v.size() != 9))
        return false;
    bool ok = false;
    const uint rgb = v.mid(1, 6).toUInt(&ok, 16);
    if (!ok)
        return false;
    int a = 255;
    if (v.size() == 9) {
        a = int(v.mid(7, 2).toUInt(&ok, 16));
        if (!ok)
            return false;
    }
    *out = QColor(int((rgb >> 16) & 0xff), int((rgb >> 8) & 0xff), int(rgb & 0xff), a);
    return true;
}

} // namespace

bool Tokens::operator==(const Tokens &o) const
{
    return radiusSmall == o.radiusSmall && radiusMedium == o.radiusMedium && focusRing == o.focusRing
        && focusColor == o.focusColor && spacing == o.spacing;
}

Tokens Tokens::parse(const QByteArray &text, bool *ok)
{
    Tokens t;
    if (ok)
        *ok = false;
    if (text.size() > MaxBytes)
        return t;
    bool versionOk = false;
    const QList<QByteArray> lines = text.split('\n');
    for (const QByteArray &raw : lines) {
        const QByteArray line = raw.trimmed();
        if (line.isEmpty() || line.startsWith('#') || line.startsWith(';') || line.startsWith('['))
            continue;
        const int eq = line.indexOf('=');
        if (eq <= 0)
            continue;
        const QByteArray key = line.left(eq).trimmed();
        const QByteArray value = line.mid(eq + 1).trimmed();
        int n = 0;
        if (key == "version") {
            versionOk = value.toInt() == Version;
        } else if (key == "radius_small" && toInt(value, 0, 32, &n)) {
            t.radiusSmall = n;
        } else if (key == "radius_medium" && toInt(value, 0, 48, &n)) {
            t.radiusMedium = n;
        } else if (key == "focus_ring" && toInt(value, 1, 4, &n)) {
            t.focusRing = n;
        } else if (key == "spacing" && toInt(value, 2, 8, &n)) {
            t.spacing = n;
        } else if (key == "focus_color") {
            QColor c;
            if (toColor(value, &c))
                t.focusColor = c;
        }
    }
    if (!versionOk)
        return Tokens();
    if (ok)
        *ok = true;
    return t;
}

Tokens Tokens::load(const QString &path)
{
    QFile f(path);
    if (!f.open(QIODevice::ReadOnly))
        return Tokens();
    // Sınırdan büyük dosya bütünüyle reddedilir (bir bayt fazlası okunur).
    return parse(f.read(MaxBytes + 1));
}

QString Tokens::path()
{
    QString base = QString::fromLocal8Bit(qgetenv("XDG_CONFIG_HOME"));
    if (base.isEmpty() || !QDir::isAbsolutePath(base))
        base = QDir::homePath() + QStringLiteral("/.config");
    return base + QStringLiteral("/nuxut/qt-style.conf");
}

} // namespace nuxut
