#include "style.h"

#include <QAbstractScrollArea>
#include <QApplication>
#include <QDir>
#include <QFileInfo>
#include <QFileSystemWatcher>
#include <QPainter>
#include <QStyleFactory>
#include <QStyleOption>
#include <QWidget>

namespace nuxut {

namespace {

// Taban stil: Breeze (KDE uygulamalarıyla aynı çizim), yoksa Fusion (QtWidgets'in içinde, her
// zaman var). Asla nullptr verilmez: QProxyStyle(nullptr) masaüstü stil anahtarını yükler, o da
// kdeglobals'taki `widgetStyle=Nuxut` olup bu stili sonsuz döngüyle yeniden yaratırdı.
QStyle *createBase()
{
    if (QStyle *s = QStyleFactory::create(QStringLiteral("breeze")))
        return s;
    return QStyleFactory::create(QStringLiteral("fusion"));
}

QColor mix(const QColor &a, const QColor &b, qreal t)
{
    const qreal u = 1.0 - t;
    return QColor::fromRgbF(float(a.redF() * u + b.redF() * t), float(a.greenF() * u + b.greenF() * t),
                            float(a.blueF() * u + b.blueF() * t), float(a.alphaF() * u + b.alphaF() * t));
}

// Taban stile odak durumunu gizler (seçeneği yerinde değiştirip kapsam sonunda geri koyar).
// Seçenekler çağıranın yığınında, tek thread'de: geçici değişiklik güvenli ve her tür seçenek
// için çalışır (kopya, somut türü bilmeyi gerektirirdi).
class FocusStrip
{
public:
    explicit FocusStrip(const QStyleOption *opt)
        : m_opt(const_cast<QStyleOption *>(opt))
    {
        if (m_opt) {
            m_saved = m_opt->state;
            m_opt->state &= ~QStyle::State_HasFocus;
        }
    }
    ~FocusStrip()
    {
        if (m_opt)
            m_opt->state = m_saved;
    }
    FocusStrip(const FocusStrip &) = delete;
    FocusStrip &operator=(const FocusStrip &) = delete;

private:
    QStyleOption *m_opt;
    QStyle::State m_saved;
};

// Material durum katmanları (framework DESIGN): üzerine gelme %8, basılı %12.
constexpr qreal HoverLayer = 0.08;
constexpr qreal PressLayer = 0.12;
// Alan kenarı: metin renginin bu opaklığı (framework `outline` tonuna yakın, ince).
constexpr qreal FieldOutline = 0.22;
// Varsayılan/işaretli düğmede vurgu karışımı (tonal kap).
constexpr qreal TonalMix = 0.30;
// Düğme zemini: palette Button çoğu şemada pencere rengine eşit; içerik renginin bu kadarı
// karıştırılarak dolgu kenarsız da seçilir kalır (framework tonal yüzey).
constexpr qreal ButtonTone = 0.08;

} // namespace

Style::Style()
    : QProxyStyle(createBase())
{
    setObjectName(QStringLiteral("Nuxut"));
    m_tokens = Tokens::load(Tokens::path());
}

Style::~Style() = default;

bool Style::keyboardFocus(const QStyleOption *opt)
{
    return opt && (opt->state & State_HasFocus) && (opt->state & State_KeyboardFocusChange);
}

void Style::polish(QApplication *app)
{
    QProxyStyle::polish(app);
    watch();
}

void Style::unpolish(QApplication *app)
{
    delete m_watcher;
    m_watcher = nullptr;
    QProxyStyle::unpolish(app);
}

void Style::watch()
{
    if (!m_watcher) {
        m_watcher = new QFileSystemWatcher(this);
        connect(m_watcher, &QFileSystemWatcher::fileChanged, this, [this] { reload(); });
        connect(m_watcher, &QFileSystemWatcher::directoryChanged, this, [this] { reload(); });
    }
    // Dosya atomik yeniden adlandırmayla yazılır (eski inode izlenmez olur): dizin de izlenir,
    // her olayda dosya yeniden eklenir.
    const QFileInfo file(Tokens::path());
    const QString dir = file.absolutePath();
    if (QFileInfo::exists(dir) && !m_watcher->directories().contains(dir))
        m_watcher->addPath(dir);
    if (file.exists() && !m_watcher->files().contains(file.absoluteFilePath()))
        m_watcher->addPath(file.absoluteFilePath());
}

void Style::reload()
{
    watch();
    const Tokens t = Tokens::load(Tokens::path());
    if (t == m_tokens)
        return;
    m_tokens = t;
    // Ölçüler (aralık, boyut) de değişmiş olabilir: StyleChange yerleşimi yeniden hesaplatır.
    const auto widgets = QApplication::allWidgets();
    for (QWidget *w : widgets) {
        if (w->style() != this)
            continue;
        QEvent e(QEvent::StyleChange);
        QCoreApplication::sendEvent(w, &e);
        w->update();
    }
}

QColor Style::focusColor(const QStyleOption *opt) const
{
    if (m_tokens.focusColor.isValid())
        return m_tokens.focusColor;
    return opt->palette.color(QPalette::Active, QPalette::Highlight);
}

void Style::ring(QPainter *p, const QRectF &rect, qreal radius, const QStyleOption *opt) const
{
    const qreal w = m_tokens.focusRing;
    const QRectF r = rect.adjusted(w / 2, w / 2, -w / 2, -w / 2);
    if (r.width() <= 0 || r.height() <= 0)
        return;
    p->save();
    p->setRenderHint(QPainter::Antialiasing);
    p->setBrush(Qt::NoBrush);
    p->setPen(QPen(focusColor(opt), w));
    const qreal rr = qMin(radius, qMin(r.width(), r.height()) / 2);
    p->drawRoundedRect(r, rr, rr);
    p->restore();
}

void Style::panel(QPainter *p, const QRect &rect, const QStyleOption *opt, Panel kind) const
{
    const QPalette &pal = opt->palette;
    const bool enabled = opt->state & State_Enabled;
    const bool hover = enabled && (opt->state & State_MouseOver);
    const bool down = enabled && (opt->state & State_Sunken);
    const bool on = opt->state & State_On;
    QColor fill;
    QColor content;
    if (kind == Panel::Field) {
        fill = pal.color(QPalette::Base);
        content = pal.color(QPalette::Text);
    } else {
        content = pal.color(QPalette::ButtonText);
        fill = kind == Panel::Flat ? QColor(Qt::transparent) : mix(pal.color(QPalette::Button), content, ButtonTone);
        if (on)
            fill = mix(pal.color(QPalette::Button), pal.color(QPalette::Highlight), TonalMix);
        if (const auto *b = qstyleoption_cast<const QStyleOptionButton *>(opt);
            b && (b->features & QStyleOptionButton::DefaultButton) && kind == Panel::Button)
            fill = mix(pal.color(QPalette::Button), pal.color(QPalette::Highlight), TonalMix);
        // Durum katmanı içerik renginden (düz düğmede saydam zemine bindirilir).
        QColor layer = content;
        if (down) {
            layer.setAlphaF(float(PressLayer));
            fill = fill.alpha() == 0 ? layer : mix(fill, content, PressLayer);
        } else if (hover) {
            layer.setAlphaF(float(HoverLayer));
            fill = fill.alpha() == 0 ? layer : mix(fill, content, HoverLayer);
        }
    }
    if (fill.alpha() == 0)
        return;
    const QRectF r = QRectF(rect).adjusted(inset(), inset(), -inset(), -inset());
    if (r.width() <= 0 || r.height() <= 0)
        return;
    const qreal radius = qMin<qreal>(m_tokens.radiusSmall, qMin(r.width(), r.height()) / 2);
    p->save();
    p->setRenderHint(QPainter::Antialiasing);
    p->setBrush(fill);
    if (kind == Panel::Field) {
        QColor edge = content;
        edge.setAlphaF(float(FieldOutline));
        p->setPen(QPen(edge, 1));
        p->drawRoundedRect(r.adjusted(0.5, 0.5, -0.5, -0.5), radius, radius);
    } else {
        p->setPen(Qt::NoPen);
        p->drawRoundedRect(r, radius, radius);
    }
    p->restore();
}

void Style::drawPrimitive(PrimitiveElement pe, const QStyleOption *opt, QPainter *p, const QWidget *w) const
{
    switch (pe) {
    case PE_FrameFocusRect:
        // Öğe görünümleri ve taban stilin kendi çağrıları: yalnızca klavye odağında halka.
        if (opt && (opt->state & State_KeyboardFocusChange))
            ring(p, opt->rect, m_tokens.radiusSmall / 2.0, opt);
        return;
    case PE_PanelButtonCommand: {
        const auto *b = qstyleoption_cast<const QStyleOptionButton *>(opt);
        const bool flat = b && (b->features & QStyleOptionButton::Flat);
        panel(p, opt->rect, opt, flat ? Panel::Flat : Panel::Button);
        return;
    }
    case PE_PanelLineEdit:
    case PE_FrameLineEdit: {
        // Çerçevesiz alan (açılır kutu/döner kutu içi) taban stile: yalnızca zemin.
        const auto *f = qstyleoption_cast<const QStyleOptionFrame *>(opt);
        if (pe == PE_PanelLineEdit && (!f || f->lineWidth <= 0))
            break;
        const bool kb = keyboardFocus(opt);
        panel(p, opt->rect, opt, Panel::Field);
        if (kb)
            ring(p, opt->rect, m_tokens.radiusSmall + inset(), opt);
        return;
    }
    default:
        break;
    }
    FocusStrip strip(opt);
    QProxyStyle::drawPrimitive(pe, opt, p, w);
}

void Style::drawControl(ControlElement ce, const QStyleOption *opt, QPainter *p, const QWidget *w) const
{
    const bool kb = keyboardFocus(opt);
    switch (ce) {
    case CE_PushButtonBevel: {
        const auto *b = qstyleoption_cast<const QStyleOptionButton *>(opt);
        const bool flat = b && (b->features & QStyleOptionButton::Flat);
        panel(p, opt->rect, opt, flat ? Panel::Flat : Panel::Button);
        if (b && (b->features & QStyleOptionButton::HasMenu)) {
            const int m = pixelMetric(PM_MenuButtonIndicator, opt, w);
            QStyleOption arrow = *opt;
            arrow.rect = QRect(opt->rect.right() - m - inset(), opt->rect.top(), m, opt->rect.height());
            proxy()->drawPrimitive(PE_IndicatorArrowDown, &arrow, p, w);
        }
        return;
    }
    case CE_PushButton: {
        {
            FocusStrip strip(opt);
            proxy()->drawControl(CE_PushButtonBevel, opt, p, w);
            proxy()->drawControl(CE_PushButtonLabel, opt, p, w);
        }
        if (kb)
            ring(p, opt->rect, m_tokens.radiusSmall + inset(), opt);
        return;
    }
    case CE_CheckBox:
    case CE_RadioButton:
    case CE_TabBarTab:
    case CE_ShapedFrame: {
        {
            FocusStrip strip(opt);
            QProxyStyle::drawControl(ce, opt, p, w);
        }
        // Çerçeve yalnızca kaydırma alanında (metin düzenleyici) odak taşır.
        const bool frame_ok = ce != CE_ShapedFrame || qobject_cast<const QAbstractScrollArea *>(w);
        if (kb && frame_ok)
            ring(p, opt->rect, ce == CE_ShapedFrame ? m_tokens.radiusMedium : m_tokens.radiusSmall, opt);
        return;
    }
    default:
        break;
    }
    FocusStrip strip(opt);
    QProxyStyle::drawControl(ce, opt, p, w);
}

void Style::drawComplexControl(ComplexControl cc, const QStyleOptionComplex *opt, QPainter *p, const QWidget *w) const
{
    const bool kb = keyboardFocus(opt);
    switch (cc) {
    case CC_ComboBox:
        if (const auto *cb = qstyleoption_cast<const QStyleOptionComboBox *>(opt); cb && cb->frame) {
            panel(p, opt->rect, opt, cb->editable ? Panel::Field : Panel::Button);
            QStyleOptionComboBox o(*cb);
            o.frame = false;
            o.subControls &= ~SC_ComboBoxFrame;
            o.state &= ~State_HasFocus;
            QProxyStyle::drawComplexControl(cc, &o, p, w);
            if (kb)
                ring(p, opt->rect, m_tokens.radiusSmall + inset(), opt);
            return;
        }
        break;
    case CC_SpinBox:
        if (const auto *sb = qstyleoption_cast<const QStyleOptionSpinBox *>(opt); sb && sb->frame) {
            panel(p, opt->rect, opt, Panel::Field);
            QStyleOptionSpinBox o(*sb);
            o.frame = false;
            o.subControls &= ~SC_SpinBoxFrame;
            o.state &= ~State_HasFocus;
            QProxyStyle::drawComplexControl(cc, &o, p, w);
            if (kb)
                ring(p, opt->rect, m_tokens.radiusSmall + inset(), opt);
            return;
        }
        break;
    case CC_ToolButton:
    case CC_Slider: {
        {
            FocusStrip strip(opt);
            QProxyStyle::drawComplexControl(cc, opt, p, w);
        }
        if (kb) {
            QRect r = opt->rect;
            if (cc == CC_Slider)
                r = proxy()->subControlRect(cc, opt, SC_SliderHandle, w).adjusted(-inset(), -inset(), inset(), inset()) & opt->rect;
            ring(p, r, cc == CC_Slider ? r.height() / 2.0 : m_tokens.radiusSmall, opt);
        }
        return;
    }
    default:
        break;
    }
    FocusStrip strip(opt);
    QProxyStyle::drawComplexControl(cc, opt, p, w);
}

int Style::pixelMetric(PixelMetric pm, const QStyleOption *opt, const QWidget *w) const
{
    const int s = m_tokens.spacing;
    switch (pm) {
    // Framework ızgarası: öğeler arası 2 birim, kenar 3 birim.
    case PM_LayoutHorizontalSpacing:
    case PM_LayoutVerticalSpacing:
        return 2 * s;
    case PM_LayoutLeftMargin:
    case PM_LayoutTopMargin:
    case PM_LayoutRightMargin:
    case PM_LayoutBottomMargin:
        return 3 * s;
    case PM_FocusFrameHMargin:
    case PM_FocusFrameVMargin:
        return inset();
    default:
        return QProxyStyle::pixelMetric(pm, opt, w);
    }
}

QSize Style::sizeFromContents(ContentsType ct, const QStyleOption *opt, const QSize &size, const QWidget *w) const
{
    QSize out = QProxyStyle::sizeFromContents(ct, opt, size, w);
    switch (ct) {
    case CT_PushButton:
    case CT_ComboBox:
    case CT_LineEdit:
    case CT_SpinBox:
        // Halka dış kenarda, dolgu içeride: taban stilin 1 px'lik çerçevesine ek pay.
        out += QSize(2 * (inset() - 1), 2 * (inset() - 1));
        break;
    default:
        break;
    }
    return out;
}

} // namespace nuxut
