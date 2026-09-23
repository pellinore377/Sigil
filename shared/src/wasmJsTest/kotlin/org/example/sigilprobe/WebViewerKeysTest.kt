package org.sigil

import kotlinx.browser.document
import org.w3c.dom.HTMLElement
import org.w3c.dom.events.Event
import org.w3c.dom.events.KeyboardEvent
import org.w3c.dom.events.KeyboardEventInit
import kotlin.test.*

class WebViewerKeysTest {
    private fun seen(target:HTMLElement):Boolean {
        var result:Boolean?=null
        val listener:(Event)->Unit={result=mediaKeepsKeys(it)}
        document.addEventListener("keydown",listener)
        try {target.dispatchEvent(KeyboardEvent("keydown",KeyboardEventInit(key="ArrowLeft",bubbles=true,cancelable=true)))}
        finally {document.removeEventListener("keydown",listener)}
        return assertNotNull(result)
    }

    @Test fun focused_clip_keeps_arrow_keys() {
        val video=document.createElement("video") as HTMLElement
        document.body!!.appendChild(video)
        try {assertTrue(seen(video))} finally {video.remove()}
    }

    @Test fun page_arrow_keys_reach_the_viewer() {
        val box=document.createElement("div") as HTMLElement
        document.body!!.appendChild(box)
        try {assertFalse(seen(box))} finally {box.remove()}
    }
}
