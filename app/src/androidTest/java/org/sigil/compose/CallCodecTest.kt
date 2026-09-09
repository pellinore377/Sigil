package org.sigil.compose

import org.junit.Assert.*
import org.junit.Test
import kotlin.math.*

class CallCodecTest {
    @Test fun opusRoundTripUsesTheDevicesCodecAndPreservesAudibleSamples() {
        var encoded = 0
        var decoded = 0
        var audible = 0
        OpusDecoder { samples, count -> decoded += count; audible += samples.count { abs(it.toInt()) > 1000 } }.use { decoder ->
            OpusEncoder { timestamp, bytes -> encoded++; decoder.input(timestamp, bytes) }.use { encoder ->
                val samples = ShortArray(960)
                try {
                    repeat(100) { frame ->
                        samples.indices.forEach { i -> samples[i] = (sin((frame * 960 + i) * 2.0 * PI * 440 / 48000) * 12000).toInt().toShort() }
                        encoder.input(samples, samples.size)
                        Thread.sleep(5)
                    }
                    repeat(10) { encoder.drain(); decoder.drain(); Thread.sleep(10) }
                } finally { samples.fill(0) }
            }
        }
        assertTrue(encoded > 80); assertTrue(decoded > 48000); assertTrue(audible > 20000)
    }
}
