package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import org.junit.Rule
import org.junit.Test
import kotlin.test.*

class TypedSigilPreviewTest {
    @get:Rule val ui=createComposeRule()

    @Test fun typed_cards_use_canonical_projection_and_animation_is_only_an_indicator() {
        var source by mutableStateOf("wave::Hello;\n\nroll::2d6;\n\ncalc::2+3;")
        ui.setContent { MaterialTheme { CompositionLocalProvider(LocalStructuredPreview provides { ContentDecoder.part(NativeCore.structuredPreview(it)) }) {
            Box(Modifier.width(400.dp)) { TypedSigilPreview(source) }
        } } }
        ui.waitUntil(5000) { ui.onAllNodesWithTag("typed-sigil-preview").fetchSemanticsNodes().isNotEmpty() }
        ui.onNodeWithText("2d6").assertExists()
        ui.onNodeWithContentDescription("Animated text: wave").assertExists()
        ui.onNodeWithText("Hello").assertDoesNotExist()
        ui.runOnIdle { source="Plain text" }
        ui.waitUntil(5000) { ui.onAllNodesWithTag("typed-sigil-preview").fetchSemanticsNodes().isEmpty() }
    }

    // The preview and the timeline card must agree on a recurring list: its own kind, its pins and its reset footer.
    @Test fun a_recurring_checklist_previews_as_one() {
        ui.setContent { MaterialTheme { CompositionLocalProvider(LocalStructuredPreview provides { ContentDecoder.part(NativeCore.structuredPreview(it)) }) {
            Box(Modifier.width(400.dp)) { TypedSigilPreview("checklist::recurr::weekly::Flat chores\n-r- Bins out\n- Hoover;") }
        } } }
        ui.waitUntil(5000) { ui.onAllNodesWithTag("typed-sigil-preview").fetchSemanticsNodes().isNotEmpty() }
        ui.onNodeWithText("Flat chores").assertExists()
        ui.onAllNodesWithContentDescription("Kept through resets").assertCountEquals(1)
        ui.onNodeWithText("Weekly · resets ",substring=true).assertExists()
    }

    @Test fun exact_acknowledgment_binds_only_armed_source_and_cancel_discards_pending_launch() {
        val launch=PreviewLaunch()
        val bounds=Rect(0f,300f,200f,450f)
        launch.source="roll::2d6;";launch.bounds=bounds
        launch.arm(launch.source, 1)
        launch.bind(launch.source, "stale", 1)
        assertNull(launch.origin("stale"))
        launch.bind("other", "wrong", 2)
        assertNull(launch.origin("wrong"))
        launch.bind(launch.source,"accepted", 2)
        assertEquals(bounds,launch.origin("accepted"))
        launch.bind(launch.source,"duplicate", 3)
        assertNull(launch.origin("duplicate"))
        launch.arm(launch.source, 1);launch.cancel();launch.bind(launch.source,"cancelled", 2)
        assertNull(launch.origin("cancelled"))
    }

    @Test fun mixed_randomizers_keep_their_own_preview_origin_after_draft_disposal() {
        val launch=PreviewLaunch()
        val dice=Rect(10f,100f,220f,310f)
        val coin=Rect(40f,330f,190f,480f)
        launch.source="roll::2d6; flip::coin;"
        launch.bounds=coin
        launch.visibleOrigins.putAll(mapOf(0 to dice,1 to coin))
        launch.arm(launch.source,4)
        launch.visibleOrigins.clear()
        launch.bounds=Rect.Zero
        launch.bind(launch.source,"sent",5)
        assertEquals(dice,launch.origin("sent",0))
        assertEquals(coin,launch.origin("sent",1))
        assertNull(launch.origin("sent",2))
        assertTrue(launch.holding(launch.source))
        launch.started("wrong",0)
        assertFalse(launch.lifted(0))
        launch.started("sent",0)
        launch.departed("sent",0)
        assertTrue(launch.holding(launch.source))
        launch.started("sent",1)
        launch.departed("sent",1)
        assertFalse(launch.holding(launch.source))
        assertTrue(launch.lifted(0))
        assertTrue(launch.lifted(1))
    }

