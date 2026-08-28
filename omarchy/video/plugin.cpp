#include <QQmlExtensionPlugin>
#include <qqml.h>
#include "VideoSurface.h"

class OmarchyMatrixVideoPlugin : public QQmlExtensionPlugin {
    Q_OBJECT
    Q_PLUGIN_METADATA(IID QQmlExtensionInterface_iid)
public:
    void registerTypes(const char *uri) override {
        qmlRegisterType<VideoSurface>(uri, 1, 0, "VideoSurface");
    }
};

#include "plugin.moc"
