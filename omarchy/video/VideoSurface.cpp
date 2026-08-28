#include "VideoSurface.h"
#include "omv_shm.h"

#include <QQuickWindow>
#include <QSGImageNode>
#include <QSGTexture>
#include <cstring>
#include <cerrno>
#include <fcntl.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <unistd.h>
#include <QDebug>
#include <cstdlib>
static const bool kDebug = std::getenv("OMV_DEBUG") != nullptr;

VideoSurface::VideoSurface(QQuickItem *parent) : QQuickItem(parent) {
    setFlag(ItemHasContents);
    m_clock.start();
    m_timer.setInterval(16);
    m_timer.setTimerType(Qt::PreciseTimer);
    connect(&m_timer, &QTimer::timeout, this, &VideoSurface::poll);
}

VideoSurface::~VideoSurface() { closeShm(); }

void VideoSurface::setSource(const QString &s) {
    if (s == m_source) return;
    closeShm();
    m_source = s;
    emit sourceChanged();
    syncTimer();
    update();
}

void VideoSurface::setFillMode(FillMode m) {
    if (m == m_fillMode) return;
    m_fillMode = m;
    emit fillModeChanged();
    update();
}

void VideoSurface::setMirror(bool m) {
    if (m == m_mirror) return;
    m_mirror = m;
    emit mirrorChanged();
    update();
}

void VideoSurface::itemChange(ItemChange change, const ItemChangeData &data) {
    QQuickItem::itemChange(change, data);
    if (change == ItemVisibleHasChanged || change == ItemSceneChange) syncTimer();
}

void VideoSurface::syncTimer() {
    const bool want = window() && isVisible() && !m_source.isEmpty();
    if (kDebug) qWarning() << "VideoSurface::syncTimer want" << want << "window" << (window() != nullptr) << "visible" << isVisible() << "source" << m_source;
    if (want && !m_timer.isActive()) { m_lastStatMs = m_clock.elapsed() - 1000; m_timer.start(); }
    else if (!want && m_timer.isActive()) m_timer.stop();
}

bool VideoSurface::openShm() {
    if (m_map) return true;
    if (m_source.isEmpty()) return false;
    const QByteArray path = m_source.toLocal8Bit();
    int fd = ::open(path.constData(), O_RDONLY | O_CLOEXEC);
    if (fd < 0) { if (kDebug) qWarning() << "VideoSurface: open failed" << m_source << strerror(errno); return false; }
    struct stat st{};
    if (::fstat(fd, &st) != 0 || (size_t)st.st_size < OMV_HDR_SIZE) { ::close(fd); return false; }
    void *map = ::mmap(nullptr, (size_t)st.st_size, PROT_READ, MAP_SHARED, fd, 0);
    if (map == MAP_FAILED) { ::close(fd); return false; }
    auto *h = static_cast<const omv_file_header *>(map);
    if (__atomic_load_n(&h->magic, __ATOMIC_ACQUIRE) != OMV_MAGIC || h->version != OMV_VERSION
        || h->format != OMV_FMT_RGBA8888 || h->slot_count == 0
        || (size_t)h->header_size + (size_t)h->slot_count * h->slot_stride > (size_t)st.st_size) {
        if (kDebug) qWarning() << "VideoSurface: header rejected magic" << Qt::hex << h->magic << "version" << h->version << "format" << h->format << "slots" << h->slot_count << "stride" << h->slot_stride << "size" << st.st_size;
        ::munmap(map, (size_t)st.st_size);
        ::close(fd);
        return false;
    }
    if (kDebug) qWarning() << "VideoSurface: mapped" << m_source << st.st_size;
    m_fd = fd;
    m_map = static_cast<const uint8_t *>(map);
    m_len = (size_t)st.st_size;
    m_ino = st.st_ino;
    m_seen = 0;
    return true;
}

void VideoSurface::closeShm() {
    if (m_map) ::munmap(const_cast<uint8_t *>(m_map), m_len);
    if (m_fd >= 0) ::close(m_fd);
    m_map = nullptr;
    m_len = 0;
    m_fd = -1;
    m_ino = 0;
    m_seen = 0;
    if (m_hasFrame) { m_hasFrame = false; emit hasFrameChanged(); }
}

