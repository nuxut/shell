// Nuxut Qt stili testleri (offscreen QPA; ctest ortamı CMakeLists.txt'te). Eklenti dosyadan
// yüklenir: test, kurulan .so'nun aynısını sınar.
#include <QApplication>
#include <QDir>
#include <QElapsedTimer>
#include <QFile>
#include <QImage>
#include <QLineEdit>
#include <QPluginLoader>
#include <QPushButton>
#include <QStyle>
#include <QStyleFactory>
#include <QStylePlugin>
#include <QTemporaryDir>
#include <QVBoxLayout>

#include <cstdio>
#include <cstdlib>

#include "tokens.h"

static int failures = 0;

#define CHECK(cond)                                                                    \
    do {                                                                               \
        if (!(cond)) {                                                                 \
            std::fprintf(stderr, "BAŞARISIZ %s:%d: %s\n", __FILE__, __LINE__, #cond); \
            ++failures;                                                                \
        }                                                                              \
    } while (0)

static void write(const QString &path, const QByteArray &text)
{
    QDir().mkpath(QFileInfo(path).absolutePath());
    // Shell gibi atomik: geçici dosya + yeniden adlandırma.
    QFile f(path + QStringLiteral(".tmp"));
    if (!f.open(QIODevice::WriteOnly))
        std::abort();
    f.write(text);
    f.close();
    QFile::remove(path);
    QFile::rename(path + QStringLiteral(".tmp"), path);
}

static void tokens()
{
    bool ok = false;
    auto t = nuxut::Tokens::parse("# Nuxut — x\nversion=1\nradius_small=12\nfocus_ring=3\nfocus_color=#ff00ff80\nspacing=4\nbilinmeyen=1\n", &ok);
    CHECK(ok);
    CHECK(t.radiusSmall == 12 && t.focusRing == 3 && t.spacing == 4);
    CHECK(t.focusColor == QColor(255, 0, 255, 128));

    // Sürüm yok/yanlış: dosya yok sayılır.
    t = nuxut::Tokens::parse("radius_small=20\n", &ok);
    CHECK(!ok && t == nuxut::Tokens());
    t = nuxut::Tokens::parse("version=2\nradius_small=20\n", &ok);
    CHECK(!ok && t.radiusSmall == 8);

    // Sınırlar ve bozuk değerler: çökme yok, değer sınırlanır ya da atlanır.
    t = nuxut::Tokens::parse("version=1\nradius_small=9999\nfocus_ring=-5\nspacing=x\nfocus_color=#zzzzzz\n=\n==\nradius_medium\n", &ok);
    CHECK(ok && t.radiusSmall == 32 && t.focusRing == 1 && t.spacing == 4 && !t.focusColor.isValid());

    // Boyut sınırı: büyük dosya bütünüyle reddedilir.
    QByteArray big("version=1\nradius_small=3\n");
    big.append(QByteArray(nuxut::Tokens::MaxBytes, '#'));
    t = nuxut::Tokens::parse(big, &ok);
    CHECK(!ok && t.radiusSmall == 8);
}

static bool hasColor(const QImage &img, QRgb c)
{
    for (int y = 0; y < img.height(); ++y)
        for (int x = 0; x < img.width(); ++x)
            if ((img.pixel(x, y) & 0xffffff) == (c & 0xffffff))
                return true;
    return false;
}

static void focus(QStyle *style)
{
    const QRgb ring = qRgb(255, 0, 255);
    QWidget win;
    auto *lay = new QVBoxLayout(&win);
    auto *button = new QPushButton(QStringLiteral("Tamam"), &win);
    auto *edit = new QLineEdit(&win);
    auto *other = new QPushButton(QStringLiteral("Öteki"), &win);
    lay->addWidget(button);
    lay->addWidget(edit);
    lay->addWidget(other);
    for (QWidget *w : {static_cast<QWidget *>(&win), static_cast<QWidget *>(button), static_cast<QWidget *>(edit), static_cast<QWidget *>(other)})
        w->setStyle(style);
    win.resize(240, 160);
    win.show();
    win.activateWindow();
    QApplication::processEvents();

    other->setFocus(Qt::OtherFocusReason);
    QApplication::processEvents();
    const QImage plainButton = button->grab().toImage();
    const QImage plainEdit = edit->grab().toImage();
    CHECK(!hasColor(plainButton, ring));

    for (QWidget *target : {static_cast<QWidget *>(button), static_cast<QWidget *>(edit)}) {
        const QImage plain = target == button ? plainButton : plainEdit;
        // Fare: Qt pencerenin klavye-odak bayrağını tıklamada siler; halka yok, görüntü aynı.
        win.setAttribute(Qt::WA_KeyboardFocusChange, false);
        target->setFocus(Qt::MouseFocusReason);
        QApplication::processEvents();
        if (!target->hasFocus()) {
            std::fprintf(stderr, "offscreen pencerede odak alınamadı\n");
            ++failures;
            return;
        }
        const QImage mouse = target->grab().toImage();
        CHECK(!hasColor(mouse, ring));
        if (target == button)
            CHECK(mouse == plain);

        // Klavye (Tab): halka çizilir.
        win.setAttribute(Qt::WA_KeyboardFocusChange, true);
        target->update();
        QApplication::processEvents();
        CHECK(hasColor(target->grab().toImage(), ring));
        other->setFocus(Qt::TabFocusReason);
        QApplication::processEvents();
    }
}

static void liveReload(QStyle *style, const QString &conf)
{
    CHECK(style->pixelMetric(QStyle::PM_LayoutHorizontalSpacing) == 8);
    write(conf, "version=1\nspacing=6\nfocus_color=#ff00ff\n");
    QElapsedTimer t;
    t.start();
    while (style->pixelMetric(QStyle::PM_LayoutHorizontalSpacing) != 12 && t.elapsed() < 3000)
        QApplication::processEvents(QEventLoop::AllEvents, 50);
    CHECK(style->pixelMetric(QStyle::PM_LayoutHorizontalSpacing) == 12);
    write(conf, "version=1\nspacing=4\nfocus_color=#ff00ff\n");
    t.restart();
    while (style->pixelMetric(QStyle::PM_LayoutHorizontalSpacing) != 8 && t.elapsed() < 3000)
        QApplication::processEvents(QEventLoop::AllEvents, 50);
    CHECK(style->pixelMetric(QStyle::PM_LayoutHorizontalSpacing) == 8);
}

int main(int argc, char **argv)
{
    // Kullanıcının yapılandırması okunmaz: geçici XDG_CONFIG_HOME.
    QTemporaryDir cfg;
    if (!cfg.isValid())
        return 2;
    qputenv("XDG_CONFIG_HOME", cfg.path().toLocal8Bit());
    const QString conf = cfg.path() + QStringLiteral("/nuxut/qt-style.conf");
    write(conf, "version=1\nfocus_color=#ff00ff\n");

    QApplication app(argc, argv);
    tokens();

    QPluginLoader loader(QString::fromLocal8Bit(qgetenv("NUXUT_STYLE_TEST_PLUGIN")));
    auto *plugin = qobject_cast<QStylePlugin *>(loader.instance());
    CHECK(plugin);
    if (!plugin) {
        std::fprintf(stderr, "eklenti yüklenemedi: %s\n", qPrintable(loader.errorString()));
        return 1;
    }
    CHECK(plugin->create(QStringLiteral("fusion")) == nullptr);
    CHECK(QStyleFactory::keys().contains(QStringLiteral("Nuxut"), Qt::CaseInsensitive));
    QStyle *style = plugin->create(QStringLiteral("Nuxut"));
    CHECK(style && style->objectName() == QStringLiteral("Nuxut"));
    if (!style)
        return 1;
    style->polish(&app);
    focus(style);
    liveReload(style, conf);
    style->unpolish(&app);
    delete style;

    if (failures)
        std::fprintf(stderr, "%d başarısız\n", failures);
    else
        std::printf("nuxut-style-test: tamam\n");
    return failures ? 1 : 0;
}
