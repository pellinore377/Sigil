package org.sigil

import kotlin.test.*

class TrackTagsTest {
    private fun syncsafe(n: Int) = byteArrayOf((n shr 21 and 0x7f).toByte(), (n shr 14 and 0x7f).toByte(), (n shr 7 and 0x7f).toByte(), (n and 0x7f).toByte())
    private fun be32(n: Int) = byteArrayOf((n shr 24).toByte(), (n shr 16).toByte(), (n shr 8).toByte(), n.toByte())
    private fun frame(id: String, body: ByteArray, v4: Boolean) = id.encodeToByteArray() + (if (v4) syncsafe(body.size) else be32(body.size)) + byteArrayOf(0, 0) + body
    private fun tag(v4: Boolean, vararg frames: ByteArray): ByteArray {
        val body = frames.fold(ByteArray(0)) { a, b -> a + b }
        return "ID3".encodeToByteArray() + byteArrayOf(if (v4) 4 else 3, 0, 0) + syncsafe(body.size) + body + ByteArray(64)
    }
    @Test fun id3_text_picture_and_lyrics_are_read_in_both_versions() {
        for (v4 in listOf(false, true)) {
            val picture = byteArrayOf(0x89.toByte(), 0x50, 0x4e, 0x47, 1, 2, 3)
            val bytes = tag(v4,
                frame("TIT2", byteArrayOf(3) + "Tracked".encodeToByteArray(), v4),
                frame("TPE1", byteArrayOf(1, 0xff.toByte(), 0xfe.toByte()) + "Mixer".flatMap { listOf(it.code.toByte(), 0.toByte()) }.toByteArray(), v4),
                frame("TLEN", byteArrayOf(0) + "6000".encodeToByteArray(), v4),
                frame("APIC", byteArrayOf(0) + "image/png".encodeToByteArray() + byteArrayOf(0, 3) + "cover".encodeToByteArray() + byteArrayOf(0) + picture, v4),
                frame("USLT", byteArrayOf(3) + "eng".encodeToByteArray() + byteArrayOf(0) + "[00:01.50]First line\n[00:03.00]Second".encodeToByteArray(), v4))
            val tags = assertNotNull(readTrackTags(bytes), "v4=$v4")
            assertEquals("Tracked", tags.title); assertEquals("Mixer", tags.artist); assertEquals(6000L, tags.lengthMs)
            assertContentEquals(picture, tags.picture); assertEquals("image/png", tags.pictureType)
            assertEquals(listOf(LyricLine(1500, "First line"), LyricLine(3000, "Second")), tags.lyrics)
            assertTrue(tags.synced)
        }
    }
    @Test fun synced_frames_and_plain_lyrics_keep_their_order() {
        val sylt = byteArrayOf(3) + "eng".encodeToByteArray() + byteArrayOf(2, 1, 0) +
            "Two".encodeToByteArray() + byteArrayOf(0) + be32(2000) + "One".encodeToByteArray() + byteArrayOf(0) + be32(1000)
        val tags = assertNotNull(readTrackTags(tag(true, frame("SYLT", sylt, true))))
        assertEquals(listOf(LyricLine(1000, "One"), LyricLine(2000, "Two")), tags.lyrics)
        assertEquals(listOf(LyricLine(null, "Verse"), LyricLine(null, ""), LyricLine(null, "Chorus")), parseLyrics("Verse\n\nChorus\n\n"))
    }
    @Test fun mp4_and_flac_metadata_are_found_and_unknown_files_are_not() {
        fun atom(name: String, body: ByteArray) = be32(body.size + 8) + ByteArray(4) { name[it].code.toByte() } + body
        fun item(name: String, kind: Int, value: ByteArray) = atom(name, atom("data", be32(kind) + be32(0) + value))
        val ilst = atom("ilst", item("©nam", 1, "Song".encodeToByteArray()) + item("©ART", 1, "Band".encodeToByteArray()) + item("covr", 13, byteArrayOf(0xff.toByte(), 0xd8.toByte())))
        val mp4 = atom("ftyp", "M4A ".encodeToByteArray()) + atom("moov", atom("udta", atom("meta", ByteArray(4) + ilst)))
        val m4a = assertNotNull(readTrackTags(mp4))
        assertEquals("Song", m4a.title); assertEquals("Band", m4a.artist); assertEquals("image/jpeg", m4a.pictureType)
        fun le32(n: Int) = byteArrayOf(n.toByte(), (n shr 8).toByte(), (n shr 16).toByte(), (n shr 24).toByte())
        val comment = le32(0) + le32(2) + le32(10) + "TITLE=Blue".encodeToByteArray() + le32(11) + "ARTIST=Reds".encodeToByteArray()
        val flac = "fLaC".encodeToByteArray() + byteArrayOf(0x84.toByte()) + byteArrayOf(0, 0, comment.size.toByte()) + comment
        val tags = assertNotNull(readTrackTags(flac))
        assertEquals("Blue", tags.title); assertEquals("Reds", tags.artist)
        assertNull(readTrackTags("plain text".encodeToByteArray()))
    }
    @Test fun delimited_text_honours_quotes_and_limits() {
        val cells = parseDelimited("a,\"b,1\",c\n\"x\"\"y\",2\n", ',', 8, 2)
        assertEquals(listOf(listOf("a", "b,1"), listOf("x\"y", "2")), cells)
        assertEquals("48.0 KB", attachmentSize(48 * 1024)); assertEquals("1.5 MB", attachmentSize(1536 * 1024)); assertEquals("0:06", trackTime(6400)); assertEquals("1:02:03", trackTime(3723000))
    }
}
