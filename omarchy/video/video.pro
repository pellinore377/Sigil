TEMPLATE = lib
CONFIG += plugin c++20 hide_symbols
CONFIG -= debug_and_release
TARGET = omarchymatrixvideo
QT += qml quick
HEADERS += VideoSurface.h omv_shm.h
SOURCES += VideoSurface.cpp plugin.cpp
QMAKE_CXXFLAGS += -O2
