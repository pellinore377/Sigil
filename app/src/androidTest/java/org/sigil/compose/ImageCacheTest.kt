package org.sigil.compose

import android.graphics.Bitmap
import android.os.SystemClock
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test
import java.io.ByteArrayOutputStream

class ImageCacheTest {
    private fun pixel()=Bitmap.createBitmap(8,8,Bitmap.Config.ARGB_8888)
    @Test fun repeatedHistoryLoadsStillAuthorizeButDecodeOnce()=runBlocking {
        val cache=ImageCache()
        var authorizations=0;var decodes=0;var permitted=true
        suspend fun load()=authorizedThumbnail(cache,{authorizations++;check(permitted);PreparedMedia(true,"file-a")},{decodes++;pixel()})
        val first=load();repeat(12){assertSame(first,load())}
        assertEquals(1,decodes);assertEquals(14,authorizations)
        permitted=false
        try {load();fail("Cached image bypassed authorization")}catch(_:IllegalStateException){}
        assertEquals(1,decodes)
    }
    @Test fun changedFilesAndEvictionNeverReuseAnUnrelatedThumbnail()=runBlocking {
        val cache=ImageCache(512)
        val a=cache.load("a"){pixel()};val b=cache.load("b"){pixel()};assertSame(a,cache.load("a"){error("hit")})
        cache.load("c"){pixel()};assertTrue(cache.retainedBytes()<=512)
        var redecoded=false;assertNotSame(b,cache.load("b"){redecoded=true;pixel()});assertTrue(redecoded)
        var revision="before"
        try {authorizedThumbnail(cache,{PreparedMedia(true,revision)},{revision="after";pixel()});fail("Changed file admitted")}catch(_:IllegalStateException){}
        var decoded=false;cache.load("before"){decoded=true;pixel()};assertTrue(decoded)
        val tiny=ImageCache();val first=tiny.load("first"){pixel()}
        repeat(64){tiny.load("tiny-$it"){pixel()}}
        assertNotSame(first,tiny.load("first"){pixel()})
    }
    @Test fun backgroundAndAccountDisposalCannotRepopulateAnInFlightCache()=runBlocking {
        val cache=ImageCache();val started=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>()
        val work=async {cache.load("pending"){started.complete(Unit);release.await();pixel()}}
        started.await();cache.setActive(false);release.complete(Unit);work.await();assertEquals(0,cache.retainedBytes())
        cache.load("background"){pixel()};assertEquals(0,cache.retainedBytes())
        cache.setActive(true);cache.load("new-account"){pixel()};assertTrue(cache.retainedBytes()>0)
        cache.clear();assertEquals(0,cache.retainedBytes())
    }
    @Test fun syntheticThumbnailDecodeAvoidsRepeatedPixelAllocation()=runBlocking {
        val original=Bitmap.createBitmap(1600,1200,Bitmap.Config.ARGB_8888)
        val pixels=IntArray(1600*1200){i->0xff000000.toInt() or ((i*1103515245+12345) and 0xffffff)}
        original.setPixels(pixels,0,1600,0,0,1600,1200);pixels.fill(0)
        val stream=ByteArrayOutputStream();original.compress(Bitmap.CompressFormat.JPEG,90,stream);original.recycle()
        val encoded=stream.toByteArray()
        try {
            val coldStart=SystemClock.elapsedRealtimeNanos()
            repeat(8){decodeThumbnail(encoded).recycle()}
            val cold=SystemClock.elapsedRealtimeNanos()-coldStart
            val cache=ImageCache();var count=0
            val first=cache.load("synthetic"){count++;decodeThumbnail(encoded)}
            assertEquals(1080,first.width);assertEquals(810,first.height)
            val hitStart=SystemClock.elapsedRealtimeNanos()
            repeat(8){assertSame(first,cache.load("synthetic"){count++;decodeThumbnail(encoded)})}
            val hits=SystemClock.elapsedRealtimeNanos()-hitStart
            assertEquals(1,count)
            androidx.test.platform.app.InstrumentationRegistry.getInstrumentation().sendStatus(0,android.os.Bundle().apply {
                putString("stream","Synthetic thumbnail: eight decodes ${cold/1000000.0}ms; eight cache hits ${hits/1000000.0}ms; retained ${cache.retainedBytes()} bytes\n")
            })
            cache.clear()
        }finally {encoded.fill(0)}
    }
}