void VideoSurface::poll() {
    const qint64 now = m_clock.elapsed();
    if (!m_map) {
        if (now - m_lastStatMs < 500) return;   // retry open at 2 Hz
        m_lastStatMs = now;
        if (!openShm()) return;
    } else if (now - m_lastStatMs >= 1000) {
        // The writer replaces the file (new inode) on geometry growth or
        // restart; re-stat at 1 Hz and remap when that happens.
        m_lastStatMs = now;
        struct stat st{};
        const QByteArray path = m_source.toLocal8Bit();
        if (::stat(path.constData(), &st) != 0 || st.st_ino != m_ino) {
            if (kDebug) qWarning() << "VideoSurface: source replaced, remapping" << m_source;
            closeShm();
            if (!openShm()) return;
        }
    }
    auto *h = reinterpret_cast<const omv_file_header *>(m_map);
    const uint64_t latest = __atomic_load_n(&h->latest, __ATOMIC_ACQUIRE);
    if (latest != m_seen) {
        m_seen = latest;
        m_frames++;
        m_lastFrameMs = now;
        if (!m_hasFrame) { m_hasFrame = true; emit hasFrameChanged(); }
        update();
    } else if (m_hasFrame && now - m_lastFrameMs > 1500) {
        m_hasFrame = false;
        emit hasFrameChanged();
    }
    if (!m_pendingSize.isEmpty() && m_pendingSize != m_frameSize) {
        m_frameSize = m_pendingSize;
        emit frameSizeChanged();
    }
    if (now - m_lastFpsMs >= 1000) {
        const qreal fps = m_frames * 1000.0 / qMax<qint64>(1, now - m_lastFpsMs);
        m_frames = 0;
        m_lastFpsMs = now;
        if (!qFuzzyCompare(fps + 1, m_fps + 1)) { m_fps = fps; emit frameRateChanged(); }
    }
    if (now - m_lastStatMs >= 1000) {
        m_lastStatMs = now;
        struct stat st{};
        const QByteArray path = m_source.toLocal8Bit();
        if (::stat(path.constData(), &st) != 0 || st.st_ino != m_ino) {
            closeShm();   // writer recreated or removed the file; reopen on next tick
        }
    }
}

QImage VideoSurface::copyLatestFrame(QSize *sizeOut) {
    if (!m_map) return {};
    auto *h = reinterpret_cast<const omv_file_header *>(m_map);
    const uint64_t latest = __atomic_load_n(&h->latest, __ATOMIC_ACQUIRE);
    if (latest == 0) return {};
    const uint32_t si = (uint32_t)(latest & 0xFF);
    if (si >= h->slot_count) return {};
    auto *slot = reinterpret_cast<const omv_slot_header *>(m_map + h->header_size + (size_t)si * h->slot_stride);
    const uint32_t s1 = __atomic_load_n(&slot->seq, __ATOMIC_ACQUIRE);
    if (s1 & 1u) return {};
    const uint32_t w = slot->width, ht = slot->height, st = slot->stride;
    if (!w || !ht || st < w * 4 || (size_t)st * ht > (size_t)h->slot_stride - OMV_SLOT_HDR) return {};
    QImage img((int)w, (int)ht, QImage::Format_RGBA8888);
    const uint8_t *src = reinterpret_cast<const uint8_t *>(slot) + OMV_SLOT_HDR;
    for (uint32_t y = 0; y < ht; ++y) std::memcpy(img.scanLine((int)y), src + (size_t)y * st, (size_t)w * 4);
    __atomic_thread_fence(__ATOMIC_ACQUIRE);
    if (__atomic_load_n(&slot->seq, __ATOMIC_RELAXED) != s1) return {};
    if (sizeOut) *sizeOut = QSize((int)w, (int)ht);
    return img;
}

QRectF VideoSurface::fittedRect(const QSize &tex) const {
    const qreal W = width(), H = height();
    if (tex.isEmpty() || W <= 0 || H <= 0) return QRectF(0, 0, W, H);
    if (m_fillMode == Stretch) return QRectF(0, 0, W, H);
    const qreal sx = W / tex.width(), sy = H / tex.height();
    const qreal s = (m_fillMode == PreserveAspectFit) ? qMin(sx, sy) : qMax(sx, sy);
    const qreal w = tex.width() * s, h = tex.height() * s;
    return QRectF((W - w) / 2, (H - h) / 2, w, h);
}

QSGNode *VideoSurface::updatePaintNode(QSGNode *old, UpdatePaintNodeData *) {
    auto *node = static_cast<QSGImageNode *>(old);
    QSize sz;
    QImage img = copyLatestFrame(&sz);
    if (!img.isNull()) {
        if (!node) {
            node = window()->createImageNode();
            node->setOwnsTexture(true);
            node->setFiltering(QSGTexture::Linear);
        }
        node->setTexture(window()->createTextureFromImage(img, QQuickWindow::TextureIsOpaque));
        m_pendingSize = sz;
    }
    if (!node || !node->texture()) return node;
    node->setRect(fittedRect(node->texture()->textureSize()));
    node->setTextureCoordinatesTransform(m_mirror ? QSGImageNode::MirrorHorizontally : QSGImageNode::NoTransform);
    return node;
}