    @Test fun staged_randomizer_preview_registers_full_size_objects_and_exact_caption_source() {
        val launch=PreviewLaunch()
        val source="roll::6d6;\n\nFor tonight"
        val part=ContentDecoder.part(NativeCore.structuredPreview("roll::6d6;"))!!
        val platform=object:MaterialPlatform {
            override val available=true
            @Composable override fun Object(kind:Int,sides:Int,face:Int,rotation:FloatArray?,label:String?,modifier:Modifier,progress:Float) {
                assertEquals(1,face)
                Box(modifier.then(Modifier.testTag("preview-die")))
            }
            override suspend fun record(data:FloatArray):FloatArray?=error("Preview must not roll")
            override fun horizontalExtent(sides:Int,face:Int,rotation:FloatArray?,outgoing:Boolean)=1f
        }
        ui.setContent {MaterialTheme {CompositionLocalProvider(LocalPreviewLaunch provides launch,LocalMaterialPlatform provides platform,
            LocalSolidMaterial provides {_,_,_->}) {Box(Modifier.width(400.dp)) {StructuredDraftPreview(part,source)}}}}
        ui.onAllNodesWithTag("preview-die").assertCountEquals(6)
        ui.onAllNodesWithTag("preview-die").fetchSemanticsNodes().forEach {assertTrue(it.boundsInRoot.width>=100f)}
        ui.runOnIdle {
            assertEquals(source,launch.source)
            assertTrue(launch.bounds.width>=310f)
            assertTrue(launch.bounds.height>=200f)
            launch.arm(source,0)
            launch.bind(source,"message",1)
            assertEquals(launch.bounds,launch.origin("message"))
        }
    }

    @Test fun leaving_preview_invalidates_visible_origin_without_cancelling_an_armed_send() {
        val launch=PreviewLaunch()
        var visible by mutableStateOf(true)
        val source="roll::2d6;"
        val platform=object:MaterialPlatform {
            override val available=true
            @Composable override fun Object(kind:Int,sides:Int,face:Int,rotation:FloatArray?,label:String?,modifier:Modifier,progress:Float) {Box(modifier)}
            override suspend fun record(data:FloatArray):FloatArray?=error("Unexpected roll")
            override fun horizontalExtent(sides:Int,face:Int,rotation:FloatArray?,outgoing:Boolean)=1f
        }
        ui.setContent {MaterialTheme {CompositionLocalProvider(LocalPreviewLaunch provides launch,LocalMaterialPlatform provides platform,LocalSolidMaterial provides {_,_,_->},LocalStructuredPreview provides {ContentDecoder.part(NativeCore.structuredPreview(it))}) {
            if(visible)Box(Modifier.width(400.dp)) {TypedSigilPreview(source)}
        }}}
        ui.waitUntil(5000) {launch.bounds.width>0}
        ui.runOnIdle {launch.arm(source,1);visible=false}
        ui.runOnIdle {
            assertEquals(Rect.Zero,launch.bounds)
            assertEquals("",launch.source)
            launch.bind(source,"committed",2)
            assertNotNull(launch.origin("committed"))
            launch.arm(source,2);launch.bind(source,"stale",3)
            assertNull(launch.origin("stale"))
        }
    }

    @Test fun provider_intent_requires_explicit_action_and_never_looks_up_while_typing() {
        var opened:PreviewIntent?=null
        var lookups=0
        val source="translate::es::redact::PRIVATE; Hello;"
        ui.setContent {MaterialTheme {CompositionLocalProvider(LocalStructuredPreview provides {ContentDecoder.part(NativeCore.structuredPreview(it))},LocalServiceAccess provides {lookups++;error("Unexpected lookup")}) {
            Box(Modifier.width(400.dp)) {TypedSigilPreview(source,open={intent,original->assertEquals(source,original);opened=intent})}
        }}}
        ui.waitUntil(5000) {ui.onAllNodesWithText("Set up translation").fetchSemanticsNodes().isNotEmpty()}
        assertEquals(0,lookups)
        ui.onNodeWithText("Set up translation").performClick()
        assertEquals("es",opened?.language)
        assertFalse(opened!!.text.contains("PRIVATE"))
        assertEquals(0,lookups)
    }
    @Test fun typed_structured_send_uses_canonical_card_mode_without_waiting_for_preview_debounce() {
        val draft=androidx.compose.foundation.text.input.TextFieldState("roll::2d6;")
        val sends=mutableListOf<Pair<String,Boolean>>()
        ui.setContent {MaterialTheme {CompositionLocalProvider(LocalStructuredPreview provides {ContentDecoder.part(NativeCore.structuredPreview(it))}) {
            Box(Modifier.width(420.dp).height(700.dp)) {ComposerPanel(draft,NativeCore::analyze,true,false,{_,_->},"peer",VoiceState(),0,null) {text,rich,_->sends+=text to rich}}
        }}}
        ui.onNodeWithContentDescription("Send message").performClick()
        assertEquals(listOf("roll::2d6;" to true),sends)
        ui.runOnIdle {draft.edit {replace(0,length,"wave::Hello;")}}
        ui.onNodeWithContentDescription("Send message").performClick()
        assertEquals("wave::Hello;" to false,sends.last())
    }

}
