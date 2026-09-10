package org.sigil.compose

import androidx.activity.ComponentActivity
import androidx.compose.runtime.*
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import org.junit.Rule
import org.junit.Test
import org.junit.Assert.assertTrue
import org.sigil.*

class DiagramTest {
    @get:Rule val ui = createAndroidComposeRule<ComponentActivity>()
    @Test fun diagram_types_expand_and_focus_nodes_without_rendering_large_lists_in_the_timeline() {
        var diagram by mutableStateOf(DiagramContent("mindmap",RichText("Synthetic diagram"),416f,264f,
            listOf(DiagramNode(RichText("Root"),"rounded",128f,24f),DiagramNode(RichText("Letters"),"process",24f,168f),DiagramNode(RichText("Calls"),"decision",232f,168f)),
            listOf(DiagramEdge(0,1,RichText("contains"),false,144f),DiagramEdge(0,2,RichText("contains"),false,240f)),emptyList()))
        val original=diagram
        val chat=ChatSummary("self","@sam:example.test","","",true,emptyList())
        ui.runOnUiThread { ui.activity.setSigilContent {
            val message=ChatMessage("diagram","sam","Diagram",true,"9:33","sent",false,emptyList(),emptyList(),null,true,
                timestamp=1000,parts=listOf(MessagePart("card","diagram","Diagram",diagram=diagram)))
            SigilApp(NativeCore::palette,NativeCore::analyze,MessengerState(phase="connected",chats=listOf(chat),selected="self",messages=listOf(message)),{ _,_-> })
        } }
        for(type in listOf("mindmap","org","flow","state","sequence")) {
            ui.runOnIdle { diagram=if(type=="sequence") original.copy(kind=type,width=624f,height=336f,
                nodes=original.nodes.mapIndexed { i,n -> n.copy(x=24f+i*208f,y=24f) },edges=original.edges.mapIndexed { i,e -> e.copy(from=if(i==0)0 else 1,to=if(i==0)1 else 0,dashed=i==1) }) else original.copy(kind=type) }
            ui.onNodeWithText("Open diagram").performClick()
            ui.onNodeWithContentDescription("List nodes").performClick()
            ui.onNode(hasText("Root") and hasAnyAncestor(isDialog())).performClick()
            ui.onNodeWithContentDescription("Clear node focus").assertIsDisplayed()
            ui.onNode(hasContentDescription("${type} diagram viewport") and hasAnyAncestor(isDialog())).assertIsDisplayed()
            if(type=="mindmap") {
                ui.onNodeWithText("Collapse branch").performClick()
                ui.onNode(hasContentDescription("Node 2") and hasAnyAncestor(isDialog())).assertDoesNotExist()
                ui.onNodeWithText("Expand branch").performClick()
            }
            if(type=="org") {
                ui.onNodeWithText("Focus branch").performClick()
                ui.onNodeWithText("Show whole diagram").assertIsDisplayed().performClick()
            }
            ui.onNodeWithContentDescription("Clear node focus").performClick()
            ui.onNodeWithText("Fit diagram").performClick()
            if(type=="flow") {
                val label=ui.onNode(hasContentDescription("Connection 1") and hasAnyAncestor(isDialog())).fetchSemanticsNode().boundsInRoot
                val child=ui.onNode(hasContentDescription("Node 2") and hasAnyAncestor(isDialog())).fetchSemanticsNode().boundsInRoot
                assertTrue("Edge labels must stay above the child node",label.bottom<=child.top)
            }
            ui.onNode(isDialog()).captureToImage().asAndroidBitmap().let { bitmap -> java.io.File(ui.activity.cacheDir,"diagram-$type.png").outputStream().use { bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG,100,it) } }
            ui.onNodeWithContentDescription("Close diagram").performClick()
        }
        ui.runOnIdle { diagram=original.copy(kind="timeline",nodes=emptyList(),edges=emptyList(),entries=listOf(DiagramEntry(RichText("March"),RichText("Started")),DiagramEntry(RichText("September"),RichText("Finished")))) }
        ui.onNodeWithText("Open diagram").performClick()
        ui.onNode(hasText("March") and hasAnyAncestor(isDialog())).assertIsDisplayed()
        ui.onNode(hasText("Finished") and hasAnyAncestor(isDialog())).assertIsDisplayed()
        ui.onNodeWithContentDescription("Close diagram").performClick()
        ui.runOnIdle { diagram=original.copy(kind="org",height=11544f,nodes=(0..79).map { DiagramNode(RichText("Item $it"),"rounded",24f,it*144f+24f) },edges=(0..78).map { DiagramEdge(it,it+1,RichText(""),false,0f) }) }
        ui.onNodeWithText("80 nodes · 79 connections").assertIsDisplayed()
        ui.onNodeWithText("Item 79").assertDoesNotExist()
        ui.onNodeWithText("Open diagram").performClick()
        ui.onNodeWithText("Overview · zoom in to read labels").assertIsDisplayed()
        ui.onNodeWithContentDescription("List nodes").performClick()
        ui.onNode(hasScrollToIndexAction() and hasAnyAncestor(isDialog())).performScrollToIndex(79)
        ui.onNodeWithText("Item 79").assertIsDisplayed().performClick()
        ui.onNodeWithContentDescription("Clear node focus").assertIsDisplayed()
        ui.onNodeWithContentDescription("Close diagram").performClick()
    }
}
