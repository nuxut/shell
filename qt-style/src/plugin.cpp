// Qt stil eklentisi girişi: `QStyleFactory::create("Nuxut")` (kdeglobals `[KDE] widgetStyle`,
// qt6ct `style=`, ya da QT_STYLE_OVERRIDE).
#include <QStylePlugin>

#include "style.h"

namespace nuxut {

class Plugin : public QStylePlugin
{
    Q_OBJECT
    Q_PLUGIN_METADATA(IID QStyleFactoryInterface_iid FILE "nuxut.json")
public:
    QStyle *create(const QString &key) override
    {
        return key.compare(QStringLiteral("nuxut"), Qt::CaseInsensitive) == 0 ? new Style : nullptr;
    }
};

} // namespace nuxut

#include "plugin.moc"
