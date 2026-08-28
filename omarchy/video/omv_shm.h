/* omv_shm.h — shared-memory video frame contract between sigil-engine
 * (writer, Rust: engine/src/rtc/shm.rs mirrors this) and the OmarchyMatrixVideo
 * QML plugin (reader). Version 1. All fields little-endian.
 *
 * File = one 4096-byte file header page followed by `slot_count` slots.
 * Each slot = 4096-byte slot header page + tightly packed pixel rows.
 * Writer: pick next slot (never the one `latest` points at), set slot.seq odd,
 * write header+pixels, set slot.seq even, then publish file.latest.
 * Reader: load latest (acquire); read slot.seq (must be even); copy; re-read seq;
 * discard the copy if seq changed. */
#pragma once
#include <stdint.h>

#define OMV_MAGIC        0x31564D4Fu   /* "OMV1" */
#define OMV_VERSION      1u
#define OMV_FMT_RGBA8888 1u            /* bytes R,G,B,A, A = 255 */
#define OMV_HDR_SIZE     4096u
#define OMV_SLOT_HDR     4096u

struct omv_file_header {
    uint32_t magic;        /* 0x00 written last */
    uint32_t version;      /* 0x04 */
    uint32_t header_size;  /* 0x08 = 4096 */
    uint32_t slot_count;   /* 0x0C = 3 */
    uint32_t slot_stride;  /* 0x10 bytes between slots, multiple of 4096 */
    uint32_t max_width;    /* 0x14 */
    uint32_t max_height;   /* 0x18 */
    uint32_t format;       /* 0x1C OMV_FMT_RGBA8888 */
    uint32_t generation;   /* 0x20 */
    uint32_t reserved0;    /* 0x24 */
    uint64_t latest;       /* 0x28 atomic: (frame_seq << 8) | slot_index; 0 = none */
    uint8_t  reserved1[16];
};

struct omv_slot_header {
    uint32_t seq;          /* 0x00 seqlock, odd while being written */
    uint32_t width;        /* 0x04 */
    uint32_t height;       /* 0x08 */
    uint32_t stride;       /* 0x0C bytes per row (>= width*4) */
    uint64_t timestamp_us; /* 0x10 CLOCK_MONOTONIC */
    uint64_t frame_number; /* 0x18 */
    uint32_t rotation;     /* 0x20 */
    uint32_t flags;        /* 0x24 bit0 = mirror hint */
    uint8_t  reserved[24];
};
