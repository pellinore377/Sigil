#pragma once
#include <QQuickItem>
#include <QTimer>
#include <QImage>
#include <QSize>
#include <QElapsedTimer>
#include <sys/types.h>

class VideoSurface : public QQuickItem {
    Q_OBJECT
    Q_PROPERTY(QString source READ source WRITE setSource NOTIFY sourceChanged)
    Q_PROPERTY(FillMode fillMode READ fillMode WRITE setFillMode NOTIFY fillModeChanged)
    Q_PROPERTY(bool mirror READ mirror WRITE setMirror NOTIFY mirrorChanged)
    Q_PROPERTY(bool hasFrame READ hasFrame NOTIFY hasFrameChanged)
    Q_PROPERTY(qreal frameRate READ frameRate NOTIFY frameRateChanged)
    Q_PROPERTY(QSize frameSize READ frameSize NOTIFY frameSizeChanged)
public:
    enum FillMode { Stretch, PreserveAspectFit, PreserveAspectCrop };
    Q_ENUM(FillMode)

    explicit VideoSurface(QQuickItem *parent = nullptr);
    ~VideoSurface() override;

    QString source() const { return m_source; }
    void setSource(const QString &s);
    FillMode fillMode() const { return m_fillMode; }
    void setFillMode(FillMode m);
    bool mirror() const { return m_mirror; }
    void setMirror(bool m);
    bool hasFrame() const { return m_hasFrame; }
    qreal frameRate() const { return m_fps; }
    QSize frameSize() const { return m_frameSize; }

signals:
    void sourceChanged();
    void fillModeChanged();
    void mirrorChanged();
    void hasFrameChanged();
    void frameRateChanged();
    void frameSizeChanged();

protected:
    QSGNode *updatePaintNode(QSGNode *old, UpdatePaintNodeData *) override;
    void itemChange(ItemChange change, const ItemChangeData &data) override;

private:
    void poll();
    void syncTimer();
    bool openShm();
    void closeShm();
    QImage copyLatestFrame(QSize *sizeOut);
    QRectF fittedRect(const QSize &tex) const;

    QString m_source;
    FillMode m_fillMode = PreserveAspectFit;
    bool m_mirror = false;
    bool m_hasFrame = false;
    qreal m_fps = 0;
    QSize m_frameSize;

    QTimer m_timer;
    int m_fd = -1;
    const uint8_t *m_map = nullptr;
    size_t m_len = 0;
    ino_t m_ino = 0;
    uint64_t m_seen = 0;
    int m_frames = 0;
    qint64 m_lastFrameMs = 0;
    qint64 m_lastStatMs = 0;
    qint64 m_lastFpsMs = 0;
    QElapsedTimer m_clock;
    QSize m_pendingSize;   // size observed on render thread, published on GUI thread
};
