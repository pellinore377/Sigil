package org.sigil.compose

import android.view.TextureView
import android.view.View
import android.view.ViewGroup
import androidx.activity.ComponentActivity
import androidx.compose.runtime.*
import androidx.compose.foundation.layout.*
import androidx.compose.material3.Text
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.sigil.*

class ObjectAppearanceTest {
    @get:Rule val ui=createAndroidComposeRule<ComponentActivity>()
    private fun textures(v:View):List<TextureView> = when(v){is TextureView->listOf(v);is ViewGroup->(0 until v.childCount).flatMap {textures(v.getChildAt(it))};else->emptyList()}
    private fun ready(count:Int) {ui.waitUntil(15_000) {var ready=false;ui.runOnUiThread {val v=textures(ui.activity.window.decorView);ready=v.size==count && v.all {it.alpha==1f}};ready}}
    private fun pixels():List<Int> {var pixels=emptyList<Int>();ui.runOnUiThread {textures(ui.activity.window.decorView).first().bitmap?.let {b->pixels=(1..9).flatMap {y->(1..9).map {x->b.getPixel(x*b.width/10,y*b.height/10)}};b.recycle()}};return pixels}
    @Test fun every_reference_shape_and_percentile_marks_render_in_both_fonts() {
        var font by mutableStateOf("Newsreader")
        val sides=listOf(4,6,8,10,12,16,20,24,30)
        ui.runOnUiThread {ui.activity.setSigilContent {
            CompositionLocalProvider(LocalAppearance provides Appearance(font=font)) {
                Column(Modifier.fillMaxSize().padding(top=40.dp)) {
                    sides.chunked(3).forEach {row->Row {row.forEach {n->Column(Modifier.weight(1f)) {Text("d$n");MaterialObject(0,n,n-1,null,null,Modifier.fillMaxWidth().height(128.dp))}}}}
                    Row {listOf("tens","units").forEach {mark->Column(Modifier.weight(1f)) {Text(mark);MaterialObject(0,10,1,null,mark,Modifier.fillMaxWidth().height(128.dp))}}}
                }
            }
        }}
        for(name in listOf("Newsreader","Google Sans Flex")) {
            ui.runOnUiThread {font=name};ui.waitForIdle();ready(11);android.os.SystemClock.sleep(300)
            ui.runOnUiThread {textures(ui.activity.window.decorView).forEach {v->val b=v.bitmap!!;val pixels=IntArray(b.width*b.height);b.getPixels(pixels,0,b.width,0,0,b.width,b.height);assertTrue("Each shape must render pixels",pixels.any {android.graphics.Color.alpha(it)>0});b.recycle()}}
            androidx.test.platform.app.InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot()?.let {b->java.io.File(ui.activity.cacheDir,"dice-$name.png").outputStream().use {b.compress(android.graphics.Bitmap.CompressFormat.PNG,100,it)};b.recycle()}
        }
    }
    @Test fun texture_and_border_gallery() {
        ui.runOnUiThread {ui.activity.setSigilContent {
            Column(Modifier.fillMaxSize().padding(top=40.dp)) {
                repeat(4) {n->
                    val a=Appearance()
                    CompositionLocalProvider(LocalAppearance provides a.copy(diceStyle=a.diceStyle.copy(texture=n),coinStyle=a.coinStyle.copy(texture=n),cardStyle=a.cardStyle.copy(texture=n,border=n%3))) {
                        Row {repeat(3) {kind->Column(Modifier.weight(1f)) {Text(listOf("Dice","Coin","Card")[kind]+" "+n);MaterialObject(kind,6,1,null,if(kind==2)"Bookstore" else null,Modifier.fillMaxWidth().height(144.dp))}}}
                    }
                }
            }
        }}
        ready(12);android.os.SystemClock.sleep(400)
        androidx.test.platform.app.InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot()?.let {b->java.io.File(ui.activity.cacheDir,"texture-gallery.png").outputStream().use {b.compress(android.graphics.Bitmap.CompressFormat.PNG,100,it)};b.recycle()}
    }
    @Test fun material_settings_change_the_native_preview_and_survive_reopening() {
        val saved=mutableMapOf<String,String>()
        var session by mutableIntStateOf(0)
        ui.runOnUiThread {ui.activity.setSigilContent {key(session) {SigilApp(NativeCore::palette,NativeCore::analyze,MessengerState(phase="connected"),{_,_->},read=saved::get,write={k,v->saved[k]=v})}}}
        fun open() {
            ui.onNodeWithContentDescription("Settings").performClick()
            ui.onAllNodesWithText("Appearance").onLast().performScrollTo().performClick()
            ui.onNodeWithText("Dice, coins & cards").performScrollTo().performClick()
        }
        open();ready(2)
        val before=pixels()
        ui.onNodeWithText("Personalized").performScrollTo().performClick()
        ui.onNodeWithTag("object-preview").performScrollTo()
        ui.waitForIdle();android.os.SystemClock.sleep(500)
        assertNotEquals("Changing modes must change the rendered material",before,pixels())
        ui.onNodeWithText("Coins").performClick();ready(1)
        ui.onNodeWithText("Cards").performClick();ready(1)
        ui.onNodeWithText("Geometric").performScrollTo().performClick()
        ui.onNodeWithText("Linen").performScrollTo().performClick()
        assertTrue(saved.getValue("account_appearance").contains("|Personalized|"))
        ui.runOnIdle {session++}
        open()
        ui.onNodeWithText("Personalized").performScrollTo().assertIsSelected()
        ui.onNodeWithText("Cards").performScrollTo().performClick()
        ui.onNodeWithText("Geometric").performScrollTo().assertIsSelected()
        ui.onNodeWithText("Linen").performScrollTo().assertIsSelected()
        ui.onNodeWithTag("object-preview").performScrollTo();ready(1)
        androidx.test.platform.app.InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot()?.let {b->java.io.File(ui.activity.cacheDir,"object-appearance.png").outputStream().use {b.compress(android.graphics.Bitmap.CompressFormat.PNG,100,it)};b.recycle()}
    }
}
