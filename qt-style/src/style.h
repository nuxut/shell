// Nuxut Qt stili: Breeze (yoksa Fusion) üzerine QProxyStyle.
//
// Taban stil çizimin çoğunu yapar (KDE uygulamalarıyla tutarlı). Nuxut'a özgü olan:
// - köşe yarıçapı, odak halkası ve aralıklar shell'in token dosyasından (tokens.h); renkler
//   QPalette'ten (plasma-integration → kdeglobals, ya da qt6ct; ikisini de shell yazar),
// - odak: taban stile odak durumu hiç verilmez; yalnızca klavyeyle gelen odakta
//   (State_KeyboardFocusChange) framework'ün ince halkası çizilir, fareyle tıklamada hiçbir şey,
// - token dosyası değişince açık uygulamalar canlı güncellenir (QFileSystemWatcher).
#pragma once

#include <QProxyStyle>

#include "tokens.h"

class QFileSystemWatcher;
class QPainter;

namespace nuxut {

class Style : public QProxyStyle
{
    Q_OBJECT
public:
    Style();
    ~Style() override;

    void polish(QApplication *app) override;
    void unpolish(QApplication *app) override;

    void drawPrimitive(PrimitiveElement pe, const QStyleOption *opt, QPainter *p, const QWidget *w = nullptr) const override;
    void drawControl(ControlElement ce, const QStyleOption *opt, QPainter *p, const QWidget *w = nullptr) const override;
    void drawComplexControl(ComplexControl cc, const QStyleOptionComplex *opt, QPainter *p, const QWidget *w = nullptr) const override;
    int pixelMetric(PixelMetric pm, const QStyleOption *opt = nullptr, const QWidget *w = nullptr) const override;
    QSize sizeFromContents(ContentsType ct, const QStyleOption *opt, const QSize &size, const QWidget *w = nullptr) const override;

    const Tokens &tokens() const { return m_tokens; }
    // Klavye odağı mı (halka çizilir)? Fareyle gelen odakta false.
    static bool keyboardFocus(const QStyleOption *opt);

private:
    void reload();
    void watch();
    void ring(QPainter *p, const QRectF &rect, qreal radius, const QStyleOption *opt) const;
    enum class Panel { Button, Flat, Field };
    // Yuvarlatılmış dolgu (+ alan için ince kenar); halkaya yer bırakan iç dikdörtgende.
    void panel(QPainter *p, const QRect &rect, const QStyleOption *opt, Panel kind) const;
    // Halka + boşluk payı: dolgu bu kadar içeriden çizilir, halka dış kenarda.
    int inset() const { return m_tokens.focusRing + 1; }
    QColor focusColor(const QStyleOption *opt) const;

    Tokens m_tokens;
    QFileSystemWatcher *m_watcher = nullptr;
};

} // namespace nuxut
